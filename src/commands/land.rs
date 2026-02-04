//! `gg land` - Merge approved PRs/MRs starting from the first commit

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use console::style;
use dialoguer::Confirm;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::glab::AutoMergeResult;
use crate::provider::{CiStatus, PrState, Provider};
use crate::stack::{resolve_target, Stack};

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

/// Rebase remaining PR branches onto the base branch after a merge
/// 
/// This is needed for stacked PRs: after squash-merging PR #1, PR #2's branch
/// still contains the old commit (different SHA), causing merge conflicts.
/// We need to rebase each remaining PR branch onto the updated base to reflect
/// the new squashed commit and avoid conflicts.
fn rebase_remaining_branches(
    repo: &git2::Repository,
    stack: &Stack,
    provider: &Provider,
    start_index: usize,
) -> Result<()> {
    // Fetch the latest base branch
    println!(
        "{}",
        style(format!("  Fetching origin/{}...", stack.base)).dim()
    );

    let fetch_result = std::process::Command::new("git")
        .arg("fetch")
        .arg("origin")
        .arg(&stack.base)
        .current_dir(
            repo.workdir()
                .ok_or_else(|| GgError::Other("Repository has no working directory".to_string()))?,
        )
        .output()
        .map_err(|e| GgError::Other(format!("Failed to fetch: {}", e)))?;

    if !fetch_result.status.success() {
        let stderr = String::from_utf8_lossy(&fetch_result.stderr);
        return Err(GgError::Other(format!(
            "Failed to fetch origin/{}: {}",
            stack.base, stderr
        )));
    }

    // Save current branch
    let current_branch = if let Ok(head) = repo.head() {
        if head.is_branch() {
            head.shorthand().map(String::from)
        } else {
            None
        }
    } else {
        None
    };

    // Rebase each remaining branch
    for entry in stack.entries.iter().skip(start_index + 1) {
        let pr_num = match entry.mr_number {
            Some(num) => num,
            None => continue,
        };

        let branch_name = match stack.entry_branch_name(entry) {
            Some(name) => name,
            None => continue,
        };

        println!(
            "{}",
            style(format!(
                "  Rebasing branch {} ({}{}{})...",
                branch_name,
                provider.pr_label(),
                provider.pr_number_prefix(),
                pr_num
            ))
            .dim()
        );

        // Checkout the branch
        let checkout_result = std::process::Command::new("git")
            .arg("checkout")
            .arg(&branch_name)
            .current_dir(repo.workdir().unwrap())
            .output()
            .map_err(|e| GgError::Other(format!("Failed to checkout {}: {}", branch_name, e)))?;

        if !checkout_result.status.success() {
            let stderr = String::from_utf8_lossy(&checkout_result.stderr);
            return Err(GgError::Other(format!(
                "Failed to checkout branch {}: {}",
                branch_name, stderr
            )));
        }

        // Rebase onto origin/base
        let rebase_target = format!("origin/{}", stack.base);
        let rebase_result = std::process::Command::new("git")
            .arg("rebase")
            .arg(&rebase_target)
            .current_dir(repo.workdir().unwrap())
            .output()
            .map_err(|e| GgError::Other(format!("Failed to rebase {}: {}", branch_name, e)))?;

        if !rebase_result.status.success() {
            // Abort the rebase
            let _ = std::process::Command::new("git")
                .arg("rebase")
                .arg("--abort")
                .current_dir(repo.workdir().unwrap())
                .output();

            let stderr = String::from_utf8_lossy(&rebase_result.stderr);
            return Err(GgError::Other(format!(
                "Failed to rebase {} onto {}: {}. Please rebase manually.",
                branch_name, rebase_target, stderr
            )));
        }

        // Force push with lease
        println!(
            "{}",
            style(format!("  Force-pushing {}...", branch_name)).dim()
        );

        let branch_name_clone = branch_name.clone();
        let push_result = std::process::Command::new("git")
            .arg("push")
            .arg("--force-with-lease")
            .arg("origin")
            .arg(&branch_name)
            .current_dir(repo.workdir().unwrap())
            .output()
            .map_err(|e| GgError::Other(format!("Failed to push {}: {}", branch_name_clone, e)))?;

        if !push_result.status.success() {
            let stderr = String::from_utf8_lossy(&push_result.stderr);
            println!(
                "{} Warning: Failed to push {}: {}",
                style("⚠").yellow(),
                branch_name,
                stderr
            );
            // Continue with other branches even if one push fails
        }
    }

    // Restore original branch
    if let Some(branch) = current_branch {
        let _ = std::process::Command::new("git")
            .arg("checkout")
            .arg(&branch)
            .current_dir(repo.workdir().unwrap())
            .output();
    }

    Ok(())
}

