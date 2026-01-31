//! `gg land` - Merge approved PRs/MRs starting from the first commit

use console::style;
use dialoguer::Confirm;

use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::provider::{PrState, Provider};
use crate::stack::Stack;

/// Run the land command
pub fn run(land_all: bool, squash: bool) -> Result<()> {
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

    // Find landable PRs (approved, open, from the start of the stack)
    let mut landed_count = 0;

    for entry in &stack.entries {
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

        // PR/MR is approved and open - land it
        if !land_all {
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

        // Wait a bit for GitHub to process
        std::thread::sleep(std::time::Duration::from_secs(2));
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
