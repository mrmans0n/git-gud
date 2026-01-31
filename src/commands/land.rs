//! `gg land` - Merge approved PRs starting from the first commit

use console::style;
use dialoguer::Confirm;

use crate::config::Config;
use crate::error::Result;
use crate::gh::{self, PrState};
use crate::git;
use crate::stack::Stack;

/// Run the land command
pub fn run(land_all: bool, squash: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Check gh is available
    gh::check_gh_installed()?;
    gh::check_gh_auth()?;

    // Require clean working directory
    git::require_clean_working_directory(&repo)?;

    // Load stack and refresh PR info
    let mut stack = Stack::load(&repo, &config)?;

    if stack.is_empty() {
        println!("{}", style("Stack is empty. Nothing to land.").dim());
        return Ok(());
    }

    println!("{}", style("Checking PR status...").dim());
    stack.refresh_mr_info()?;

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
                    "{} Commit {} has no PR. Run `gg sync` first.",
                    style("Error:").red().bold(),
                    entry.short_sha
                );
                break;
            }
        };

        // Check PR state
        let pr_info = gh::get_pr_info(pr_num)?;

        match pr_info.state {
            PrState::Merged => {
                println!("{} PR #{} already merged", style("✓").green(), pr_num);
                landed_count += 1;
                continue;
            }
            PrState::Closed => {
                println!("{} PR #{} is closed. Stopping.", style("✗").red(), pr_num);
                break;
            }
            PrState::Draft => {
                println!(
                    "{} PR #{} is a draft. Mark as ready before landing.",
                    style("○").yellow(),
                    pr_num
                );
                break;
            }
            PrState::Open => {
                // Check if approved (skip if --all is used)
                if !land_all {
                    let approved = gh::check_pr_approved(pr_num)?;
                    if !approved {
                        println!(
                            "{} PR #{} is not approved. Stopping.",
                            style("○").yellow(),
                            pr_num
                        );
                        break;
                    }
                }
            }
        }

        // PR is approved and open - land it
        if !land_all {
            let confirm = Confirm::new()
                .with_prompt(format!("Merge PR #{} ({})? ", pr_num, entry.title))
                .default(true)
                .interact()
                .unwrap_or(false);

            if !confirm {
                println!("{}", style("Stopping.").dim());
                break;
            }
        }

        println!("{} Merging PR #{}...", style("→").cyan(), pr_num);

        match gh::merge_pr(pr_num, squash, false) {
            Ok(()) => {
                println!(
                    "{} Merged PR #{} into {}",
                    style("OK").green().bold(),
                    pr_num,
                    stack.base
                );
                landed_count += 1;

                // Remove PR mapping from config
                config.remove_mr_for_entry(&stack.name, gg_id);

                // Update the base of remaining PRs to point to the main branch
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
                                    "  Updating PR #{} base to {}...",
                                    remaining_pr, stack.base
                                ))
                                .dim()
                            );
                            if let Err(e) = gh::update_pr_base(remaining_pr, &stack.base) {
                                println!(
                                    "{} Warning: Failed to update PR #{} base: {}",
                                    style("⚠").yellow(),
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
                    "{} Failed to merge PR #{}: {}",
                    style("Error:").red().bold(),
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
            "{} Landed {} PR(s)",
            style("OK").green().bold(),
            landed_count
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
                style("  All PRs landed! Run `gg clean` to remove the stack.").dim()
            );
        }
    }

    Ok(())
}
