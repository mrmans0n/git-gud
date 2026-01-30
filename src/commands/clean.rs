//! `gg clean` - Clean up merged stacks

use console::style;
use dialoguer::Confirm;
use git2::BranchType;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::glab::{self, MrState};
use crate::stack;

/// Run the clean command
pub fn run(clean_all: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Get username
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| glab::whoami().ok())
        .unwrap_or_else(|| "unknown".to_string());

    // Get all stacks
    let stacks = stack::list_all_stacks(&repo, &config, &username)?;

    if stacks.is_empty() {
        println!("{}", style("No stacks to clean.").dim());
        return Ok(());
    }

    let mut cleaned_count = 0;

    for stack_name in &stacks {
        // Try to determine if stack is fully merged
        let branch_name = git::format_stack_branch(&username, stack_name);

        // Check if the branch exists
        if repo.find_branch(&branch_name, BranchType::Local).is_err() {
            // Branch doesn't exist, just clean config
            config.remove_stack(stack_name);
            cleaned_count += 1;
            continue;
        }

        // Load the stack to check MR status
        let is_merged = check_stack_merged(&repo, &config, stack_name, &username)?;

        if is_merged {
            if !clean_all {
                let confirm = Confirm::new()
                    .with_prompt(format!("Delete merged stack '{}'? ", stack_name))
                    .default(true)
                    .interact()
                    .unwrap_or(false);

                if !confirm {
                    continue;
                }
            }

            // Delete local branch
            if let Ok(mut branch) = repo.find_branch(&branch_name, BranchType::Local) {
                // Make sure we're not on this branch
                let current = git::current_branch_name(&repo);
                if current.as_deref() == Some(&branch_name) {
                    // Switch to base branch first
                    let base = config
                        .get_base_for_stack(stack_name)
                        .map(|s| s.to_string())
                        .or_else(|| git::find_base_branch(&repo).ok())
                        .unwrap_or_else(|| "main".to_string());

                    git::checkout_branch(&repo, &base)?;
                }

                branch.delete()?;
            }

            // Delete remote entry branches
            if let Some(stack_config) = config.get_stack(stack_name) {
                for entry_id in stack_config.mrs.keys() {
                    let entry_branch = git::format_entry_branch(&username, stack_name, entry_id);
                    // Try to delete remote branch (ignore errors)
                    let _ = git::delete_remote_branch(&entry_branch);
                }
            }

            // Remove from config
            config.remove_stack(stack_name);

            println!(
                "{} Deleted stack '{}'",
                style("OK").green().bold(),
                stack_name
            );
            cleaned_count += 1;
        } else {
            println!(
                "{} Stack '{}' has unmerged commits, skipping",
                style("○").yellow(),
                stack_name
            );
        }
    }

    // Save updated config
    config.save(git_dir)?;

    if cleaned_count > 0 {
        println!();
        println!(
            "{} Cleaned {} stack(s)",
            style("OK").green().bold(),
            cleaned_count
        );
    } else {
        println!("{}", style("No stacks to clean.").dim());
    }

    Ok(())
}

/// Check if a stack is fully merged
fn check_stack_merged(
    repo: &git2::Repository,
    config: &Config,
    stack_name: &str,
    username: &str,
) -> Result<bool> {
    let branch_name = git::format_stack_branch(username, stack_name);

    // Get base branch
    let base = config
        .get_base_for_stack(stack_name)
        .map(|s| s.to_string())
        .or_else(|| git::find_base_branch(repo).ok())
        .ok_or(GgError::NoBaseBranch)?;

    // Check if stack branch is ancestor of base (fully merged)
    let stack_ref = repo.revparse_single(&branch_name)?;
    let base_ref = repo
        .revparse_single(&base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", base)))?;

    let stack_oid = stack_ref.id();
    let base_oid = base_ref.id();

    // If stack is ancestor of base, it's merged
    if repo.merge_base(stack_oid, base_oid)? == stack_oid {
        return Ok(true);
    }

    // Alternative: check if all MRs are merged
    if let Some(stack_config) = config.get_stack(stack_name) {
        if stack_config.mrs.is_empty() {
            // No MRs tracked, can't determine merge status
            return Ok(false);
        }

        let mut all_merged = true;
        for mr_num in stack_config.mrs.values() {
            match glab::view_mr(*mr_num) {
                Ok(info) => {
                    if info.state != MrState::Merged {
                        all_merged = false;
                        break;
                    }
                }
                Err(_) => {
                    // MR might be deleted, assume not merged
                    all_merged = false;
                    break;
                }
            }
        }

        return Ok(all_merged);
    }

    Ok(false)
}
