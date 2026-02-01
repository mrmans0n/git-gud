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
        Some(name) => name,
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
    } else if check_remote_stack_exists(&repo, &username, &stack_name) {
        // Stack exists on remote but not locally - fetch and checkout
        let remote_branch = format!("origin/{}/{}", username, stack_name);
        let local_branch = git::format_stack_branch(&username, &stack_name);

        println!(
            "{} Fetching remote stack {}...",
            style("→").cyan(),
            style(&stack_name).cyan()
        );

        // Fetch the specific branch
        std::process::Command::new("git")
            .args(["fetch", "origin", &format!("{}/{}", username, stack_name)])
            .output()
            .map_err(|e| GgError::Command("git fetch".to_string(), e.to_string()))?;

        // Create local branch tracking the remote
        let remote_ref = repo.revparse_single(&remote_branch)?;
        let remote_commit = remote_ref.peel_to_commit()?;
        repo.branch(&local_branch, &remote_commit, false)?;

        // Set up tracking
        let _ = std::process::Command::new("git")
            .args(["branch", "--set-upstream-to", &remote_branch, &local_branch])
            .output();

        // Also fetch entry branches for this stack
        let _ = std::process::Command::new("git")
            .args([
                "fetch",
                "origin",
                &format!(
                    "refs/heads/{}/{}--*:refs/remotes/origin/{}/{}--*",
                    username, stack_name, username, stack_name
                ),
            ])
            .output();

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

    Ok(())
}

/// Check if a stack exists on remote
fn check_remote_stack_exists(repo: &git2::Repository, username: &str, stack_name: &str) -> bool {
    let remote_branch = format!("origin/{}/{}", username, stack_name);
    repo.revparse_single(&remote_branch).is_ok()
}
