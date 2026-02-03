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

/// Polling interval (10 seconds)
const POLL_INTERVAL_SECS: u64 = 10;

/// Cleanup after successfully merging a PR/MR:
/// - Remove the PR/MR mapping from config
/// - Update the base of remaining PRs/MRs if landing all
fn cleanup_after_merge(
    config: &mut Config,
    stack: &Stack,
    provider: &Provider,
    gg_id: &str,
    pr_num: u64,
    land_all: bool,
) {
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
                        "  Updating {} {}{} base to {}...",
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        remaining_pr,
                        stack.base
                    ))
                    .dim()
                );
                if let Err(e) = provider.update_pr_base(remaining_pr, &stack.base) {
                    println!(
                        "{} Warning: Failed to update {} {}{} base: {}",
                        style("⚠").yellow(),
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        remaining_pr,
                        e
                    );
                }
            }
        }
    }
}

/// Run the land command
pub fn run(land_all: bool, squash: bool, wait: bool, auto_clean: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Detect and check provider
    let provider = Provider::detect(&repo)?;
    provider.check_installed()?;
    provider.check_auth()?;

    // Check if merge trains are enabled (GitLab only)
    let merge_trains_enabled = provider.check_merge_trains_enabled().unwrap_or(false);
    if merge_trains_enabled {
        println!(
            "{}",
            style("Merge trains enabled - MRs will be added to the merge train").dim()
        );
    }

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
                    "{} {} {}{} already merged",
                    style("✓").green(),
                    provider.pr_label(),
                    provider.pr_number_prefix(),
                    pr_num
                );
                landed_count += 1;
                continue;
            }
            PrState::Closed => {
                println!(
                    "{} {} {}{} is closed. Stopping.",
                    style("✗").red(),
                    provider.pr_label(),
                    provider.pr_number_prefix(),
                    pr_num
                );
                break;
            }
            PrState::Draft => {
                println!(
                    "{} {} {}{} is a draft. Mark as ready before landing.",
                    style("○").yellow(),
                    provider.pr_label(),
                    provider.pr_number_prefix(),
                    pr_num
                );
                break;
            }
            PrState::Open => {
                // If wait flag is set, wait for CI and approvals
                if wait {
                    let timeout_minutes = config.get_land_wait_timeout_minutes();
                    if let Err(e) = wait_for_pr_ready(
                        &provider,
                        pr_num,
                        land_all,
                        timeout_minutes,
                        interrupted.as_ref(),
                        &stack.base,
                    ) {
                        println!(
                            "{} {} {}{}: {}",
                            style("Error:").red().bold(),
                            provider.pr_label(),
                            provider.pr_number_prefix(),
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
                                "{} {} {}{} is not approved. Stopping.",
                                style("○").yellow(),
                                provider.pr_label(),
                                provider.pr_number_prefix(),
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
                    "Merge {} {}{} ({})? ",
                    provider.pr_label(),
                    provider.pr_number_prefix(),
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

        // Use merge trains if enabled (GitLab only)
        if merge_trains_enabled {
            println!(
                "{} Adding {} {}{} to merge train...",
                style("→").cyan(),
                provider.pr_label(),
                provider.pr_number_prefix(),
                pr_num
            );

            match provider.add_to_merge_train(pr_num) {
                Ok(()) => {
                    println!(
                        "{} Added {} {}{} to merge train",
                        style("OK").green().bold(),
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num
                    );
                    landed_count += 1;

                    // Remove PR/MR mapping from config
                    config.remove_mr_for_entry(&stack.name, gg_id);

                    // Update the base of remaining PRs/MRs to point to the main branch
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
                                        "  Updating {} {}{} base to {}...",
                                        provider.pr_label(),
                                        provider.pr_number_prefix(),
                                        remaining_pr,
                                        stack.base
                                    ))
                                    .dim()
                                );
                                if let Err(e) = provider.update_pr_base(remaining_pr, &stack.base) {
                                    println!(
                                        "{} Warning: Failed to update {} {}{} base: {}",
                                        style("⚠").yellow(),
                                        provider.pr_label(),
                                        provider.pr_number_prefix(),
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
                        "{} Failed to add to merge train, attempting direct merge: {}",
                        style("Warning:").yellow(),
                        e
                    );
                    // Fallback to direct merge
                    match provider.merge_pr(pr_num, squash, false) {
                        Ok(()) => {
                            println!(
                                "{} Merged {} {}{} into {}",
                                style("OK").green().bold(),
                                provider.pr_label(),
                                provider.pr_number_prefix(),
                                pr_num,
                                stack.base
                            );
                            landed_count += 1;

                            // Remove PR/MR mapping from config
                            config.remove_mr_for_entry(&stack.name, gg_id);

                            // Update the base of remaining PRs/MRs to point to the main branch
                            if land_all {
                                let current_index = stack
                                    .entries
                                    .iter()
                                    .position(|e| e.mr_number == Some(pr_num))
                                    .unwrap_or(0);

                                for remaining_entry in stack.entries.iter().skip(current_index + 1)
                                {
                                    if let Some(remaining_pr) = remaining_entry.mr_number {
                                        println!(
                                            "{}",
                                            style(format!(
                                                "  Updating {} {}{} base to {}...",
                                                provider.pr_label(),
                                                provider.pr_number_prefix(),
                                                remaining_pr,
                                                stack.base
                                            ))
                                            .dim()
                                        );
                                        if let Err(e) =
                                            provider.update_pr_base(remaining_pr, &stack.base)
                                        {
                                            println!(
                                                "{} Warning: Failed to update {} {}{} base: {}",
                                                style("⚠").yellow(),
                                                provider.pr_label(),
                                                provider.pr_number_prefix(),
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
                                "{} Failed to merge {} {}{}: {}",
                                style("Error:").red().bold(),
                                provider.pr_label(),
                                provider.pr_number_prefix(),
                                pr_num,
                                e
                            );
                            break;
                        }
                    }
                }
            }
        } else {
            println!(
                "{} Merging {} {}{}...",
                style("→").cyan(),
                provider.pr_label(),
                provider.pr_number_prefix(),
                pr_num
            );

            match provider.merge_pr(pr_num, squash, false) {
                Ok(()) => {
                    println!(
                        "{} Merged {} {}{} into {}",
                        style("OK").green().bold(),
                        provider.pr_label(),
                        provider.pr_number_prefix(),
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
                                        "  Updating {} {}{} base to {}...",
                                        provider.pr_label(),
                                        provider.pr_number_prefix(),
                                        remaining_pr,
                                        stack.base
                                    ))
                                    .dim()
                                );
                                if let Err(e) = provider.update_pr_base(remaining_pr, &stack.base) {
                                    println!(
                                        "{} Warning: Failed to update {} {}{} base: {}",
                                        style("⚠").yellow(),
                                        provider.pr_label(),
                                        provider.pr_number_prefix(),
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
                        "{} Failed to merge {} {}{}: {}",
                        style("Error:").red().bold(),
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num,
                        e
                    );
                    break;
                }
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
            // All PRs/MRs landed successfully - offer to clean up
            let should_clean = if auto_clean {
                true
            } else if atty::is(atty::Stream::Stdout) {
                // Interactive prompt (only if stdout is a TTY)
                println!();
                Confirm::new()
                    .with_prompt("All PRs merged successfully. Clean up this stack?")
                    .default(false)
                    .interact()
                    .unwrap_or(false)
            } else {
                // Non-interactive, don't clean
                println!(
                    "{}",
                    style(format!(
                        "  All {}s landed! Run `gg clean` to remove the stack.",
                        provider.pr_label()
                    ))
                    .dim()
                );
                false
            };

            if should_clean {
                println!();
                println!("{}", style("Cleaning up stack...").dim());

                // First, rebase to update main and detect merged commits
                let rebase_result = crate::commands::rebase::run(Some(stack.base.clone()));
                if let Err(e) = rebase_result {
                    println!("{} Failed to rebase: {}", style("Warning:").yellow(), e);
                    println!(
                        "{}",
                        style("  You may need to run `gg rebase` and `gg clean` manually.").dim()
                    );
                    return Ok(());
                }

                // Then, clean the stack
                let clean_result = crate::commands::clean::run_for_stack(&stack.name, true);
                match clean_result {
                    Ok(()) => {
                        println!("{} Stack cleaned successfully", style("OK").green().bold());
                    }
                    Err(e) => {
                        println!(
                            "{} Failed to clean stack: {}",
                            style("Warning:").yellow(),
                            e
                        );
                        println!(
                            "{}",
                            style("  You may need to run `gg clean` manually.").dim()
                        );
                    }
                }
            } else if !auto_clean && atty::is(atty::Stream::Stdout) {
                println!(
                    "{}",
                    style("  Run `gg clean` to remove the stack when ready.").dim()
                );
            }
        }
    }

    Ok(())
}

/// Wait for a PR/MR to be ready to merge (CI passes, approvals met)
/// Also monitors merge train status if merge trains are enabled
fn wait_for_pr_ready(
    provider: &Provider,
    pr_num: u64,
    skip_approval: bool,
    timeout_minutes: u64,
    interrupted: Option<&Arc<AtomicBool>>,
    target_branch: &str,
) -> Result<()> {
    let start_time = Instant::now();
    let timeout = Duration::from_secs(timeout_minutes * 60);
    let poll_interval = Duration::from_secs(POLL_INTERVAL_SECS);

    // Check if merge trains are enabled
    let merge_trains_enabled = provider.check_merge_trains_enabled().unwrap_or(false);

    println!(
        "{} Waiting for {} {}{} to be ready...",
        style("⏳").cyan(),
        provider.pr_label(),
        provider.pr_number_prefix(),
        pr_num
    );
    println!(
        "{}",
        style(format!(
            "  (Checking CI status and approvals every {}s, timeout after {}m)",
            POLL_INTERVAL_SECS, timeout_minutes
        ))
        .dim()
    );

    loop {
        // Check timeout
        if start_time.elapsed() > timeout {
            return Err(GgError::Other(format!(
                "Timeout waiting for {} {}{} to be ready",
                provider.pr_label(),
                provider.pr_number_prefix(),
                pr_num
            )));
        }

        // Check if interrupted
        if let Some(flag) = interrupted {
            if flag.load(Ordering::SeqCst) {
                return Err(GgError::Other("Interrupted by user".to_string()));
            }
        }

        // Check merge train status if enabled
        if merge_trains_enabled {
            if let Ok(Some(train_info)) = provider.get_merge_train_status(pr_num, target_branch) {
                use crate::glab::MergeTrainStatus;
                match train_info.status {
                    MergeTrainStatus::Merged => {
                        println!(
                            "{} {} {}{} has been merged via merge train!",
                            style("✓").green(),
                            provider.pr_label(),
                            provider.pr_number_prefix(),
                            pr_num
                        );
                        return Ok(());
                    }
                    MergeTrainStatus::Merging => {
                        println!("  {} Merge train: merging now...", style("🚂").cyan());
                    }
                    MergeTrainStatus::Fresh => {
                        if let Some(pos) = train_info.position {
                            println!(
                                "  {} Merge train: position {} (fresh, ready to merge)",
                                style("🚂").cyan(),
                                pos
                            );
                        }
                    }
                    MergeTrainStatus::Stale => {
                        println!(
                            "  {} Merge train: MR is stale (needs rebase)",
                            style("⚠").yellow()
                        );
                    }
                    _ => {
                        if let Some(pos) = train_info.position {
                            println!("  {} Merge train: position {}", style("🚂").cyan(), pos);
                        }
                    }
                }

                if train_info.pipeline_running {
                    println!(
                        "  {} Merge train pipeline is running...",
                        style("⏳").cyan()
                    );
                }
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
                    "{} {}{} CI failed",
                    provider.pr_label(),
                    provider.pr_number_prefix(),
                    pr_num
                )));
            }
            CiStatus::Canceled => {
                return Err(GgError::Other(format!(
                    "{} {}{} CI was canceled",
                    provider.pr_label(),
                    provider.pr_number_prefix(),
                    pr_num
                )));
            }
            CiStatus::Unknown => {
                println!(
                    "  {} CI status unknown, waiting for checks to start...",
                    style("⏳").cyan()
                );
                false
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
                "{} {} {}{} is ready to merge!",
                style("✓").green(),
                provider.pr_label(),
                provider.pr_number_prefix(),
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
        assert_eq!(POLL_INTERVAL_SECS, 10);
    }

    #[test]
    fn test_poll_interval_is_reasonable() {
        // Poll interval should be between 1 and 60 seconds
        const { assert!(POLL_INTERVAL_SECS >= 1 && POLL_INTERVAL_SECS <= 60) };
    }
}
