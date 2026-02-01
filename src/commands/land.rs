//! `gg land` - Merge approved PRs/MRs starting from the first commit

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use console::style;
use dialoguer::Confirm;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::provider::{CiStatus, PrState, Provider};
use crate::stack::Stack;

/// Default timeout for waiting (30 minutes)
const DEFAULT_WAIT_TIMEOUT_SECS: u64 = 30 * 60;

/// Polling interval (10 seconds)
const POLL_INTERVAL_SECS: u64 = 10;

/// Run the land command
pub fn run(land_all: bool, squash: bool, wait: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Detect and check provider
    let provider = Provider::detect(&repo)?;
    provider.check_installed()?;
    provider.check_auth()?;

    // Note: We don't require a clean working directory here because land
    // only performs remote operations (merging PRs via the API). It doesn't
    // modify local files.

    // Load stack and refresh PR info
    let mut stack = Stack::load(&repo, &config)?;

    if stack.is_empty() {
        println!("{}", style("Stack is empty. Nothing to land.").dim());
        return Ok(());
    }

    println!(
        "{}",
        style(format!("Checking {} status...", provider.pr_label())).dim()
    );
    stack.refresh_mr_info(&provider)?;

    // Set up Ctrl+C handler if waiting
    let interrupted = if wait {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);
        ctrlc::set_handler(move || {
            flag_clone.store(true, Ordering::SeqCst);
            println!();
            println!("{}", style("Interrupted. Stopping...").yellow());
        })
        .map_err(|e| GgError::Other(format!("Failed to set Ctrl+C handler: {}", e)))?;
        Some(flag)
    } else {
        None
    };

    // Find landable PRs (approved, open, from the start of the stack)
    let mut landed_count = 0;

    for entry in &stack.entries {
        // Check if interrupted
        if let Some(ref flag) = interrupted {
            if flag.load(Ordering::SeqCst) {
                println!("{}", style("Interrupted by user.").yellow());
                break;
            }
        }

        let gg_id = match &entry.gg_id {
            Some(id) => id,
            None => {
                println!(
                    "{} Commit {} is missing GG-ID. Run `gg sync` first.",
                    style("Error:").red().bold(),
                    entry.short_sha
                );
                break;
            }
        };

        let pr_num = match entry.mr_number {
            Some(num) => num,
            None => {
                println!(
                    "{} Commit {} has no {}. Run `gg sync` first.",
                    style("Error:").red().bold(),
                    entry.short_sha,
                    provider.pr_label()
                );
                break;
            }
        };

        // Check PR/MR state
        let pr_info = provider.get_pr_info(pr_num)?;

        match pr_info.state {
            PrState::Merged => {
                println!(
                    "{} {} #{} already merged",
                    style("✓").green(),
                    provider.pr_label(),
                    pr_num
                );
                landed_count += 1;
                continue;
            }
            PrState::Closed => {
                println!(
                    "{} {} #{} is closed. Stopping.",
                    style("✗").red(),
                    provider.pr_label(),
                    pr_num
                );
                break;
            }
            PrState::Draft => {
                println!(
                    "{} {} #{} is a draft. Mark as ready before landing.",
                    style("○").yellow(),
                    provider.pr_label(),
                    pr_num
                );
                break;
            }
            PrState::Open => {
                // If wait flag is set, wait for CI and approvals
                if wait {
                    if let Err(e) =
                        wait_for_pr_ready(&provider, pr_num, land_all, interrupted.as_ref())
                    {
                        println!(
                            "{} {} #{}: {}",
                            style("Error:").red().bold(),
                            provider.pr_label(),
                            pr_num,
                            e
                        );
                        break;
                    }
                } else {
                    // Check if approved (skip if --all is used)
                    if !land_all {
                        let approved = provider.check_pr_approved(pr_num)?;
                        if !approved {
                            println!(
                                "{} {} #{} is not approved. Stopping.",
                                style("○").yellow(),
                                provider.pr_label(),
                                pr_num
                            );
                            break;
                        }
                    }
                }
            }
        }

        // PR/MR is approved and open - land it
        if !land_all && !wait {
            let confirm = Confirm::new()
                .with_prompt(format!(
                    "Merge {} #{} ({})? ",
                    provider.pr_label(),
                    pr_num,
                    entry.title
                ))
                .default(true)
                .interact()
                .unwrap_or(false);

            if !confirm {
                println!("{}", style("Stopping.").dim());
                break;
            }
        }

        println!(
            "{} Merging {} #{}...",
            style("→").cyan(),
            provider.pr_label(),
            pr_num
        );

        match provider.merge_pr(pr_num, squash, false) {
            Ok(()) => {
                println!(
                    "{} Merged {} #{} into {}",
                    style("OK").green().bold(),
                    provider.pr_label(),
                    pr_num,
                    stack.base
                );
                landed_count += 1;

                // Remove PR/MR mapping from config
                config.remove_mr_for_entry(&stack.name, gg_id);

                // Update the base of remaining PRs/MRs to point to the main branch
                // This is critical for stacked PRs - after merging PR #1, PR #2 should
                // point to main instead of PR #1's branch (which no longer exists)
                if land_all {
                    let current_index = stack
                        .entries
                        .iter()
                        .position(|e| e.mr_number == Some(pr_num))
                        .unwrap_or(0);

                    for remaining_entry in stack.entries.iter().skip(current_index + 1) {
                        if let Some(remaining_pr) = remaining_entry.mr_number {
                            println!(
                                "{}",
                                style(format!(
                                    "  Updating {} #{} base to {}...",
                                    provider.pr_label(),
                                    remaining_pr,
                                    stack.base
                                ))
                                .dim()
                            );
                            if let Err(e) = provider.update_pr_base(remaining_pr, &stack.base) {
                                println!(
                                    "{} Warning: Failed to update {} #{} base: {}",
                                    style("⚠").yellow(),
                                    provider.pr_label(),
                                    remaining_pr,
                                    e
                                );
                            }
                        }
                    }
                }
            }
            Err(e) => {
                println!(
                    "{} Failed to merge {} #{}: {}",
                    style("Error:").red().bold(),
                    provider.pr_label(),
                    pr_num,
                    e
                );
                break;
            }
        }

        if !land_all {
            break;
        }

        // Wait a bit for the provider to process
        std::thread::sleep(Duration::from_secs(2));
    }

    // Save updated config
    config.save(git_dir)?;

    if landed_count > 0 {
        println!();
        println!(
            "{} Landed {} {}(s)",
            style("OK").green().bold(),
            landed_count,
            provider.pr_label()
        );

        // Suggest rebasing if there are remaining commits
        if landed_count < stack.len() {
            println!(
                "{}",
                style("  Run `gg rebase` to update remaining commits onto the new base.").dim()
            );
        } else {
            println!(
                "{}",
                style(format!(
                    "  All {}s landed! Run `gg clean` to remove the stack.",
                    provider.pr_label()
                ))
                .dim()
            );
        }
    }

    Ok(())
}

