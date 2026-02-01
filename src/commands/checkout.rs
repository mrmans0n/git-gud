//! `gg co` / `gg sw` - Create or switch to a stack

use console::style;
use dialoguer::FuzzySelect;
use git2::BranchType;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::provider::Provider;
use crate::stack;

/// Run the checkout command
pub fn run(stack_name: Option<String>, base: Option<String>) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Get username from config or provider
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| {
            Provider::detect(&repo)
                .ok()
                .and_then(|p| p.whoami().ok())
        })
        .ok_or_else(|| GgError::Command(
            "git-provider".to_string(),
            "Could not determine username. Set branch_username in config or authenticate with gh/glab".to_string()
        ))?;

    // If no stack name provided, show fuzzy selector
    let stack_name = match stack_name {
        Some(name) => {
            // Sanitize and validate the stack name
            let sanitized = git::sanitize_stack_name(&name)?;
            if sanitized != name {
                println!(
                    "{} Converted stack name to: {}",
                    style("→").cyan(),
                    style(&sanitized).cyan()
                );
            }
            sanitized
        }
        None => {
            // Get list of existing stacks
            let stacks = stack::list_all_stacks(&repo, &config, &username)?;

            if stacks.is_empty() {
                return Err(GgError::Other(
                    "No stacks found. Use `gg co <stack-name>` to create one.".to_string(),
                ));
            }

            let selection = FuzzySelect::new()
                .with_prompt("Select a stack")
                .items(&stacks)
                .interact()
                .map_err(|e| GgError::Other(format!("Selection cancelled: {}", e)))?;

            stacks[selection].clone()
        }
    };

    // Format the branch name
    let branch_name = git::format_stack_branch(&username, &stack_name);

    // Check if main stack branch exists
    let branch_exists = repo.find_branch(&branch_name, BranchType::Local).is_ok();

    if branch_exists {
        // Switch to existing main stack branch
        git::checkout_branch(&repo, &branch_name)?;
        println!(
            "{} Switched to stack {}",
            style("OK").green().bold(),
            style(&stack_name).cyan()
        );
    } else if let Some(entry_branch) =
        git::find_entry_branch_for_stack(&repo, &username, &stack_name)
    {
        // Main stack branch doesn't exist, but an entry branch does - use that
        git::checkout_branch(&repo, &entry_branch)?;
        println!(
            "{} Switched to stack {}",
            style("OK").green().bold(),
            style(&stack_name).cyan()
        );
    } else {
        // Stack doesn't exist locally - check if it exists on remote
        // First fetch to ensure we have up-to-date remote refs
        // Note: We use subprocess git fetch because git2's fetch requires
        // complex auth callback setup, while git CLI uses system credentials
        println!(
            "{} Checking remote for stack {}...",
            style("→").cyan(),
            style(&stack_name).cyan()
        );
        let _ = std::process::Command::new("git")
            .args(["fetch", "origin", "--prune"])
            .output();

        if check_remote_stack_exists(&repo, &username, &stack_name) {
            // Stack exists on remote - checkout
            // Try to find either the main stack branch or an entry branch
            let remote_stack_branch = format!("origin/{}/{}", username, stack_name);
            let target_branch = if repo.revparse_single(&remote_stack_branch).is_ok() {
                // Main stack branch exists
                remote_stack_branch
            } else {
                // Find an entry branch for this stack
                find_remote_entry_branch(&repo, &username, &stack_name).ok_or_else(|| {
                    GgError::Other(format!(
                        "Could not find remote branch for stack '{}'",
                        stack_name
                    ))
                })?
            };

            // Get the commit from the remote branch
            let remote_ref = repo.revparse_single(&target_branch)?;
            let remote_commit = remote_ref.peel_to_commit()?;

            // Create local stack branch pointing to this commit
            let local_branch = git::format_stack_branch(&username, &stack_name);
            repo.branch(&local_branch, &remote_commit, false)?;

            // Checkout the branch
            git::checkout_branch(&repo, &local_branch)?;

            println!(
                "{} Checked out remote stack {}",
                style("OK").green().bold(),
                style(&stack_name).cyan()
            );
        } else {
            // Create new stack
            let base_branch = base
                .or_else(|| config.defaults.base.clone())
                .or_else(|| git::find_base_branch(&repo).ok())
                .ok_or(GgError::NoBaseBranch)?;

            // Find the base commit
            let base_ref = repo
                .revparse_single(&base_branch)
                .or_else(|_| repo.revparse_single(&format!("origin/{}", base_branch)))
                .map_err(|_| GgError::NoBaseBranch)?;
            let base_commit = base_ref.peel_to_commit()?;

            // Create the branch
            repo.branch(&branch_name, &base_commit, false)?;

            // Checkout the new branch
            git::checkout_branch(&repo, &branch_name)?;

            // Initialize stack config
            let default_base = config
                .defaults
                .base
                .as_deref()
                .unwrap_or("main")
                .to_string();
            let stack_config = config.get_or_create_stack(&stack_name);
            if base_branch != default_base {
                stack_config.base = Some(base_branch.clone());
            }

            // Save username if not already set
            if config.defaults.branch_username.is_none() {
                config.defaults.branch_username = Some(username);
            }

            config.save(git_dir)?;

            println!(
                "{} Created stack {} based on {}",
                style("OK").green().bold(),
                style(&stack_name).cyan(),
                style(&base_branch).yellow()
            );
        }
    }

    Ok(())
}

/// Find a remote entry branch for a stack (returns the first one found)
fn find_remote_entry_branch(
    repo: &git2::Repository,
    username: &str,
    stack_name: &str,
) -> Option<String> {
    let branches = repo.branches(Some(git2::BranchType::Remote)).ok()?;

    for branch_result in branches.flatten() {
        if let Ok(Some(name)) = branch_result.0.name() {
            if let Some(branch_name) = name.strip_prefix("origin/") {
                if let Some((branch_user, branch_stack, _)) = git::parse_entry_branch(branch_name) {
                    if branch_user == username && branch_stack == stack_name {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }

    None
}

/// Check if a stack exists on remote (either main branch or entry branches)
fn check_remote_stack_exists(repo: &git2::Repository, username: &str, stack_name: &str) -> bool {
    // Check for main stack branch
    let remote_branch = format!("origin/{}/{}", username, stack_name);
    if repo.revparse_single(&remote_branch).is_ok() {
        return true;
    }

    // Check for entry branches (origin/username/stack--c-xxx)
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Remote)) {
        for branch_result in branches.flatten() {
            if let Ok(Some(name)) = branch_result.0.name() {
                if let Some(branch_name) = name.strip_prefix("origin/") {
                    if let Some((branch_user, branch_stack, _)) =
                        git::parse_entry_branch(branch_name)
                    {
                        if branch_user == username && branch_stack == stack_name {
                            return true;
                        }
                    }
                }
            }
        }
    }

    false
}
