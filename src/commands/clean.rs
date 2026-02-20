//! `gg clean` - Clean up merged stacks

use console::style;
use dialoguer::Confirm;
use git2::{BranchType, Repository};
use std::path::Path;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::output::{print_json, CleanResponse, CleanResultJson, OUTPUT_VERSION};
use crate::provider::{PrState, Provider};
use crate::stack;

/// Run the clean command for a specific stack (used by auto-clean)
#[allow(dead_code)]
pub fn run_for_stack(stack_name: &str, force: bool) -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "clean")?;

    run_for_stack_with_repo(&repo, stack_name, force)
}

/// Run clean for a stack with an already-open repository (no lock acquisition)
pub fn run_for_stack_with_repo(repo: &Repository, stack_name: &str, force: bool) -> Result<()> {
    let git_dir = repo.commondir();
    let mut config = Config::load(git_dir)?;

    // Detect provider (best-effort)
    let provider = Provider::detect(repo).ok();

    // Get username
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| provider.as_ref().and_then(|p| p.whoami().ok()))
        .unwrap_or_else(|| "unknown".to_string());

    git::validate_branch_username(&username)?;

    // Check if the stack exists
    let branch_name = git::format_stack_branch(&username, stack_name);

    // Check if stack is fully merged
    let merge_status = check_stack_merged(repo, &config, stack_name, &username, provider.as_ref())?;

    if !merge_status.merged && !force {
        return Err(GgError::Other(format!(
            "Stack '{}' has unmerged commits",
            stack_name
        )));
    }

    let _ = maybe_remove_configured_worktree(repo, &mut config, stack_name, false)?;

    // Delete local branch
    if let Ok(mut branch) = repo.find_branch(&branch_name, BranchType::Local) {
        // Make sure we're not on this branch
        let current = git::current_branch_name(repo);
        if current.as_deref() == Some(&branch_name) {
            // Switch to base branch first
            let base = config
                .get_base_for_stack(stack_name)
                .map(|s| s.to_string())
                .or_else(|| git::find_base_branch(repo).ok())
                .unwrap_or_else(|| "main".to_string());

            git::checkout_branch(repo, &base)?;
        }

        // Check if branch is HEAD of a worktree
        if let Some(wt_name) = git::is_branch_checked_out_in_worktree(repo, &branch_name) {
            // Try to prune if stale
            if !git::try_prune_worktree(repo, &wt_name) {
                // Worktree still exists - warn and try to remove it
                println!(
                    "{} Branch '{}' is checked out in worktree '{}'. Removing worktree.",
                    style("Note:").cyan(),
                    branch_name,
                    wt_name
                );
                let _ = git::remove_worktree(&wt_name);
            }
        }

        // Try to delete the branch, handle errors gracefully
        if let Err(e) = branch.delete() {
            println!(
                "{} Could not delete local branch '{}': {}",
                style("Warning:").yellow(),
                branch_name,
                e
            );
            println!(
                "  You may need to manually remove the worktree first: git worktree remove <path>"
            );
        }
    }

    let allow_remote_delete = merge_status.verified;
    if !allow_remote_delete {
        println!(
            "{} Skipping remote branch deletion for '{}' because merge verification is unavailable.",
            style("Warning:").yellow(),
            stack_name
        );
    }

    // Delete entry branches (local and remote when verified)
    delete_entry_branches(
        repo,
        &config,
        stack_name,
        &username,
        /*delete_remote=*/ allow_remote_delete,
        /*silent=*/ false,
    );

    // Remove from config
    config.remove_stack(stack_name);

    // Save updated config
    config.save(git_dir)?;

    Ok(())
}

