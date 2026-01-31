//! `gg land` - Merge approved MRs starting from the first commit

use console::style;
use dialoguer::Confirm;

use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::provider::{self, PrState};
use crate::stack::Stack;

/// Run the land command
pub fn run(land_all: bool, squash: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Get provider and check it's available
    let provider = provider::get_provider(&config, &repo)?;
    provider.check_installed()?;
    provider.check_auth()?;

    // Require clean working directory
    git::require_clean_working_directory(&repo)?;

    // Load stack and refresh PR/MR info
    let mut stack = Stack::load(&repo, &config)?;

    if stack.is_empty() {
        println!("{}", style("Stack is empty. Nothing to land.").dim());
        return Ok(());
    }

    println!("{}", style("Checking PR/MR status...").dim());
    stack.refresh_mr_info(provider.as_ref())?;

    let pr_prefix = provider.pr_prefix();

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

        // Check PR/MR state
        let mr_info = provider.view_pr(mr_num)?;

        match mr_info.state {
            PrState::Merged => {
                println!(
                    "{} {}{} already merged",
                    style("✓").green(),
                    pr_prefix,
                    mr_num
                );
                landed_count += 1;
                continue;
            }
            PrState::Closed => {
                println!(
                    "{} {}{} is closed. Stopping.",
                    style("✗").red(),
                    pr_prefix,
                    mr_num
                );
                break;
            }
            PrState::Draft => {
                println!(
                    "{} {}{} is a draft. Mark as ready before landing.",
                    style("○").yellow(),
                    pr_prefix,
                    mr_num
                );
                break;
            }
            PrState::Open => {
                // Check if approved
                let approved = provider.check_approved(mr_num)?;
                if !approved {
                    println!(
                        "{} {}{} is not approved. Stopping.",
                        style("○").yellow(),
                        pr_prefix,
                        mr_num
                    );
                    break;
                }
            }
        }

        // PR/MR is approved and open - land it
        if !land_all {
            let confirm = Confirm::new()
                .with_prompt(format!(
                    "Merge {}{} ({})? ",
                    pr_prefix, mr_num, entry.title
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
            "{} Merging {}{}...",
            style("→").cyan(),
            pr_prefix,
            mr_num
        );

        match provider.merge_pr(mr_num, squash, true) {
            Ok(()) => {
                println!(
                    "{} Merged {}{} into {}",
                    style("OK").green().bold(),
                    pr_prefix,
                    mr_num,
                    stack.base
                );
                landed_count += 1;

                // Remove PR/MR mapping from config
                config.remove_mr_for_entry(&stack.name, gg_id);
            }
            Err(e) => {
                println!(
                    "{} Failed to merge {}{}: {}",
                    style("Error:").red().bold(),
                    pr_prefix,
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
