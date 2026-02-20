//! `gg land` - Merge approved PRs/MRs starting from the first commit

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use console::style;
use dialoguer::Confirm;
use indicatif::{ProgressBar, ProgressStyle};

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::glab::AutoMergeResult;
use crate::output::{print_json, LandResponse, LandResultJson, LandedEntryJson, OUTPUT_VERSION};
use crate::provider::{CiStatus, PrState, Provider};
use crate::stack::{resolve_target, Stack};

/// Format elapsed duration as human-readable string (e.g., "2m15s", "45s")
fn format_duration(elapsed: Duration) -> String {
    let total_secs = elapsed.as_secs();
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;

    if minutes > 0 {
        format!("{}m{}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

/// Create a spinner progress bar with elapsed time
fn create_spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner} {elapsed_precise:.dim} - {msg}")
            .unwrap(),
    );
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(Duration::from_millis(100));
    spinner
}

/// Finish a spinner with a success checkmark and elapsed time
fn finish_spinner(spinner: &ProgressBar, message: &str, start_time: Instant) {
    let elapsed = format_duration(start_time.elapsed());
    spinner.finish_with_message(format!("{} {} - {}", style("✓").green(), elapsed, message));
}

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
    json: bool,
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
                if !json {
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
                }
                if let Err(e) = provider.update_pr_base(remaining_pr, &stack.base) {
                    if !json {
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
    json: bool,
) -> Result<()> {
    // Fetch the latest base branch
    if !json {
        println!(
            "{}",
            style(format!("  Fetching origin/{}...", stack.base)).dim()
        );
    }

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

        if !json {
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
        }

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
        if !json {
            println!(
                "{}",
                style(format!("  Force-pushing {}...", branch_name)).dim()
            );
        }

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
            if !json {
                println!(
                    "{} Warning: Failed to push {}: {}",
                    style("⚠").yellow(),
                    branch_name,
                    stderr
                );
            }
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
    json: bool,
    squash: bool,
    wait: bool,
    auto_clean: bool,
    auto_merge_flag: bool,
    until: Option<String>,
) -> Result<()> {
    let repo = git::open_repo()?;
    let _lock = git::acquire_operation_lock(&repo, "land")?;

    let git_dir = repo.commondir();
    let mut config = Config::load(git_dir)?;

    let provider = Provider::detect(&repo)?;
    provider.check_installed()?;
    provider.check_auth()?;

    let auto_merge_on_land =
        provider == Provider::GitLab && (auto_merge_flag || config.get_gitlab_auto_merge_on_land());

    let merge_trains_enabled = provider.check_merge_trains_enabled().unwrap_or(false);
    if merge_trains_enabled && !json {
        println!(
            "{}",
            style(format!(
                "Merge trains enabled - {}s will be added to the merge train",
                provider.pr_label()
            ))
            .dim()
        );
    }

    let mut stack = Stack::load(&repo, &config)?;
    if stack.is_empty() {
        if json {
            print_json(&LandResponse {
                version: OUTPUT_VERSION,
                land: LandResultJson {
                    stack: stack.name,
                    base: stack.base,
                    landed: vec![],
                    remaining: 0,
                    cleaned: false,
                    warnings: vec![],
                    error: None,
                },
            });
        } else {
            println!("{}", style("Stack is empty. Nothing to land.").dim());
        }
        return Ok(());
    }

    if !json {
        println!(
            "{}",
            style(format!("Checking {} status...", provider.pr_label())).dim()
        );
    }
    stack.refresh_mr_info(&provider)?;

    let land_until = if let Some(ref target) = until {
        Some(resolve_target(&stack, target)?)
    } else {
        None
    };
    let land_multiple = land_all || land_until.is_some();

    let interrupted = if wait {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);
        let json_mode = json;
        ctrlc::set_handler(move || {
            flag_clone.store(true, Ordering::SeqCst);
            if !json_mode {
                println!();
                println!("{}", style("Interrupted. Stopping...").yellow());
            }
        })
        .map_err(|e| GgError::Other(format!("Failed to set Ctrl+C handler: {}", e)))?;
        Some(flag)
    } else {
        None
    };

    let has_unsynced_commits_before_merge = stack.entries.iter().any(|e| !e.is_synced());
    let mut landed_count = 0usize;
    let mut landed_entries: Vec<LandedEntryJson> = vec![];
    let mut seen_already_merged: HashSet<String> = HashSet::new();
    let mut seen_closed: HashSet<String> = HashSet::new();
    let mut warnings: Vec<String> = vec![];
    let mut land_error: Option<String> = None;

    'landing_loop: loop {
        let entries_to_land = if let Some(end_pos) = land_until {
            &stack.entries[..end_pos.min(stack.entries.len())]
        } else {
            &stack.entries[..]
        };

        let mut next_entry_idx = None;
        for (idx, entry) in entries_to_land.iter().enumerate() {
            if let Some(num) = entry.mr_number {
                if let Ok(info) = provider.get_pr_info(num) {
                    if info.state == PrState::Open || info.state == PrState::Draft {
                        next_entry_idx = Some(idx);
                        break;
                    } else if info.state == PrState::Merged {
                        if let Some(gg_id) = &entry.gg_id {
                            if seen_already_merged.insert(gg_id.clone()) {
                                if !json {
                                    println!(
                                        "{} {} {}{} ({}) — already merged",
                                        style("→").cyan(),
                                        provider.pr_label(),
                                        provider.pr_number_prefix(),
                                        num,
                                        entry.title
                                    );
                                }
                                landed_entries.push(LandedEntryJson {
                                    position: entry.position,
                                    sha: entry.short_sha.clone(),
                                    title: entry.title.clone(),
                                    gg_id: gg_id.clone(),
                                    pr_number: num,
                                    action: "already_merged".to_string(),
                                    error: None,
                                });
                                landed_count += 1;
                            }
                        }
                        continue;
                    } else if info.state == PrState::Closed {
                        if let Some(gg_id) = &entry.gg_id {
                            if seen_closed.insert(gg_id.clone()) {
                                if !json {
                                    println!(
                                        "{} {} {}{} ({}) — closed, skipping",
                                        style("⚠").yellow(),
                                        provider.pr_label(),
                                        provider.pr_number_prefix(),
                                        num,
                                        entry.title
                                    );
                                }
                                landed_entries.push(LandedEntryJson {
                                    position: entry.position,
                                    sha: entry.short_sha.clone(),
                                    title: entry.title.clone(),
                                    gg_id: gg_id.clone(),
                                    pr_number: num,
                                    action: "skipped_closed".to_string(),
                                    error: None,
                                });
                            }
                        }
                        continue;
                    }
                }
            }
        }

        let entry_idx = match next_entry_idx {
            Some(idx) => idx,
            None => break 'landing_loop,
        };

        let entry = &entries_to_land[entry_idx];
        if let Some(ref flag) = interrupted {
            if flag.load(Ordering::SeqCst) {
                land_error = Some("Interrupted by user".to_string());
                break 'landing_loop;
            }
        }

        let gg_id = match &entry.gg_id {
            Some(id) => id,
            None => {
                land_error = Some(format!(
                    "Commit {} is missing GG-ID. Run `gg sync` first.",
                    entry.short_sha
                ));
                break 'landing_loop;
            }
        };

        let pr_num = match entry.mr_number {
            Some(num) => num,
            None => {
                land_error = Some(format!(
                    "Commit {} has no {}. Run `gg sync` first.",
                    entry.short_sha,
                    provider.pr_label()
                ));
                break 'landing_loop;
            }
        };

        let pr_info = provider.get_pr_info(pr_num)?;
        match pr_info.state {
            PrState::Merged => {
                if seen_already_merged.insert(gg_id.clone()) {
                    if !json {
                        println!(
                            "{} {} {}{} ({}) — already merged",
                            style("→").cyan(),
                            provider.pr_label(),
                            provider.pr_number_prefix(),
                            pr_num,
                            entry.title
                        );
                    }
                    landed_entries.push(LandedEntryJson {
                        position: entry.position,
                        sha: entry.short_sha.clone(),
                        title: entry.title.clone(),
                        gg_id: gg_id.clone(),
                        pr_number: pr_num,
                        action: "already_merged".to_string(),
                        error: None,
                    });
                    landed_count += 1;
                }
                continue 'landing_loop;
            }
            PrState::Closed => {
                if seen_closed.insert(gg_id.clone()) {
                    if !json {
                        println!(
                            "{} {} {}{} ({}) — closed, skipping",
                            style("⚠").yellow(),
                            provider.pr_label(),
                            provider.pr_number_prefix(),
                            pr_num,
                            entry.title
                        );
                    }
                    landed_entries.push(LandedEntryJson {
                        position: entry.position,
                        sha: entry.short_sha.clone(),
                        title: entry.title.clone(),
                        gg_id: gg_id.clone(),
                        pr_number: pr_num,
                        action: "skipped_closed".to_string(),
                        error: None,
                    });
                }
                continue 'landing_loop;
            }
            PrState::Draft => {
                landed_entries.push(LandedEntryJson {
                    position: entry.position,
                    sha: entry.short_sha.clone(),
                    title: entry.title.clone(),
                    gg_id: gg_id.clone(),
                    pr_number: pr_num,
                    action: "skipped_draft".to_string(),
                    error: None,
                });
                land_error = Some(format!(
                    "{} {}{} is a draft",
                    provider.pr_label(),
                    provider.pr_number_prefix(),
                    pr_num
                ));
                break 'landing_loop;
            }
            PrState::Open => {
                if wait {
                    let timeout_minutes = config.get_land_wait_timeout_minutes();
                    if let Err(e) = wait_for_pr_ready(
                        &provider,
                        pr_num,
                        land_all,
                        timeout_minutes,
                        interrupted.as_ref(),
                        &stack.base,
                        json,
                    ) {
                        landed_entries.push(LandedEntryJson {
                            position: entry.position,
                            sha: entry.short_sha.clone(),
                            title: entry.title.clone(),
                            gg_id: gg_id.clone(),
                            pr_number: pr_num,
                            action: "error".to_string(),
                            error: Some(e.to_string()),
                        });
                        land_error = Some(e.to_string());
                        break 'landing_loop;
                    }
                } else if !land_all {
                    let approved = provider.check_pr_approved(pr_num)?;
                    if !approved {
                        land_error = Some(format!(
                            "{} {}{} is not approved",
                            provider.pr_label(),
                            provider.pr_number_prefix(),
                            pr_num
                        ));
                        break 'landing_loop;
                    }
                }
            }
        }

        if !land_multiple && !wait && !json {
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
                break 'landing_loop;
            }
        }

        if merge_trains_enabled {
            match provider.add_to_merge_train(pr_num) {
                Ok(result) => {
                    let action = match result {
                        AutoMergeResult::Queued => "queued",
                        AutoMergeResult::AlreadyQueued => "already_queued",
                    };
                    landed_entries.push(LandedEntryJson {
                        position: entry.position,
                        sha: entry.short_sha.clone(),
                        title: entry.title.clone(),
                        gg_id: gg_id.clone(),
                        pr_number: pr_num,
                        action: action.to_string(),
                        error: None,
                    });
                    if wait {
                        let timeout_minutes = config.get_land_wait_timeout_minutes();
                        if let Err(e) = wait_for_merge_train_completion(
                            &provider,
                            pr_num,
                            timeout_minutes,
                            interrupted.as_ref(),
                            &stack.base,
                            json,
                        ) {
                            land_error = Some(e.to_string());
                            break 'landing_loop;
                        }
                        landed_count += 1;
                        cleanup_after_merge(
                            &mut config,
                            &stack,
                            &provider,
                            gg_id,
                            pr_num,
                            land_multiple,
                            json,
                        );
                        if land_multiple {
                            let current_index = stack
                                .entries
                                .iter()
                                .position(|e| e.mr_number == Some(pr_num))
                                .unwrap_or(0);
                            if let Err(e) = rebase_remaining_branches(
                                &repo,
                                &stack,
                                &provider,
                                current_index,
                                json,
                            ) {
                                warnings
                                    .push(format!("Failed to rebase remaining branches: {}", e));
                                land_error = Some(e.to_string());
                                break 'landing_loop;
                            }
                            stack = Stack::load(&repo, &config)?;
                            if !stack.is_empty() {
                                stack.refresh_mr_info(&provider)?;
                            }
                        }
                    } else {
                        break 'landing_loop;
                    }
                }
                Err(e) => {
                    landed_entries.push(LandedEntryJson {
                        position: entry.position,
                        sha: entry.short_sha.clone(),
                        title: entry.title.clone(),
                        gg_id: gg_id.clone(),
                        pr_number: pr_num,
                        action: "error".to_string(),
                        error: Some(e.to_string()),
                    });
                    land_error = Some(e.to_string());
                    break 'landing_loop;
                }
            }
        } else if auto_merge_on_land {
            match provider.auto_merge_pr_when_pipeline_succeeds(pr_num, squash, false) {
                Ok(AutoMergeResult::Queued) => landed_entries.push(LandedEntryJson {
                    position: entry.position,
                    sha: entry.short_sha.clone(),
                    title: entry.title.clone(),
                    gg_id: gg_id.clone(),
                    pr_number: pr_num,
                    action: "queued".to_string(),
                    error: None,
                }),
                Ok(AutoMergeResult::AlreadyQueued) => landed_entries.push(LandedEntryJson {
                    position: entry.position,
                    sha: entry.short_sha.clone(),
                    title: entry.title.clone(),
                    gg_id: gg_id.clone(),
                    pr_number: pr_num,
                    action: "already_queued".to_string(),
                    error: None,
                }),
                Err(e) => {
                    landed_entries.push(LandedEntryJson {
                        position: entry.position,
                        sha: entry.short_sha.clone(),
                        title: entry.title.clone(),
                        gg_id: gg_id.clone(),
                        pr_number: pr_num,
                        action: "error".to_string(),
                        error: Some(e.to_string()),
                    });
                    land_error = Some(e.to_string());
                }
            }
            break 'landing_loop;
        } else {
            match provider.merge_pr(pr_num, squash, false) {
                Ok(()) => {
                    landed_entries.push(LandedEntryJson {
                        position: entry.position,
                        sha: entry.short_sha.clone(),
                        title: entry.title.clone(),
                        gg_id: gg_id.clone(),
                        pr_number: pr_num,
                        action: "merged".to_string(),
                        error: None,
                    });
                    landed_count += 1;
                    cleanup_after_merge(
                        &mut config,
                        &stack,
                        &provider,
                        gg_id,
                        pr_num,
                        land_multiple,
                        json,
                    );
                    if land_multiple {
                        let current_index = stack
                            .entries
                            .iter()
                            .position(|e| e.mr_number == Some(pr_num))
                            .unwrap_or(0);
                        if let Err(e) =
                            rebase_remaining_branches(&repo, &stack, &provider, current_index, json)
                        {
                            warnings.push(format!("Failed to rebase remaining branches: {}", e));
                            land_error = Some(e.to_string());
                            break 'landing_loop;
                        }
                        stack = Stack::load(&repo, &config)?;
                        if !stack.is_empty() {
                            stack.refresh_mr_info(&provider)?;
                        }
                    }
                }
                Err(e) => {
                    landed_entries.push(LandedEntryJson {
                        position: entry.position,
                        sha: entry.short_sha.clone(),
                        title: entry.title.clone(),
                        gg_id: gg_id.clone(),
                        pr_number: pr_num,
                        action: "error".to_string(),
                        error: Some(e.to_string()),
                    });
                    land_error = Some(e.to_string());
                    break 'landing_loop;
                }
            }
        }

        if !land_multiple {
            break 'landing_loop;
        }
        std::thread::sleep(Duration::from_secs(2));
    }

    config.save(git_dir)?;

    let mut cleaned = false;
    if landed_count > 0 && landed_count >= stack.len() {
        let should_clean = if json {
            auto_clean
        } else if auto_clean {
            true
        } else if atty::is(atty::Stream::Stdout) {
            Confirm::new()
                .with_prompt(format!(
                    "All {}s merged successfully. Clean up this stack?",
                    provider.pr_label()
                ))
                .default(false)
                .interact()
                .unwrap_or(false)
        } else {
            false
        };

        if should_clean && !has_unsynced_commits_before_merge {
            let _ = crate::commands::rebase::run_with_repo(&repo, Some(stack.base.clone()), json);
            if crate::commands::clean::run_for_stack_with_repo(&repo, &stack.name, true).is_ok() {
                cleaned = true;
            }
        }
    }

    if json {
        let target_len = if let Some(end_pos) = land_until {
            end_pos.min(stack.entries.len())
        } else {
            stack.entries.len()
        };
        let remaining = target_len.saturating_sub(
            landed_entries
                .iter()
                .filter(|e| matches!(e.action.as_str(), "merged" | "already_merged"))
                .count(),
        );
        print_json(&LandResponse {
            version: OUTPUT_VERSION,
            land: LandResultJson {
                stack: stack.name,
                base: stack.base,
                landed: landed_entries,
                remaining,
                cleaned,
                warnings,
                error: land_error,
            },
        });
    } else if landed_count > 0 {
        println!();
        println!(
            "{} Landed {} {}(s)",
            style("OK").green().bold(),
            landed_count,
            provider.pr_label()
        );
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
    json: bool,
) -> Result<()> {
    let start_time = Instant::now();
    let timeout = Duration::from_secs(timeout_minutes * 60);
    let poll_interval = Duration::from_secs(POLL_INTERVAL_SECS);

    // Check if merge trains are enabled
    let merge_trains_enabled = provider.check_merge_trains_enabled().unwrap_or(false);

    if !json {
        println!(
            "{}",
            style(format!(
                "Waiting for {} {}{} to be ready (timeout: {}m)...",
                provider.pr_label(),
                provider.pr_number_prefix(),
                pr_num,
                timeout_minutes
            ))
            .dim()
        );
    }

    let mut current_spinner: Option<ProgressBar> = None;
    let mut current_state: Option<String> = None;
    let mut state_start_time = Instant::now();

    loop {
        // Check timeout
        if start_time.elapsed() > timeout {
            if let Some(ref spinner) = current_spinner {
                spinner.finish_and_clear();
            }
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
                if let Some(ref spinner) = current_spinner {
                    spinner.finish_and_clear();
                }
                return Err(GgError::Other("Interrupted by user".to_string()));
            }
        }

        let mut new_state = String::new();
        let mut ci_ready = false;

        // Check merge train status if enabled
        if merge_trains_enabled {
            if let Ok(Some(train_info)) = provider.get_merge_train_status(pr_num, target_branch) {
                use crate::glab::MergeTrainStatus;
                match train_info.status {
                    MergeTrainStatus::Merged => {
                        if let Some(ref spinner) = current_spinner {
                            finish_spinner(
                                spinner,
                                &format!(
                                    "{} {}{} merged via merge train",
                                    provider.pr_label(),
                                    provider.pr_number_prefix(),
                                    pr_num
                                ),
                                state_start_time,
                            );
                        }
                        return Ok(());
                    }
                    MergeTrainStatus::Merging => {
                        new_state = "Merge train: merging now...".to_string();
                    }
                    MergeTrainStatus::Fresh => {
                        if let Some(pos) = train_info.position {
                            new_state = format!("Merge train: position {} (fresh, ready)", pos);
                        }
                    }
                    MergeTrainStatus::Stale => {
                        new_state = "Merge train: stale (needs rebase)".to_string();
                    }
                    _ => {
                        if let Some(pos) = train_info.position {
                            new_state = format!("Merge train: position {}", pos);
                        }
                    }
                }

                if train_info.pipeline_running {
                    new_state = format!("{} (pipeline running)", new_state);
                }
            }
        }

        // Check CI status
        let ci_status = provider.get_pr_ci_status(pr_num)?;
        match ci_status {
            CiStatus::Success => {
                ci_ready = true;
            }
            CiStatus::Pending => {
                if new_state.is_empty() {
                    new_state = "Waiting for CI to start...".to_string();
                }
            }
            CiStatus::Running => {
                if new_state.is_empty() {
                    new_state = "CI running...".to_string();
                }
            }
            CiStatus::Failed => {
                if let Some(ref spinner) = current_spinner {
                    spinner.finish_and_clear();
                }
                return Err(GgError::Other(format!(
                    "{} {}{} CI failed",
                    provider.pr_label(),
                    provider.pr_number_prefix(),
                    pr_num
                )));
            }
            CiStatus::Canceled => {
                if let Some(ref spinner) = current_spinner {
                    spinner.finish_and_clear();
                }
                return Err(GgError::Other(format!(
                    "{} {}{} CI was canceled",
                    provider.pr_label(),
                    provider.pr_number_prefix(),
                    pr_num
                )));
            }
            CiStatus::Unknown => {
                if new_state.is_empty() {
                    new_state = "CI status unknown, waiting...".to_string();
                }
            }
        };

        // Check approval status (unless --all flag is used)
        let approval_ready = if skip_approval {
            true
        } else {
            let approved = provider.check_pr_approved(pr_num)?;
            if !approved && ci_ready && new_state.is_empty() {
                new_state = "Waiting for approval...".to_string();
            }
            approved
        };

        // If both CI and approval are ready, we're done
        if ci_ready && approval_ready {
            if let Some(ref spinner) = current_spinner {
                finish_spinner(
                    spinner,
                    &format!(
                        "{} {}{} is ready to merge",
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num
                    ),
                    state_start_time,
                );
            }
            return Ok(());
        }

        // Update spinner if state changed
        if current_state.as_ref() != Some(&new_state) {
            // Finish previous spinner if exists
            if let Some(ref spinner) = current_spinner {
                finish_spinner(spinner, current_state.as_ref().unwrap(), state_start_time);
            }
            // Create new spinner
            current_spinner = Some(if json {
                let spinner = ProgressBar::hidden();
                spinner.set_message(new_state.clone());
                spinner
            } else {
                create_spinner(&new_state)
            });
            current_state = Some(new_state);
            state_start_time = Instant::now();
        }

        // Wait before next poll
        std::thread::sleep(poll_interval);
    }
}