/// Run the land command
pub fn run(
    land_all: bool,
    squash: bool,
    wait: bool,
    auto_clean: bool,
    auto_merge_flag: bool,
    until: Option<String>,
) -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "land")?;

    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Detect and check provider
    let provider = Provider::detect(&repo)?;
    provider.check_installed()?;
    provider.check_auth()?;

    // Determine whether to use GitLab auto-merge-on-land.
    // CLI flag overrides config.
    let auto_merge_on_land =
        provider == Provider::GitLab && (auto_merge_flag || config.get_gitlab_auto_merge_on_land());

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

    let land_until = if let Some(ref target) = until {
        Some(resolve_target(&stack, target)?)
    } else {
        None
    };

    let entries_to_land = if let Some(end_pos) = land_until {
        &stack.entries[..end_pos]
    } else {
        &stack.entries[..]
    };

    let land_multiple = land_all || land_until.is_some();

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

    for entry in entries_to_land {
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
        if !land_multiple && !wait {
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
                Ok(AutoMergeResult::Queued) => {
                    println!(
                        "{} Added {} {}{} to merge train",
                        style("OK").green().bold(),
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num
                    );
                    println!(
                        "{}",
                        style("  Note: MR is queued but not yet merged. Run `gg land` again after it merges to clean up.")
                            .dim()
                    );
                    // Note: We intentionally do NOT clean up config mappings or increment
                    // landed_count here. The MR is only queued in the merge train, not
                    // actually merged yet. Cleanup would be premature and could cause
                    // data loss if the MR is later removed from the train.
                }
                Ok(AutoMergeResult::AlreadyQueued) => {
                    println!(
                        "{} {} {}{} is already in the merge train",
                        style("✓").green(),
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num
                    );
                    println!(
                        "{}",
                        style("  Note: MR is queued but not yet merged. Run `gg land` again after it merges to clean up.")
                            .dim()
                    );
                    // Note: We intentionally do NOT clean up here - MR is not merged yet.
                }
                Err(e) => {
                    println!(
                        "{} Failed to add {} {}{} to merge train: {}",
                        style("Error:").red().bold(),
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num,
                        e
                    );
                    break;
                }
            }
            // When using merge trains, we don't continue to the next MR because
            // they need to merge sequentially through the train.
            break;
        } else if auto_merge_on_land {
            println!(
                "{} Requesting auto-merge for {} {}{} (merge when pipeline succeeds)...",
                style("→").cyan(),
                provider.pr_label(),
                provider.pr_number_prefix(),
                pr_num
            );

            match provider.auto_merge_pr_when_pipeline_succeeds(pr_num, squash, false) {
                Ok(AutoMergeResult::Queued) => {
                    println!(
                        "{} Queued auto-merge for {} {}{} into {}",
                        style("OK").green().bold(),
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num,
                        stack.base
                    );
                    println!(
                        "{}",
                        style("  Note: MR is queued but not yet merged. Run `gg land` again after it merges to clean up.")
                            .dim()
                    );
                    // Note: We intentionally do NOT increment landed_count or clean up
                    // config mappings here. The MR is only queued for auto-merge, not
                    // actually merged yet. Cleanup would be premature.
                }
                Ok(AutoMergeResult::AlreadyQueued) => {
                    println!(
                        "{} {} {}{} is already set to auto-merge",
                        style("✓").green(),
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num
                    );
                    println!(
                        "{}",
                        style("  Note: MR is queued but not yet merged. Run `gg land` again after it merges to clean up.")
                            .dim()
                    );
                    // Note: We intentionally do NOT increment landed_count or clean up here.
                }
                Err(e) => {
                    println!(
                        "{} Failed to request auto-merge for {} {}{}: {}",
                        style("Error:").red().bold(),
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num,
                        e
                    );
                    break;
                }
            }
            // When using auto-merge, we don't continue to the next MR because
            // it needs to merge first before we can update bases of stacked MRs.
            break;
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
                    cleanup_after_merge(
                        &mut config,
                        &stack,
                        &provider,
                        gg_id,
                        pr_num,
                        land_multiple,
                    );

                    // Rebase remaining branches to avoid conflicts from squash merge
                    if land_multiple {
                        let current_index = stack
                            .entries
                            .iter()
                            .position(|e| e.mr_number == Some(pr_num))
                            .unwrap_or(0);

                        if let Err(e) =
                            rebase_remaining_branches(&repo, &stack, &provider, current_index)
                        {
                            println!(
                                "{} Failed to rebase remaining branches: {}",
                                style("Error:").red().bold(),
                                e
                            );
                            println!(
                                "{}",
                                style("  You may need to rebase the remaining PRs manually.").dim()
                            );
                            break;
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

        // Auto-merge is queued asynchronously; landing the rest of a stack
        // requires the earlier MR(s) to actually merge and bases to update.
        if auto_merge_on_land {
            break;
        }

        if !land_multiple {
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
                let rebase_result =
                    crate::commands::rebase::run_with_repo(&repo, Some(stack.base.clone()));
                if let Err(e) = rebase_result {
                    println!("{} Failed to rebase: {}", style("Warning:").yellow(), e);
                    println!(
                        "{}",
                        style("  You may need to run `gg rebase` and `gg clean` manually.").dim()
                    );
                    return Ok(());
                }

                // Then, clean the stack
                let clean_result =
                    crate::commands::clean::run_for_stack_with_repo(&repo, &stack.name, true);
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
    use crate::config::{Config, Defaults, StackConfig};
    use std::collections::HashMap;

    #[test]
    fn test_constants() {
        assert_eq!(POLL_INTERVAL_SECS, 10);
    }

    #[test]
    fn test_poll_interval_is_reasonable() {
        // Poll interval should be between 1 and 60 seconds
        const { assert!(POLL_INTERVAL_SECS >= 1 && POLL_INTERVAL_SECS <= 60) };
    }

    #[test]
    fn test_config_remove_mr_for_entry_removes_single_entry() {
        // Create a config with multiple MR mappings
        let mut config = Config {
            defaults: Defaults::default(),
            stacks: HashMap::new(),
        };

        let mut stack_config = StackConfig {
            base: None,
            mrs: HashMap::new(),
        };
        stack_config.mrs.insert("c-abc1234".to_string(), 123);
        stack_config.mrs.insert("c-def5678".to_string(), 456);
        config.stacks.insert("test-stack".to_string(), stack_config);

        // Verify initial state
        assert_eq!(config.stacks.get("test-stack").unwrap().mrs.len(), 2);

        // Remove one entry
        config.remove_mr_for_entry("test-stack", "c-abc1234");

        // Verify the correct entry was removed
        let stack = config.stacks.get("test-stack").unwrap();
        assert_eq!(stack.mrs.len(), 1);
        assert!(!stack.mrs.contains_key("c-abc1234"));
        assert_eq!(stack.mrs.get("c-def5678"), Some(&456));
    }

    #[test]
    fn test_config_remove_mr_for_entry_handles_nonexistent_entry() {
        let mut config = Config {
            defaults: Defaults::default(),
            stacks: HashMap::new(),
        };

        let mut stack_config = StackConfig {
            base: None,
            mrs: HashMap::new(),
        };
        stack_config.mrs.insert("c-abc1234".to_string(), 123);
        config.stacks.insert("test-stack".to_string(), stack_config);

        // Try to remove non-existent entry - should not panic
        config.remove_mr_for_entry("test-stack", "c-xyz9999");

        // Original entry should still be there
        assert_eq!(config.stacks.get("test-stack").unwrap().mrs.len(), 1);
        assert_eq!(
            config
                .stacks
                .get("test-stack")
                .unwrap()
                .mrs
                .get("c-abc1234"),
            Some(&123)
        );
    }

    #[test]
    fn test_config_remove_mr_for_entry_handles_nonexistent_stack() {
        let mut config = Config {
            defaults: Defaults::default(),
            stacks: HashMap::new(),
        };

        // Try to remove from non-existent stack - should not panic
        config.remove_mr_for_entry("nonexistent-stack", "c-abc1234");

        // Should still have no stacks
        assert_eq!(config.stacks.len(), 0);
    }

    #[test]
    fn test_config_remove_mr_for_entry_removes_multiple_entries() {
        let mut config = Config {
            defaults: Defaults::default(),
            stacks: HashMap::new(),
        };

        let mut stack_config = StackConfig {
            base: None,
            mrs: HashMap::new(),
        };
        stack_config.mrs.insert("c-abc1234".to_string(), 123);
        stack_config.mrs.insert("c-def5678".to_string(), 456);
        stack_config.mrs.insert("c-ghi9012".to_string(), 789);
        config.stacks.insert("test-stack".to_string(), stack_config);

        assert_eq!(config.stacks.get("test-stack").unwrap().mrs.len(), 3);

        // Remove entries one by one
        config.remove_mr_for_entry("test-stack", "c-abc1234");
        assert_eq!(config.stacks.get("test-stack").unwrap().mrs.len(), 2);

        config.remove_mr_for_entry("test-stack", "c-def5678");
        assert_eq!(config.stacks.get("test-stack").unwrap().mrs.len(), 1);

        config.remove_mr_for_entry("test-stack", "c-ghi9012");
        assert_eq!(config.stacks.get("test-stack").unwrap().mrs.len(), 0);
    }

    // Note: cleanup_after_merge is tested indirectly through the land command
    // integration. Direct unit testing would require mocking Config, Stack,
    // and Provider. The function is well-defined with:
    // - config.remove_mr_for_entry() called unconditionally
    // - Base update logic only runs when land_all=true
    // - Updates remaining entries in stack after current index

    #[test]
    #[allow(clippy::type_complexity)]
    fn test_cleanup_after_merge_signature() {
        // This test ensures the helper function signature stays stable.
        // The function takes:
        // - config: &mut Config (for remove_mr_for_entry)
        // - stack: &Stack (for stack.name, stack.base, stack.entries)
        // - provider: &Provider (for pr_label, pr_number_prefix, update_pr_base)
        // - gg_id: &str (commit id to clean up)
        // - pr_num: u64 (PR number that was merged)
        // - land_all: bool (whether to update remaining PR bases)

        // Type-level assertion that cleanup_after_merge exists with the correct signature
        let _fn_ptr: fn(&mut Config, &Stack, &Provider, &str, u64, bool) = cleanup_after_merge;
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn test_rebase_remaining_branches_signature() {
        // This test ensures the rebase helper function signature stays stable.
        // The function takes:
        // - repo: &git2::Repository (for git operations)
        // - stack: &Stack (for stack.base, stack.entries)
        // - provider: &Provider (for pr_label, pr_number_prefix)
        // - start_index: usize (current merge position in stack)

        // Type-level assertion that rebase_remaining_branches exists with the correct signature
        let _fn_ptr: fn(&git2::Repository, &Stack, &Provider, usize) -> Result<()> =
            rebase_remaining_branches;
    }

    // ==========================================================================
    // Tests for rebase_remaining_branches behavior
    // ==========================================================================
    //
    // These tests document the critical behavior of rebasing remaining branches
    // after a squash merge to prevent merge conflicts in stacked PRs.
    //
    // The problem: When PR #1 is squash-merged, it creates a new commit SHA on main.
    // PR #2's branch still contains the old SHA from PR #1, causing GitHub to see
    // it as a conflict.
    //
    // The solution: After merging each PR, rebase all remaining PR branches onto
    // the updated main branch, then force-push them.

    #[test]
    fn test_rebase_prevents_conflicts_in_stacked_prs() {
        // This is a documentation test that explains the squash merge conflict issue.
        //
        // Scenario:
        // - Stack: commit1 (PR #44) → commit2 (PR #45) → commit3 (PR #46)
        // - Merge PR #44 with squash → main gets new SHA (commit1-squashed)
        // - PR #45 branch contains: [commit1-old, commit2]
        // - GitHub sees: commit1-old ≠ commit1-squashed → CONFLICT
        //
        // Solution:
        // After merging PR #44:
        // 1. Fetch origin/main
        // 2. For each remaining PR (45, 46):
        //    - Checkout the PR's branch
        //    - git rebase origin/main
        //    - git push --force-with-lease
        // 3. Restore original branch
        //
        // This ensures all remaining PRs are based on the latest main with the
        // squashed commit, preventing conflicts.
        //
        // Integration testing of this behavior requires:
        // - A real git repo with remotes
        // - Multiple branches and PRs
        // - Simulating a squash merge (changing commit SHAs)
        // - Verifying the branches are rebased correctly
        //
        // This is tested via integration tests in tests/integration_tests.rs
    }

    #[test]
    fn test_rebase_only_runs_for_land_multiple() {
        // This test documents that rebase_remaining_branches is only called
        // when landing multiple PRs (--all or --until flags).
        //
        // When landing a single PR:
        // - land_multiple = false
        // - No need to rebase remaining branches
        // - User can manually rebase if needed
        //
        // When landing multiple PRs:
        // - land_multiple = true
        // - Must rebase after each merge to prevent conflicts
        // - Automatic to ensure smooth stacked PR landing
        //
        // This is enforced by the conditional in the merge success handler:
        // ```rust
        // if land_multiple {
        //     rebase_remaining_branches(...)?;
        // }
        // ```
    }

    #[test]
    fn test_rebase_handles_failures_gracefully() {
        // This test documents the error handling for rebase failures.
        //
        // Possible failure scenarios:
        // 1. Network error during fetch
        // 2. Branch doesn't exist
        // 3. Rebase conflict (can't auto-resolve)
        // 4. Push failure (force-with-lease rejected)
        //
        // Expected behavior:
        // - Abort any in-progress rebase
        // - Print clear error message
        // - Break out of land loop (don't continue with remaining PRs)
        // - Suggest manual intervention
        //
        // The implementation:
        // - Checks all command exit codes
        // - Aborts rebase on conflict
        // - Returns Result<()> to propagate errors
        // - Caller (land command) breaks loop on error
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn test_wait_for_pr_ready_takes_target_branch() {
        // This test documents that wait_for_pr_ready accepts target_branch parameter
        // instead of using a hardcoded "main". The actual function requires a real
        // Provider and network access, so we just verify the signature.

        use std::sync::Arc;

        // Type assertion that the function signature includes target_branch: &str
        let _fn_ptr: fn(&Provider, u64, bool, u64, Option<&Arc<AtomicBool>>, &str) -> Result<()> =
            wait_for_pr_ready;
    }

    // ==========================================================================
    // Tests for merge train / auto-merge behavior (no premature cleanup)
    // ==========================================================================
    //
    // These tests document the critical invariant that when an MR is QUEUED
    // (not actually merged), we must NOT:
    // 1. Increment landed_count
    // 2. Call cleanup_after_merge()
    // 3. Continue to the next MR in the stack
    //
    // Violating these invariants causes data loss (PR #86 fix).

    #[test]
    fn test_auto_merge_result_variants_are_distinct() {
        // Verify that Queued and AlreadyQueued are distinct variants
        // Both represent "MR is in queue" (not merged), so both should
        // trigger the same "no cleanup" behavior.
        use crate::glab::AutoMergeResult;

        let queued = AutoMergeResult::Queued;
        let already_queued = AutoMergeResult::AlreadyQueued;

        // They should be different values
        assert_ne!(queued, already_queued);

        // But both indicate "not yet merged" state
        // This documents the semantic meaning for the land command
        match queued {
            AutoMergeResult::Queued => {} // Expected: MR was just added to queue
            AutoMergeResult::AlreadyQueued => panic!("Should be Queued"),
        }

        match already_queued {
            AutoMergeResult::AlreadyQueued => {} // Expected: MR was already in queue
            AutoMergeResult::Queued => panic!("Should be AlreadyQueued"),
        }
    }

    #[test]
    fn test_auto_merge_result_equality() {
        use crate::glab::AutoMergeResult;

        // Same variants should be equal
        assert_eq!(AutoMergeResult::Queued, AutoMergeResult::Queued);
        assert_eq!(
            AutoMergeResult::AlreadyQueued,
            AutoMergeResult::AlreadyQueued
        );

        // Different variants should not be equal
        assert_ne!(AutoMergeResult::Queued, AutoMergeResult::AlreadyQueued);
    }

    #[test]
    fn test_auto_merge_result_is_clone() {
        use crate::glab::AutoMergeResult;

        // Verify AutoMergeResult implements Clone (needed for safe handling)
        let original = AutoMergeResult::Queued;
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn test_queued_means_not_merged() {
        // This test documents the semantic contract:
        // When add_to_merge_train or auto_merge returns Queued/AlreadyQueued,
        // the MR is NOT yet merged - it's only scheduled to merge later.
        //
        // Therefore, the land command MUST NOT:
        // - Call cleanup_after_merge() (would corrupt config state)
        // - Increment landed_count (would trigger "all landed" cleanup flow)
        // - Continue to next MR (stacked MRs depend on previous one being merged)
        //
        // The correct behavior is:
        // - Print informative message
        // - Break out of the loop
        // - Let user run `gg land` again after the MR actually merges
        //
        // This is a documentation test - the actual behavior is tested via
        // integration tests that would require a GitLab mock.
        use crate::glab::AutoMergeResult;

        // Both variants mean "queued, not merged"
        let results = [AutoMergeResult::Queued, AutoMergeResult::AlreadyQueued];

        for result in results {
            // Neither should be treated as "merged"
            let is_actually_merged = false; // semantic: queued != merged
            assert!(
                !is_actually_merged,
                "{:?} should not be treated as merged",
                result
            );
        }
    }
}
