//! `gg co` / `gg sw` - Create or switch to a stack

use console::style;
use dialoguer::FuzzySelect;
use git2::BranchType;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::provider::Provider;
use crate::stack;

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

/// Run the checkout command
pub fn run(stack_name: Option<String>, base: Option<String>, use_worktree: bool) -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "checkout")?;

    let git_dir = repo.commondir();
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

    git::validate_branch_username(&username)?;

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
        if use_worktree {
            let worktree_path =
                ensure_stack_worktree(&repo, &mut config, &stack_name, &branch_name)?;
            println!(
                "{} Opened stack {} in worktree {}",
                style("OK").green().bold(),
                style(&stack_name).cyan(),
                style(worktree_path.display()).yellow()
            );
        } else {
            // Switch to existing main stack branch
            git::checkout_branch(&repo, &branch_name)?;
            println!(
                "{} Switched to stack {}",
                style("OK").green().bold(),
                style(&stack_name).cyan()
            );
        }
    } else if let Some(entry_branch) =
        git::find_entry_branch_for_stack(&repo, &username, &stack_name)
    {
        // Main stack branch doesn't exist, but an entry branch does - use that
        if use_worktree {
            let worktree_path =
                ensure_stack_worktree(&repo, &mut config, &stack_name, &entry_branch)?;
            println!(
                "{} Opened stack {} in worktree {}",
                style("OK").green().bold(),
                style(&stack_name).cyan(),
                style(worktree_path.display()).yellow()
            );
        } else {
            git::checkout_branch(&repo, &entry_branch)?;
            println!(
                "{} Switched to stack {}",
                style("OK").green().bold(),
                style(&stack_name).cyan()
            );
        }
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

            if use_worktree {
                ensure_stack_worktree(&repo, &mut config, &stack_name, &local_branch)?;
            } else {
                // Checkout the branch
                git::checkout_branch(&repo, &local_branch)?;
            }

            // Import PR mappings from remote
            if let Err(e) =
                import_pr_mappings_for_remote_stack(&repo, &mut config, &username, &stack_name)
            {
                println!(
                    "{} Could not import PR mappings: {}",
                    style("Warning:").yellow(),
                    e
                );
                println!(
                    "{}",
                    style("Continuing without PR mappings. Run `gg sync` to create/update PRs.")
                        .dim()
                );
            }

            if use_worktree {
                let worktree_path = config
                    .get_stack(&stack_name)
                    .and_then(|stack| stack.worktree_path.as_deref())
                    .unwrap_or("<unknown>");
                println!(
                    "{} Checked out remote stack {} in worktree {}",
                    style("OK").green().bold(),
                    style(&stack_name).cyan(),
                    style(worktree_path).yellow()
                );
            } else {
                println!(
                    "{} Checked out remote stack {}",
                    style("OK").green().bold(),
                    style(&stack_name).cyan()
                );
            }
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

            // Checkout the new branch unless user requested a worktree
            if !use_worktree {
                git::checkout_branch(&repo, &branch_name)?;
            }

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

            if use_worktree {
                let worktree_path =
                    ensure_stack_worktree(&repo, &mut config, &stack_name, &branch_name)?;
                println!(
                    "{} Created stack {} based on {} in worktree {}",
                    style("OK").green().bold(),
                    style(&stack_name).cyan(),
                    style(&base_branch).yellow(),
                    style(worktree_path.display()).yellow()
                );
            } else {
                println!(
                    "{} Created stack {} based on {}",
                    style("OK").green().bold(),
                    style(&stack_name).cyan(),
                    style(&base_branch).yellow()
                );
            }
        }
    }

    Ok(())
}