/// Run the clean command
pub fn run(clean_all: bool, json: bool) -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "clean")?;

    if json && !clean_all {
        crate::output::print_json_error(
            "--json requires --all (cannot show interactive prompts in JSON mode)",
        );
        std::process::exit(1);
    }

    let git_dir = repo.commondir();
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

    git::validate_branch_username(&username)?;

    // Get all stacks
    let stacks = stack::list_all_stacks(&repo, &config, &username)?;

    let mut cleaned: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    if stacks.is_empty() {
        if json {
            print_json(&CleanResponse {
                version: OUTPUT_VERSION,
                clean: CleanResultJson { cleaned, skipped },
            });
        } else {
            println!("{}", style("No stacks to clean.").dim());
        }
        return Ok(());
    }

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
                /*silent=*/ json,
            );
            config.remove_stack(stack_name);
            cleaned.push(stack_name.clone());
            continue;
        }

        // Load the stack to check MR status
        let merge_status =
            check_stack_merged(&repo, &config, stack_name, &username, provider.as_ref())?;

        if merge_status.merged {
            if !clean_all && !json {
                let confirm = Confirm::new()
                    .with_prompt(format!("Delete merged stack '{}'? ", stack_name))
                    .default(true)
                    .interact()
                    .unwrap_or(false);

                if !confirm {
                    skipped.push(stack_name.clone());
                    continue;
                }
            }

            let removed_or_not_configured =
                maybe_remove_configured_worktree(&repo, &mut config, stack_name, json)?;
            if json && !removed_or_not_configured {
                skipped.push(format!(
                    "{} (worktree not removed: confirmation defaults to false in --json mode)",
                    stack_name
                ));
                continue;
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

                // Check if branch is HEAD of a worktree
                if let Some(wt_name) = git::is_branch_checked_out_in_worktree(&repo, &branch_name) {
                    // Try to prune if stale
                    if !git::try_prune_worktree(&repo, &wt_name) {
                        // Worktree still exists - warn and try to remove it
                        if !json {
                            println!(
                                "{} Branch '{}' is checked out in worktree '{}'. Removing worktree.",
                                style("Note:").cyan(),
                                branch_name,
                                wt_name
                            );
                        }
                        let _ = git::remove_worktree(&wt_name);
                    }
                }

                // Try to delete the branch, handle errors gracefully
                if let Err(e) = branch.delete() {
                    if !json {
                        println!(
                            "{} Could not delete local branch '{}': {}",
                            style("Warning:").yellow(),
                            branch_name,
                            e
                        );
                        println!(
                            "  You may need to manually remove the worktree first: git worktree remove <path>"
                        );
                    }
                }
            }

            let allow_remote_delete = merge_status.verified;
            if !allow_remote_delete && !json {
                println!(
                    "{} Skipping remote branch deletion for '{}' because merge verification is unavailable.",
                    style("Warning:").yellow(),
                    stack_name
                );
            }

            // Delete entry branches (local and remote when verified)
            delete_entry_branches(
                &repo,
                &config,
                stack_name,
                &username,
                /*delete_remote=*/ allow_remote_delete,
                /*silent=*/ json,
            );

            // Remove from config
            config.remove_stack(stack_name);

            if !json {
                println!(
                    "{} Deleted stack '{}'",
                    style("OK").green().bold(),
                    stack_name
                );
            }
            cleaned.push(stack_name.clone());
        } else {
            if !json {
                println!(
                    "{} Stack '{}' has unmerged commits, skipping",
                    style("○").yellow(),
                    stack_name
                );
            }
            skipped.push(stack_name.clone());
        }
    }

    // Save updated config
    config.save(git_dir)?;

    if json {
        print_json(&CleanResponse {
            version: OUTPUT_VERSION,
            clean: CleanResultJson { cleaned, skipped },
        });
    } else if !cleaned.is_empty() {
        println!();
        println!(
            "{} Cleaned {} stack(s)",
            style("OK").green().bold(),
            cleaned.len()
        );
    } else {
        println!("{}", style("No stacks to clean.").dim());
    }

    Ok(())
}

fn maybe_remove_configured_worktree(
    repo: &Repository,
    config: &mut Config,
    stack_name: &str,
    silent: bool,
) -> Result<bool> {
    let Some(stack_cfg) = config.get_stack(stack_name) else {
        return Ok(true);
    };
    let Some(worktree_path) = stack_cfg.worktree_path.clone() else {
        return Ok(true);
    };

    let confirm = if silent {
        false
    } else {
        let prompt = format!(
            "Stack '{}' has an associated worktree at '{}'. Remove it?",
            stack_name, worktree_path
        );
        Confirm::new()
            .with_prompt(prompt)
            .default(false)
            .interact()
            .unwrap_or(false)
    };

    if !confirm {
        if !silent {
            println!(
                "{} Keeping worktree '{}'.",
                style("Note:").cyan(),
                worktree_path
            );
        }
        return Ok(false);
    }

    let repo_root = repo
        .workdir()
        .ok_or_else(|| GgError::Other("Repository has no working directory".to_string()))?;

    let output = std::process::Command::new("git")
        .arg("worktree")
        .arg("remove")
        .arg(&worktree_path)
        .arg("--force")
        .current_dir(repo_root)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to remove worktree '{}': {}",
            worktree_path,
            stderr.trim()
        )));
    }

    if let Some(stack_cfg_mut) = config.stacks.get_mut(stack_name) {
        stack_cfg_mut.worktree_path = None;
    }

    if !silent && !Path::new(&worktree_path).exists() {
        println!(
            "{} Removed worktree '{}'.",
            style("OK").green().bold(),
            worktree_path
        );
    }

    Ok(true)
}

