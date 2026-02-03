//! `gg clean` - Clean up merged stacks

use console::style;
use dialoguer::Confirm;
use git2::BranchType;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::provider::{PrState, Provider};
use crate::stack;

/// Run the clean command for a specific stack (used by auto-clean)
pub fn run_for_stack(stack_name: &str, force: bool) -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "clean")?;

    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Detect provider (best-effort)
    let provider = Provider::detect(&repo).ok();

    // Get username
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| provider.as_ref().and_then(|p| p.whoami().ok()))
        .unwrap_or_else(|| "unknown".to_string());

    // Check if the stack exists
    let branch_name = git::format_stack_branch(&username, stack_name);

    // Check if stack is fully merged
    let is_merged = check_stack_merged(&repo, &config, stack_name, &username, provider.as_ref())?;

    if !is_merged && !force {
        return Err(GgError::Other(format!(
            "Stack '{}' has unmerged commits",
            stack_name
        )));
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

    // Delete entry branches (local and remote)
    delete_entry_branches(
        &repo, &config, stack_name, &username, /*delete_remote=*/ true,
    );

    // Remove from config
    config.remove_stack(stack_name);

    // Save updated config
    config.save(git_dir)?;

    Ok(())
}

/// Run the clean command
pub fn run(clean_all: bool) -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "clean")?;

    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Detect provider (best-effort).
    // Some repos (e.g. local remotes in tests) won't match GitHub/GitLab.
    let provider = Provider::detect(&repo).ok();

    // Get username
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| provider.as_ref().and_then(|p| p.whoami().ok()))
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
            // Branch doesn't exist: clean LOCAL orphan entry branches and config.
            // Be conservative: do NOT delete remote branches here because we can't
            // reliably verify merge status without the main stack branch.
            delete_entry_branches(
                &repo, &config, stack_name, &username, /*delete_remote=*/ false,
            );
            config.remove_stack(stack_name);
            cleaned_count += 1;
            continue;
        }

        // Load the stack to check MR status
        let is_merged =
            check_stack_merged(&repo, &config, stack_name, &username, provider.as_ref())?;

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

            // Delete entry branches (local and remote)
            delete_entry_branches(
                &repo, &config, stack_name, &username, /*delete_remote=*/ true,
            );

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
    provider: Option<&Provider>,
) -> Result<bool> {
    // Primary method: check if all MRs are merged
    // This works correctly with squash and rebase merges
    if let Some(stack_config) = config.get_stack(stack_name) {
        if stack_config.mrs.is_empty() {
            // No MRs tracked, fall back to git merge check
        } else if let Some(provider) = provider {
            let mut all_merged = true;
            for mr_num in stack_config.mrs.values() {
                match provider.get_pr_info(*mr_num) {
                    Ok(info) => {
                        if info.state != PrState::Merged {
                            all_merged = false;
                            break;
                        }
                    }
                    Err(_) => {
                        // PR/MR might be deleted or inaccessible
                        // If we can't verify, assume it's not merged to be safe
                        all_merged = false;
                        break;
                    }
                }
            }

            // Additional safety: verify commits are reachable from base branch
            // This catches edge cases where PR is marked merged but commits aren't in base
            if all_merged
                && verify_commits_reachable(repo, config, stack_name, username).is_err()
            {
                // If we can't verify reachability, be conservative
                all_merged = false;
            }

            return Ok(all_merged);
        }
    }

    // Fallback: check if stack branch is ancestor of base
    // This works for merge commits but not for squash/rebase
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
    Ok(repo.merge_base(stack_oid, base_oid)? == stack_oid)
}

/// Verify that all commits in the stack are reachable from the base branch
/// This provides additional safety before deleting remote branches
fn verify_commits_reachable(
    repo: &git2::Repository,
    config: &Config,
    stack_name: &str,
    username: &str,
) -> Result<()> {
    let branch_name = git::format_stack_branch(username, stack_name);

    // Get base branch
    let base = config
        .get_base_for_stack(stack_name)
        .map(|s| s.to_string())
        .or_else(|| git::find_base_branch(repo).ok())
        .ok_or(GgError::NoBaseBranch)?;

    // Get base commit (prefer origin/<base> for most up-to-date state)
    let base_ref = repo
        .revparse_single(&format!("origin/{}", base))
        .or_else(|_| repo.revparse_single(&base))?;
    let base_commit = base_ref.peel_to_commit()?;

    // Get stack branch commit
    let stack_ref = repo.revparse_single(&branch_name)?;
    let stack_commit = stack_ref.peel_to_commit()?;

    // Walk commits from stack to base
    let mut revwalk = repo.revwalk()?;
    revwalk.push(stack_commit.id())?;
    revwalk.hide(base_commit.id())?;

    // If there are any commits not reachable from base, it's not fully merged
    if let Some(oid) = revwalk.next() {
        let _oid = oid?;
        // If we get here, there are commits in stack not in base
        return Err(GgError::Other(format!(
            "Stack '{}' has commits not reachable from {}",
            stack_name, base
        )));
    }

    Ok(())
}

/// Delete all entry branches for a stack (both local and remote)
fn delete_entry_branches(
    repo: &git2::Repository,
    config: &Config,
    stack_name: &str,
    username: &str,
    delete_remote: bool,
) {
    // First, delete entry branches from config (if any)
    if let Some(stack_config) = config.get_stack(stack_name) {
        for entry_id in stack_config.mrs.keys() {
            let entry_branch = git::format_entry_branch(username, stack_name, entry_id);
            // Delete local entry branch
            if let Ok(mut branch) = repo.find_branch(&entry_branch, BranchType::Local) {
                let _ = branch.delete();
            }
            // Delete remote entry branch
            if delete_remote {
                let _ = git::delete_remote_branch(&entry_branch);
            }
        }
    }

    // Also scan for any orphaned entry branches matching this stack
    // (in case config is out of sync with actual branches)
    let branches: Vec<String> = repo
        .branches(Some(BranchType::Local))
        .ok()
        .map(|branches| {
            branches
                .filter_map(|b| b.ok())
                .filter_map(|(branch, _)| branch.name().ok().flatten().map(String::from))
                .filter(|name| {
                    // Match branches like "username/stack-name--c-XXXXX"
                    if let Some((branch_user, branch_stack, _)) = git::parse_entry_branch(name) {
                        branch_user == username && branch_stack == *stack_name
                    } else {
                        false
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    for branch_name in branches {
        if let Ok(mut branch) = repo.find_branch(&branch_name, BranchType::Local) {
            let _ = branch.delete();
        }
        // Also try to delete from remote
        if delete_remote {
            let _ = git::delete_remote_branch(&branch_name);
        }
    }
}