/// Wait for a PR/MR to be ready to merge (CI passes, approvals met)
fn wait_for_pr_ready(
    provider: &Provider,
    pr_num: u64,
    skip_approval: bool,
    interrupted: Option<&Arc<AtomicBool>>,
) -> Result<()> {
    let start_time = Instant::now();
    let timeout = Duration::from_secs(DEFAULT_WAIT_TIMEOUT_SECS);
    let poll_interval = Duration::from_secs(POLL_INTERVAL_SECS);

    println!(
        "{} Waiting for {} #{} to be ready...",
        style("⏳").cyan(),
        provider.pr_label(),
        pr_num
    );
    println!(
        "{}",
        style(format!(
            "  (Checking CI status and approvals every {}s, timeout after {}m)",
            POLL_INTERVAL_SECS,
            DEFAULT_WAIT_TIMEOUT_SECS / 60
        ))
        .dim()
    );

    loop {
        // Check timeout
        if start_time.elapsed() > timeout {
            return Err(GgError::Other(format!(
                "Timeout waiting for {} #{} to be ready",
                provider.pr_label(),
                pr_num
            )));
        }

        // Check if interrupted
        if let Some(flag) = interrupted {
            if flag.load(Ordering::SeqCst) {
                return Err(GgError::Other("Interrupted by user".to_string()));
            }
        }

        // Check CI status
        let ci_status = provider.get_pr_ci_status(pr_num)?;
        let ci_ready = match ci_status {
            CiStatus::Success => true,
            CiStatus::Pending | CiStatus::Running => {
                println!(
                    "  {} CI is {}...",
                    style("⏳").cyan(),
                    match ci_status {
                        CiStatus::Pending => "pending",
                        CiStatus::Running => "running",
                        _ => unreachable!(),
                    }
                );
                false
            }
            CiStatus::Failed => {
                return Err(GgError::Other(format!(
                    "{} #{} CI failed",
                    provider.pr_label(),
                    pr_num
                )));
            }
            CiStatus::Canceled => {
                return Err(GgError::Other(format!(
                    "{} #{} CI was canceled",
                    provider.pr_label(),
                    pr_num
                )));
            }
            CiStatus::Unknown => {
                println!("  {} CI status unknown, proceeding...", style("⚠").yellow());
                true
            }
        };

        // Check approval status (unless --all flag is used)
        let approval_ready = if skip_approval {
            true
        } else {
            let approved = provider.check_pr_approved(pr_num)?;
            if !approved {
                println!("  {} Waiting for approval...", style("⏳").cyan());
            }
            approved
        };

        // If both CI and approval are ready, we're done
        if ci_ready && approval_ready {
            println!(
                "{} {} #{} is ready to merge!",
                style("✓").green(),
                provider.pr_label(),
                pr_num
            );
            return Ok(());
        }

        // Wait before next poll
        std::thread::sleep(poll_interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(DEFAULT_WAIT_TIMEOUT_SECS, 30 * 60);
        assert_eq!(POLL_INTERVAL_SECS, 10);
    }

    #[test]
    fn test_timeout_values_are_reasonable() {
        // Timeout should be at least 5 minutes
        const { assert!(DEFAULT_WAIT_TIMEOUT_SECS >= 5 * 60) };
        // Poll interval should be between 1 and 60 seconds
        const { assert!(POLL_INTERVAL_SECS >= 1 && POLL_INTERVAL_SECS <= 60) };
    }
}