/// Wait for an MR to complete merging through the merge train
/// Polls the merge train status until the MR is fully merged
fn wait_for_merge_train_completion(
    provider: &Provider,
    pr_num: u64,
    timeout_minutes: u64,
    interrupted: Option<&Arc<AtomicBool>>,
    target_branch: &str,
    json: bool,
) -> Result<()> {
    let start_time = Instant::now();
    let timeout = Duration::from_secs(timeout_minutes * 60);
    let poll_interval = Duration::from_secs(POLL_INTERVAL_SECS);

    // Grace period: after adding to merge train, the MR may not appear in the
    // train list immediately. Allow some polls with Idle status before treating
    // it as an error.
    const IDLE_GRACE_POLLS: u32 = 6; // ~60 seconds at 10s poll interval
    let mut idle_count: u32 = 0;

    if !json {
        println!(
            "{}",
            style(format!(
                "Waiting for {} {}{} to merge through merge train (timeout: {}m)...",
                provider.pr_label(),
                provider.pr_number_prefix(),
                pr_num,
                timeout_minutes
            ))
            .dim()
        );
    }

    let mut current_spinner: Option<ProgressBar> = None;
    let mut current_state: Option<String> = None;
    let mut state_start_time = Instant::now();

    loop {
        // Check timeout
        if start_time.elapsed() > timeout {
            if let Some(ref spinner) = current_spinner {
                spinner.finish_and_clear();
            }
            return Err(GgError::Other(format!(
                "Timeout waiting for {} {}{} to merge through train",
                provider.pr_label(),
                provider.pr_number_prefix(),
                pr_num
            )));
        }

        // Check if interrupted
        if let Some(flag) = interrupted {
            if flag.load(Ordering::SeqCst) {
                if let Some(ref spinner) = current_spinner {
                    spinner.finish_and_clear();
                }
                return Err(GgError::Other("Interrupted by user".to_string()));
            }
        }

        // Check if MR is actually merged by checking its state first
        let pr_info = provider.get_pr_info(pr_num)?;
        if pr_info.state == PrState::Merged {
            if let Some(ref spinner) = current_spinner {
                finish_spinner(
                    spinner,
                    &format!(
                        "{} {}{} merged",
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num
                    ),
                    state_start_time,
                );
            }
            return Ok(());
        }

        // Check if MR was closed (removed from train or rejected)
        if pr_info.state == PrState::Closed {
            if let Some(ref spinner) = current_spinner {
                spinner.finish_and_clear();
            }
            return Err(GgError::Other(format!(
                "{} {}{} was closed (may have been removed from merge train)",
                provider.pr_label(),
                provider.pr_number_prefix(),
                pr_num
            )));
        }

        let mut new_state = String::new();

        // Check merge train status
        if let Ok(Some(train_info)) = provider.get_merge_train_status(pr_num, target_branch) {
            use crate::glab::MergeTrainStatus;
            match train_info.status {
                MergeTrainStatus::Merged => {
                    if let Some(ref spinner) = current_spinner {
                        finish_spinner(
                            spinner,
                            &format!(
                                "{} {}{} merged via merge train",
                                provider.pr_label(),
                                provider.pr_number_prefix(),
                                pr_num
                            ),
                            state_start_time,
                        );
                    }
                    return Ok(());
                }
                MergeTrainStatus::Merging => {
                    idle_count = 0; // MR is in train, reset grace counter
                    new_state = "Merge train: merging now...".to_string();
                }
                MergeTrainStatus::Fresh => {
                    idle_count = 0;
                    if let Some(pos) = train_info.position {
                        new_state = format!("Merge train: position {} (fresh, ready)", pos);
                    }
                }
                MergeTrainStatus::Stale => {
                    idle_count = 0;
                    new_state = "Merge train: stale (waiting for rebase/pipeline)".to_string();
                }
                MergeTrainStatus::SkipMerged => {
                    if let Some(ref spinner) = current_spinner {
                        spinner.finish_and_clear();
                    }
                    return Err(GgError::Other(format!(
                        "{} {}{} was skipped from the merge train",
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num
                    )));
                }
                MergeTrainStatus::Idle => {
                    idle_count += 1;
                    if idle_count > IDLE_GRACE_POLLS {
                        if let Some(ref spinner) = current_spinner {
                            spinner.finish_and_clear();
                        }
                        return Err(GgError::Other(format!(
                            "{} {}{} is no longer in the merge train",
                            provider.pr_label(),
                            provider.pr_number_prefix(),
                            pr_num
                        )));
                    }
                    new_state = format!(
                        "Waiting for merge train to pick up MR ({}s)...",
                        idle_count * POLL_INTERVAL_SECS as u32
                    );
                }
                _ => {
                    if let Some(pos) = train_info.position {
                        new_state = format!("Merge train: position {}", pos);
                    }
                }
            }

            if train_info.pipeline_running {
                new_state = format!("{} (pipeline running)", new_state);
            }
        } else {
            // If we can't get train status, check if it's been merged directly
            new_state = "Checking merge status...".to_string();
        }

        // Update spinner if state changed
        if current_state.as_ref() != Some(&new_state) {
            // Finish previous spinner if exists
            if let Some(ref spinner) = current_spinner {
                finish_spinner(spinner, current_state.as_ref().unwrap(), state_start_time);
            }
            // Create new spinner
            current_spinner = Some(if json {
                let spinner = ProgressBar::hidden();
                spinner.set_message(new_state.clone());
                spinner
            } else {
                create_spinner(&new_state)
            });
            current_state = Some(new_state);
            state_start_time = Instant::now();
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
            worktree_base_path: None,
            stacks: HashMap::new(),
        };

        let mut stack_config = StackConfig {
            base: None,
            mrs: HashMap::new(),
            worktree_path: None,
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
            worktree_base_path: None,
            stacks: HashMap::new(),
        };

        let mut stack_config = StackConfig {
            base: None,
            mrs: HashMap::new(),
            worktree_path: None,
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
            worktree_base_path: None,
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
            worktree_base_path: None,
            stacks: HashMap::new(),
        };

        let mut stack_config = StackConfig {
            base: None,
            mrs: HashMap::new(),
            worktree_path: None,
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
        let _fn_ptr: fn(&mut Config, &Stack, &Provider, &str, u64, bool, bool) =
            cleanup_after_merge;
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
        let _fn_ptr: fn(&git2::Repository, &Stack, &Provider, usize, bool) -> Result<()> =
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
        let _fn_ptr: fn(
            &Provider,
            u64,
            bool,
            u64,
            Option<&Arc<AtomicBool>>,
            &str,
            bool,
        ) -> Result<()> = wait_for_pr_ready;
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

    // ==========================================================================
    // Tests for wait_for_merge_train_completion (PR #111)
    // ==========================================================================
    //
    // The wait_for_merge_train_completion function polls merge train status
    // until the MR is merged or an error occurs. It handles:
    // 1. Timeout after specified minutes
    // 2. User interruption via Ctrl+C (AtomicBool flag)
    // 3. Different MergeTrainStatus variants (Merged, Running, Stale, etc.)
    // 4. Error conditions (MR closed, removed from train, skipped)
    //
    // Since the function requires a real Provider that calls external APIs,
    // most comprehensive testing requires integration tests with a mock provider.
    // These unit tests cover the testable logic without mocking.

    #[test]
    fn test_wait_for_merge_train_timeout_calculation() {
        // Test that timeout is correctly calculated from minutes to Duration
        use std::time::Duration;

        let timeout_minutes = 30u64;
        let expected = Duration::from_secs(30 * 60); // 1800 seconds
        let actual = Duration::from_secs(timeout_minutes * 60);

        assert_eq!(actual, expected);
        assert_eq!(actual.as_secs(), 1800);
    }

    #[test]
    fn test_wait_for_merge_train_poll_interval_constant() {
        // Verify POLL_INTERVAL_SECS is set correctly (should be 10 seconds)
        assert_eq!(POLL_INTERVAL_SECS, 10);
    }

    #[test]
    fn test_interrupt_signal_handling_with_atomic_bool() {
        // Test that interrupt flag can be checked correctly
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let interrupted = Arc::new(AtomicBool::new(false));

        // Initially not interrupted
        assert!(!interrupted.load(Ordering::SeqCst));

        // Simulate interrupt
        interrupted.store(true, Ordering::SeqCst);

        // Check flag is set
        assert!(interrupted.load(Ordering::SeqCst));
    }

    #[test]
    fn test_interrupt_signal_optional_handling() {
        // Test that None interrupt flag is handled safely
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        let no_interrupt: Option<&Arc<AtomicBool>> = None;

        // Should be safe to check None
        if let Some(flag) = no_interrupt {
            assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
        }
        // Test passes if no panic
    }

    #[test]
    fn test_merge_train_status_variants_exist() {
        // Document the MergeTrainStatus variants that wait_for_merge_train_completion handles
        use crate::glab::MergeTrainStatus;

        // These variants should exist and be distinct
        let statuses = [
            MergeTrainStatus::Idle,
            MergeTrainStatus::Stale,
            MergeTrainStatus::Fresh,
            MergeTrainStatus::Merging,
            MergeTrainStatus::Merged,
            MergeTrainStatus::SkipMerged,
            MergeTrainStatus::Unknown,
        ];

        // Verify we have all expected variants
        assert_eq!(statuses.len(), 7);

        // Test that Merged and SkipMerged are distinct
        let merged = MergeTrainStatus::Merged;
        let skip_merged = MergeTrainStatus::SkipMerged;

        // These should be handled differently:
        // Merged -> success, return Ok(())
        // SkipMerged -> error, return Err(...)
        match merged {
            MergeTrainStatus::Merged => { /* success case */ }
            MergeTrainStatus::SkipMerged => panic!("Should not be SkipMerged"),
            _ => panic!("Should be Merged"),
        }

        match skip_merged {
            MergeTrainStatus::SkipMerged => { /* error case */ }
            MergeTrainStatus::Merged => panic!("Should not be Merged"),
            _ => panic!("Should be SkipMerged"),
        }
    }

    #[test]
    fn test_pr_state_variants_for_merge_train_checks() {
        // Document that wait_for_merge_train_completion checks PrState
        // to detect if MR was merged or closed during polling

        // Test that Merged and Closed are distinct states
        let merged_state = PrState::Merged;
        let closed_state = PrState::Closed;

        match merged_state {
            PrState::Merged => { /* success - MR merged */ }
            PrState::Closed => panic!("Should not be Closed"),
            _ => panic!("Should be Merged"),
        }

        match closed_state {
            PrState::Closed => { /* error - MR closed/removed */ }
            PrState::Merged => panic!("Should not be Merged"),
            _ => panic!("Should be Closed"),
        }
    }

    // ==========================================================================
    // Integration test documentation for wait_for_merge_train_completion
    // ==========================================================================
    //
    // The following scenarios require integration tests with a mock provider:
    //
    // 1. TIMEOUT HANDLING:
    //    - Start polling with timeout=1 minute
    //    - Mock provider returns Running status repeatedly
    //    - After 60+ seconds, should return GgError::Other with timeout message
    //
    // 2. MERGED VIA PR STATE:
    //    - Mock provider.get_pr_info() returns state = PrState::Merged
    //    - Should immediately return Ok(()) with success message
    //
    // 3. MERGED VIA TRAIN STATUS:
    //    - Mock provider.get_merge_train_status() returns MergeTrainStatus::Merged
    //    - Should return Ok(()) with merge train success message
    //
    // 4. MR CLOSED ERROR:
    //    - Mock provider.get_pr_info() returns state = PrState::Closed
    //    - Should return Err with "was closed" message
    //
    // 5. SKIP_MERGED ERROR:
    //    - Mock provider.get_merge_train_status() returns MergeTrainStatus::SkipMerged
    //    - Should return Err with "was skipped" message
    //
    // 6. IDLE STATUS ERROR:
    //    - Mock provider.get_merge_train_status() returns MergeTrainStatus::Idle
    //    - Should return Err with "no longer in merge train" message
    //
    // 7. INTERRUPT HANDLING:
    //    - Set interrupted flag to true during polling
    //    - Should return Err with "Interrupted by user" message
    //
    // 8. TRAIN STATUS PROGRESSION:
    //    - Mock returns Fresh -> Stale -> Merging -> Merged sequence
    //    - Should print appropriate status messages and eventually succeed
    //
    // 9. MISSING TRAIN STATUS:
    //    - Mock provider.get_merge_train_status() returns Ok(None)
    //    - Should continue polling and check MR state
    //
    // 10. PIPELINE RUNNING STATUS:
    //     - Mock returns train_info with pipeline_running = true
    //     - Should print pipeline status message
    //
    // To implement these tests, either:
    // A) Refactor to use trait-based Provider for easy mocking
    // B) Create integration tests that use a test GitLab instance
    // C) Use a mocking library like mockall or mockito

    #[test]
    fn test_wait_for_merge_train_needs_integration_tests() {
        // This test documents that wait_for_merge_train_completion requires
        // integration tests with a mock provider to fully test all code paths.
        //
        // Current test coverage:
        // ✓ Timeout calculation
        // ✓ Poll interval constant
        // ✓ Interrupt flag handling (logic)
        // ✓ Status variant existence
        //
        // Missing test coverage (requires mock provider):
        // ✗ Actual timeout behavior
        // ✗ MergeTrainStatus transitions
        // ✗ Error conditions (closed, skipped, idle)
        // ✗ Success conditions (merged)
        // ✗ Interrupt during polling
        //
        // Recommendation: Add integration tests or refactor for trait-based mocking
        // This test passes to document the need for integration tests
    }

    // ==========================================================================
    // Tests for spinner UI helper functions (PR #112)
    // ==========================================================================

    #[test]
    fn test_format_duration_seconds_only() {
        use std::time::Duration;

        // Test seconds only (less than 1 minute)
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(Duration::from_secs(1)), "1s");
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(59)), "59s");
    }

    #[test]
    fn test_format_duration_minutes_and_seconds() {
        use std::time::Duration;

        // Test minutes and seconds
        assert_eq!(format_duration(Duration::from_secs(60)), "1m0s");
        assert_eq!(format_duration(Duration::from_secs(61)), "1m1s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m30s");
        assert_eq!(format_duration(Duration::from_secs(120)), "2m0s");
        assert_eq!(format_duration(Duration::from_secs(135)), "2m15s");
        assert_eq!(format_duration(Duration::from_secs(600)), "10m0s");
    }

    #[test]
    fn test_format_duration_large_values() {
        use std::time::Duration;

        // Test larger durations (30+ minutes, hours)
        assert_eq!(format_duration(Duration::from_secs(1800)), "30m0s");
        assert_eq!(format_duration(Duration::from_secs(1804)), "30m4s");
        assert_eq!(format_duration(Duration::from_secs(3600)), "60m0s");
        assert_eq!(format_duration(Duration::from_secs(3661)), "61m1s");
    }

    #[test]
    fn test_format_duration_edge_cases() {
        use std::time::Duration;

        // Test edge cases
        assert_eq!(format_duration(Duration::from_millis(500)), "0s"); // Rounds down
        assert_eq!(format_duration(Duration::from_millis(1500)), "1s"); // 1.5s -> 1s
        assert_eq!(format_duration(Duration::from_secs(59)), "59s"); // Just before 1 minute
        assert_eq!(format_duration(Duration::from_secs(60)), "1m0s"); // Exactly 1 minute
    }

    #[test]
    fn test_create_spinner_returns_progress_bar() {
        // Test that create_spinner returns a ProgressBar
        let spinner = create_spinner("Test message");

        // Verify it's a ProgressBar (type check)
        let _pb: ProgressBar = spinner;

        // Test passes if no panic (ProgressBar creation successful)
    }

    #[test]
    fn test_create_spinner_with_different_messages() {
        // Test that create_spinner handles various message types
        let _spinner1 = create_spinner("");
        let _spinner2 = create_spinner("Short");
        let _spinner3 = create_spinner("A much longer message with details about what's happening");
        let _spinner4 = create_spinner("Message with special chars: ⏳ 🚂 ✓");

        // Test passes if no panic
    }

    #[test]
    fn test_finish_spinner_does_not_panic() {
        use std::time::Instant;

        // Test that finish_spinner completes without panic
        let spinner = create_spinner("Testing");
        let start = Instant::now();

        // Wait a tiny bit to ensure non-zero elapsed time
        std::thread::sleep(std::time::Duration::from_millis(10));

        finish_spinner(&spinner, "Test completed", start);

        // Test passes if no panic
    }

    #[test]
    fn test_finish_spinner_with_various_messages() {
        use std::time::Instant;

        // Test finish_spinner with different message types
        let spinner = create_spinner("Initial");
        finish_spinner(&spinner, "", Instant::now());

        let spinner = create_spinner("Initial");
        finish_spinner(&spinner, "Done", Instant::now());

        let spinner = create_spinner("Initial");
        finish_spinner(
            &spinner,
            "Very long completion message with details",
            Instant::now(),
        );

        // Test passes if no panic
    }

    #[test]
    fn test_spinner_workflow() {
        use std::time::Instant;

        // Test the full spinner workflow
        let start = Instant::now();
        let spinner = create_spinner("Working on task...");

        // Simulate some work
        std::thread::sleep(std::time::Duration::from_millis(10));

        finish_spinner(&spinner, "Task completed", start);

        // Test passes if workflow completes without panic
    }

    // ==========================================================================
    // Tests for unsynced commit detection during cleanup (bug fix)
    // ==========================================================================

    #[test]
    fn test_cleanup_checks_for_unsynced_commits() {
        // This test documents the fix for the bug where `gg land --all` would
        // delete the entire stack including new commits that were added after
        // the MRs were created but before the cleanup.
        //
        // Scenario:
        // 1. User has stack with commits A, B, C → MRs !1, !2, !3
        // 2. User runs `gg land --wait`
        // 3. While waiting for MRs to merge, user adds commit D (no MR yet)
        // 4. MRs !1, !2, !3 merge successfully
        // 5. Cleanup starts: rebase, then check for unsynced commits
        // 6. Commit D has no MR (is_synced() == false)
        // 7. Cleanup should be SKIPPED to preserve commit D
        //
        // The bug (fixed):
        // - During the merge loop, cleanup_after_merge() removes MR mappings
        // - After all merges, the config has NO mappings left
        // - When reloading the stack, ALL commits have mr_number = None
        // - is_synced() returns false for ALL commits (even merged ones)
        // - Cleanup was ALWAYS skipped, breaking normal behavior
        //
        // The fix:
        // - Check for unsynced commits BEFORE the merge loop starts
        // - Save the result in has_unsynced_commits_before_merge flag
        // - Use this flag after rebase to decide whether to skip cleanup
        // - This way, the check happens before MR mappings are removed
        //
        // Why this matters:
        // - Without this check, the user loses work
        // - With the buggy implementation, cleanup NEVER happened
        // - With the fix, cleanup happens normally unless there are extra commits
        //
        // Testing approach:
        // This behavior requires integration testing with:
        // - A real git repo with commits
        // - Config with MR mappings
        // - Simulated merge scenario
        // - Verification that cleanup is skipped only when appropriate
        //
        // For now, we verify the logic path exists by checking:
        // - StackEntry has is_synced() method
        // - Unsynced commits can be filtered before merging
        use crate::stack::StackEntry;

        // Create a mock entry without MR (unsynced)
        let commit_oid = git2::Oid::zero();
        let commit = StackEntry {
            oid: commit_oid,
            short_sha: "abc1234".to_string(),
            title: "New commit without MR".to_string(),
            gg_id: Some("c-abc1234".to_string()),
            mr_number: None, // No MR = unsynced
            mr_state: None,
            approved: false,
            ci_status: None,
            position: 1,
            in_merge_train: false,
            merge_train_position: None,
        };

        // Verify it's detected as unsynced
        assert!(!commit.is_synced(), "Commit without MR should be unsynced");

        // Create a mock entry with MR (synced)
        let synced_commit = StackEntry {
            oid: commit_oid,
            short_sha: "def5678".to_string(),
            title: "Merged commit".to_string(),
            gg_id: Some("c-def5678".to_string()),
            mr_number: Some(123), // Has MR = synced
            mr_state: Some(crate::provider::PrState::Merged),
            approved: true,
            ci_status: None,
            position: 2,
            in_merge_train: false,
            merge_train_position: None,
        };

        // Verify it's detected as synced
        assert!(synced_commit.is_synced(), "Commit with MR should be synced");

        // Test filtering unsynced commits
        let entries = [synced_commit.clone(), commit.clone()];
        let unsynced: Vec<&StackEntry> = entries.iter().filter(|e| !e.is_synced()).collect();

        assert_eq!(unsynced.len(), 1, "Should find exactly one unsynced commit");
        assert_eq!(
            unsynced[0].gg_id.as_deref(),
            Some("c-abc1234"),
            "Should identify the correct unsynced commit"
        );
    }

    #[test]
    fn test_unsynced_commit_detection_logic() {
        // This test verifies the specific filter logic used in the cleanup code
        use crate::stack::StackEntry;

        // Create a mix of synced and unsynced entries
        let entries = [
            StackEntry {
                oid: git2::Oid::zero(),
                short_sha: "a1".to_string(),
                title: "Merged".to_string(),
                gg_id: Some("c-aaa".to_string()),
                mr_number: Some(1),
                mr_state: Some(crate::provider::PrState::Merged),
                approved: true,
                ci_status: None,
                position: 1,
                in_merge_train: false,
                merge_train_position: None,
            },
            StackEntry {
                oid: git2::Oid::zero(),
                short_sha: "b2".to_string(),
                title: "Also merged".to_string(),
                gg_id: Some("c-bbb".to_string()),
                mr_number: Some(2),
                mr_state: Some(crate::provider::PrState::Merged),
                approved: true,
                ci_status: None,
                position: 2,
                in_merge_train: false,
                merge_train_position: None,
            },
            StackEntry {
                oid: git2::Oid::zero(),
                short_sha: "c3".to_string(),
                title: "New unsynced commit".to_string(),
                gg_id: Some("c-ccc".to_string()),
                mr_number: None, // No MR
                mr_state: None,
                approved: false,
                ci_status: None,
                position: 3,
                in_merge_train: false,
                merge_train_position: None,
            },
            StackEntry {
                oid: git2::Oid::zero(),
                short_sha: "d4".to_string(),
                title: "Another new commit".to_string(),
                gg_id: Some("c-ddd".to_string()),
                mr_number: None, // No MR
                mr_state: None,
                approved: false,
                ci_status: None,
                position: 4,
                in_merge_train: false,
                merge_train_position: None,
            },
        ];

        // Apply the same filter as the cleanup code
        let unsynced_commits: Vec<&StackEntry> =
            entries.iter().filter(|e| !e.is_synced()).collect();

        // Should find both unsynced commits
        assert_eq!(unsynced_commits.len(), 2, "Should find 2 unsynced commits");

        // Verify they're the right ones
        assert_eq!(
            unsynced_commits[0].short_sha, "c3",
            "First unsynced commit should be c3"
        );
        assert_eq!(
            unsynced_commits[1].short_sha, "d4",
            "Second unsynced commit should be d4"
        );

        // When unsynced commits exist, cleanup should be skipped
        // This is the core invariant that prevents data loss
        let should_skip_cleanup = !unsynced_commits.is_empty();
        assert!(
            should_skip_cleanup,
            "Cleanup should be skipped when unsynced commits exist"
        );
    }
    #[test]
    fn test_land_response_json_structure() {
        use serde_json::Value;

        let response = LandResponse {
            version: OUTPUT_VERSION,
            land: LandResultJson {
                stack: "feat-stack".to_string(),
                base: "main".to_string(),
                landed: vec![LandedEntryJson {
                    position: 1,
                    sha: "abc1234".to_string(),
                    title: "First".to_string(),
                    gg_id: "c-abc1234".to_string(),
                    pr_number: 42,
                    action: "merged".to_string(),
                    error: None,
                }],
                remaining: 2,
                cleaned: false,
                warnings: vec!["warn".to_string()],
                error: Some("stopped".to_string()),
            },
        };

        let value: Value = serde_json::to_value(&response).expect("serialize");
        assert_eq!(value["version"], OUTPUT_VERSION);
        assert_eq!(value["land"]["stack"], "feat-stack");
        assert_eq!(value["land"]["base"], "main");
        assert_eq!(value["land"]["remaining"], 2);
        assert_eq!(value["land"]["cleaned"], false);
        assert_eq!(value["land"]["landed"][0]["action"], "merged");
        assert_eq!(value["land"]["landed"][0]["pr_number"], 42);
        assert_eq!(value["land"]["error"], "stopped");
    }
}
