//! `gg land` - Merge approved MRs starting from the first commit

use console::style;
use dialoguer::Confirm;

use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::glab::{self, MrState};
use crate::stack::Stack;

/// Run the land command
pub fn run(land_all: bool, squash: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Check glab is available
    glab::check_glab_installed()?;
    glab::check_glab_auth()?;

    // Require clean working directory
    git::require_clean_working_directory(&repo)?;

    // Load stack and refresh MR info
    let mut stack = Stack::load(&repo, &config)?;

    if stack.is_empty() {
        println!("{}", style("Stack is empty. Nothing to land.").dim());
        return Ok(());
    }

    println!("{}", style("Checking MR status...").dim());
    stack.refresh_mr_info()?;

    // Find landable MRs (approved, open, from the start of the stack)
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

        let mr_num = match entry.mr_number {
            Some(num) => num,
            None => {
                println!(
                    "{} Commit {} has no MR. Run `gg sync` first.",
                    style("Error:").red().bold(),
                    entry.short_sha
                );
                break;
            }
        };

        // Check MR state
        let mr_info = glab::view_mr(mr_num)?;

        match mr_info.state {
            MrState::Merged => {
                println!(
                    "{} MR !{} already merged",
                    style("✓").green(),
                    mr_num
                );
                landed_count += 1;
                continue;
            }
            MrState::Closed => {
                println!(
                    "{} MR !{} is closed. Stopping.",
                    style("✗").red(),
                    mr_num
                );
                break;
            }
            MrState::Draft => {
                println!(
                    "{} MR !{} is a draft. Mark as ready before landing.",
                    style("○").yellow(),
                    mr_num
                );
                break;
            }
            MrState::Open => {
                // Check if approved
                let approved = glab::check_mr_approved(mr_num)?;
                if !approved {
                    println!(
                        "{} MR !{} is not approved. Stopping.",
                        style("○").yellow(),
                        mr_num
                    );
                    break;
                }
            }
        }

        // MR is approved and open - land it
        if !land_all {
            let confirm = Confirm::new()
                .with_prompt(format!(
                    "Merge MR !{} ({})? ",
                    mr_num, entry.title
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
            "{} Merging MR !{}...",
            style("→").cyan(),
            mr_num
        );

        match glab::merge_mr(mr_num, squash, true) {
            Ok(()) => {
                println!(
                    "{} Merged MR !{} into {}",
                    style("OK").green().bold(),
                    mr_num,
                    stack.base
                );
                landed_count += 1;

                // Remove MR mapping from config
                config.remove_mr_for_entry(&stack.name, gg_id);
            }
            Err(e) => {
                println!(
                    "{} Failed to merge MR !{}: {}",
                    style("Error:").red().bold(),
                    mr_num,
                    e
                );
                break;
            }
        }

        if !land_all {
            break;
        }

        // Wait a bit for GitLab to process
        std::thread::sleep(std::time::Duration::from_secs(2));
    }

    // Save updated config
    config.save(git_dir)?;

    if landed_count > 0 {
        println!();
        println!(
            "{} Landed {} MR(s)",
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
                style("  All MRs landed! Run `gg clean` to remove the stack.").dim()
            );
        }
    }

    Ok(())
}