fn ensure_stack_worktree(
    repo: &git2::Repository,
    config: &mut Config,
    stack_name: &str,
    branch_name: &str,
) -> Result<std::path::PathBuf> {
    let git_dir = repo.commondir();
    let repo_root = repo
        .workdir()
        .ok_or_else(|| GgError::Other("Repository has no working directory".to_string()))?;

    let maybe_existing = config
        .get_stack(stack_name)
        .and_then(|stack| stack.worktree_path.as_deref())
        .map(std::path::PathBuf::from);

    let target_path =
        maybe_existing.unwrap_or_else(|| config.render_worktree_path(repo_root, stack_name));

    if !is_worktree_registered(repo_root, &target_path) {
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let output = Command::new("git")
            .arg("worktree")
            .arg("add")
            .arg(&target_path)
            .arg(branch_name)
            .current_dir(repo_root)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GgError::Other(format!(
                "Failed to create worktree at '{}': {}",
                target_path.display(),
                stderr.trim()
            )));
        }
    }

    let stack_cfg = config.get_or_create_stack(stack_name);
    stack_cfg.worktree_path = Some(target_path.to_string_lossy().to_string());
    config.save(git_dir)?;

    Ok(target_path)
}

fn is_worktree_registered(repo_root: &Path, target_path: &Path) -> bool {
    let output = Command::new("git")
        .arg("worktree")
        .arg("list")
        .arg("--porcelain")
        .current_dir(repo_root)
        .output();

    let Ok(output) = output else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    let target = target_path.to_string_lossy();
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .any(|line| line == format!("worktree {}", target))
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

/// Import PR/MR mappings for a remote stack by querying the provider
/// This is called after checking out a remote stack to populate the local config
/// with existing PR/MR numbers, preventing "PR already exists" errors on sync.
fn import_pr_mappings_for_remote_stack(
    repo: &git2::Repository,
    config: &mut Config,
    username: &str,
    stack_name: &str,
) -> Result<()> {
    // Detect and check provider
    let provider = Provider::detect(repo)?;

    // Only attempt to import if provider tools are installed and authenticated
    // If they're not, just skip silently - sync will handle it later
    if provider.check_installed().is_err() || provider.check_auth().is_err() {
        return Ok(());
    }

    let git_dir = repo.commondir();
    let mut imported_count = 0;
    let mut skipped_branches: HashSet<String> = HashSet::new();

    // Find all entry branches for this stack (both local and remote)
    let branches = repo.branches(None)?;

    for branch_result in branches {
        let (branch, branch_type) = branch_result?;
        let branch_name = match branch.name()? {
            Some(name) => name,
            None => continue,
        };

        // Parse the branch name to extract username, stack name, and GG-ID
        let parsed = match branch_type {
            BranchType::Local => git::parse_entry_branch(branch_name),
            BranchType::Remote => {
                // Strip "origin/" prefix for remote branches
                branch_name
                    .strip_prefix("origin/")
                    .and_then(git::parse_entry_branch)
            }
        };

        let (branch_user, branch_stack, gg_id) = match parsed {
            Some(p) => p,
            None => continue,
        };

        // Only process branches for this specific stack
        if branch_user != username || branch_stack != stack_name {
            continue;
        }

        // Check if we already have a mapping for this GG-ID
        if config.get_mr_for_entry(stack_name, &gg_id).is_some() {
            continue;
        }

        // Query the provider for PRs on this branch
        // Use the branch name without "origin/" prefix
        let query_branch = match branch_type {
            BranchType::Local => branch_name.to_string(),
            BranchType::Remote => branch_name
                .strip_prefix("origin/")
                .unwrap_or(branch_name)
                .to_string(),
        };

        match provider.list_prs_for_branch(&query_branch) {
            Ok(pr_numbers) => {
                if let Some(&pr_number) = pr_numbers.first() {
                    // Save the mapping
                    config.set_mr_for_entry(stack_name, &gg_id, pr_number);
                    imported_count += 1;
                }
            }
            Err(_) => {
                // If we can't query this branch, just skip it
                // This might happen if the branch doesn't exist remotely yet
                skipped_branches.insert(query_branch);
            }
        }
    }

    if imported_count > 0 {
        // Save config with new mappings
        config.save(git_dir)?;
        println!(
            "{} Imported {} PR mapping(s) for stack {}",
            style("→").cyan(),
            imported_count,
            style(stack_name).cyan()
        );
    }

    Ok(())
}
