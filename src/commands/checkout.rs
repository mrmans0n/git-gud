//! `gg co` / `gg sw` - Create or switch to a stack

use console::style;
use dialoguer::FuzzySelect;
use git2::BranchType;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::glab;
use crate::stack;

/// Run the checkout command
pub fn run(stack_name: Option<String>, base: Option<String>) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Get username from config or glab
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| glab::whoami().ok())
        .ok_or_else(|| GgError::GlabError(
            "Could not determine username. Set branch_username in config or run `glab auth login`".to_string()
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

    // Check if branch exists
    let branch_exists = repo.find_branch(&branch_name, BranchType::Local).is_ok();

    if branch_exists {
        // Switch to existing branch
        git::checkout_branch(&repo, &branch_name)?;

        // Save as current stack
        config.defaults.current_stack = Some(stack_name.clone());
        config.save(git_dir)?;

        println!(
            "{} Switched to stack {}",
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

        // Save as current stack
        config.defaults.current_stack = Some(stack_name.clone());

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