#[derive(Debug, Clone, Copy)]
struct MergeStatus {
    merged: bool,
    /// True when merge state was verified via provider checks.
    verified: bool,
}

/// Check if a stack is fully merged
fn check_stack_merged(
    repo: &git2::Repository,
    config: &Config,
    stack_name: &str,
    username: &str,
    provider: Option<&Provider>,
) -> Result<MergeStatus> {
    // Primary method: check if all MRs are merged.
    // This works correctly with squash and rebase merges.
    if let Some(stack_config) = config.get_stack(stack_name) {
        if !stack_config.mrs.is_empty() {
            if let Some(provider) = provider {
                let mut all_merged = true;
                let mut provider_verified = true;
                for mr_num in stack_config.mrs.values() {
                    match provider.get_pr_info(*mr_num) {
                        Ok(info) => {
                            if info.state != PrState::Merged {
                                all_merged = false;
                            }
                        }
                        Err(_) => {
                            // PR/MR might be deleted or inaccessible.
                            provider_verified = false;
                            all_merged = false;
                            break;
                        }
                    }
                    if !all_merged {
                        break;
                    }
                }

                // Note: we intentionally do NOT call verify_commits_reachable here.
                // With squash or rebase merges, the local commit SHAs will differ from
                // what's on the base branch, so a revwalk check would incorrectly report
                // unmerged commits. The provider API confirmation is authoritative.

                if all_merged {
                    return Ok(MergeStatus {
                        merged: true,
                        verified: provider_verified,
                    });
                }
            }
        }
    }

    // Fallback: check if stack branch is ancestor of base.
    // This supports local/manual merge simulations even when provider checks are unavailable
    // or cannot be trusted.
    let merged = is_stack_branch_ancestor_of_base(repo, config, stack_name, username)?;

    Ok(MergeStatus {
        merged,
        verified: false,
    })
}

fn is_stack_branch_ancestor_of_base(
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

    Ok(repo.merge_base(stack_oid, base_oid)? == stack_oid)
}

#[allow(dead_code)]
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
    silent: bool,
) {
    // First, delete entry branches from config (if any)
    if let Some(stack_config) = config.get_stack(stack_name) {
        for entry_id in stack_config.mrs.keys() {
            let entry_id = match git::normalize_gg_id(entry_id) {
                Some(id) => id,
                None => {
                    if !silent {
                        println!(
                            "{} Skipping invalid GG-ID '{}' in config.",
                            style("Warning:").yellow(),
                            entry_id
                        );
                    }
                    continue;
                }
            };
            let entry_branch = git::format_entry_branch(username, stack_name, &entry_id);
            // Delete local entry branch
            if let Ok(mut branch) = repo.find_branch(&entry_branch, BranchType::Local) {
                // Check if branch is HEAD of a worktree
                if let Some(wt_name) = git::is_branch_checked_out_in_worktree(repo, &entry_branch) {
                    // Try to prune if stale
                    if !git::try_prune_worktree(repo, &wt_name) {
                        // Worktree still exists - try to remove it silently
                        let _ = git::remove_worktree(&wt_name);
                    }
                }
                // Try to delete, ignore errors (best effort for entry branches)
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
            // Check if branch is HEAD of a worktree
            if let Some(wt_name) = git::is_branch_checked_out_in_worktree(repo, &branch_name) {
                // Try to prune if stale
                if !git::try_prune_worktree(repo, &wt_name) {
                    // Worktree still exists - try to remove it silently
                    let _ = git::remove_worktree(&wt_name);
                }
            }
            // Try to delete, ignore errors (best effort for entry branches)
            let _ = branch.delete();
        }
        // Also try to delete from remote
        if delete_remote {
            let _ = git::delete_remote_branch(&branch_name);
        }
    }
}
