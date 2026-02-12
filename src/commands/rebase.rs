//! `gg rebase` - Rebase the stack onto an updated base branch

use console::style;
use git2::Repository;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack::Stack;

/// Run the rebase command
pub fn run(target: Option<String>) -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "rebase")?;

    run_with_repo(&repo, target)
}

/// Run rebase with an already-open repository (no lock acquisition)
pub fn run_with_repo(repo: &Repository, target: Option<String>) -> Result<()> {
    let config = Config::load(repo.commondir())?;

    // Auto-stash uncommitted changes if present
    let needs_stash = !git::is_working_directory_clean(repo)?;
    if needs_stash {
        println!("{}", style("Auto-stashing uncommitted changes...").dim());
        git::run_git_command(&["stash", "push", "-m", "gg-rebase-autostash"])?;
    }

    // Determine target branch
    // If no target provided, we need to be on a stack to get the base branch
    let target_branch = if let Some(t) = target {
        t
    } else {
        // No target provided, must be on a stack
        let stack = Stack::load(repo, &config)?;
        stack.base.clone()
    };

    // Remember current branch to return to after updating base
    let current_branch = git::current_branch_name(repo);

    println!(
        "{}",
        style(format!("Updating {} and rebasing stack...", target_branch)).dim()
    );

    // Fetch the latest from remote first
    let fetch_result = git::run_git_command(&["fetch", "origin", "--prune"]);
    if let Err(e) = fetch_result {
        println!(
            "{} Could not fetch from origin: {}",
            style("Warning:").yellow(),
            e
        );
    }

    // Update local base branch to match remote (fast-forward)
    // This ensures merged PRs are reflected in the local base
    let update_result = update_local_branch(&target_branch);
    if let Err(e) = update_result {
        println!(
            "{} Could not update local {}: {}",
            style("Warning:").yellow(),
            target_branch,
            e
        );
        println!("  Continuing with rebase onto origin/{}...", target_branch);
    } else {
        println!(
            "{} Updated local {} to latest",
            style("→").cyan(),
            target_branch
        );
    }

    // Return to stack branch if we switched away
    if let Some(ref branch) = current_branch {
        let _ = git::run_git_command(&["checkout", branch]);
    }

    // Perform the rebase
    let rebase_target = format!("origin/{}", target_branch);
    let rebase_result = git::run_git_command(&["rebase", &rebase_target]);

    match rebase_result {
        Ok(_) => {
            println!(
                "{} Rebased stack onto {}",
                style("OK").green().bold(),
                target_branch
            );

            // Restore stashed changes if we stashed earlier
            if needs_stash {
                println!("{}", style("Restoring stashed changes...").dim());
                match git::run_git_command(&["stash", "pop"]) {
                    Ok(_) => {
                        println!("{} Changes restored", style("→").cyan());
                    }
                    Err(e) => {
                        println!(
                            "{} Could not restore stashed changes: {}",
                            style("Warning:").yellow(),
                            e
                        );
                        println!("  Your changes are in the stash. Run 'git stash pop' manually.");
                    }
                }
            }

            Ok(())
        }
        Err(e) => {
            let error_str = e.to_string();
            if error_str.contains("CONFLICT") || error_str.contains("conflict") {
                println!("{} Rebase conflict detected.", style("!").yellow().bold());
                println!("  Resolve conflicts, then run `gg continue`");
                println!("  Or run `gg abort` to cancel the rebase");

                if needs_stash {
                    println!(
                        "  {}",
                        style("Note: Your uncommitted changes are stashed. They will be restored after the rebase completes.").dim()
                    );
                }

                Err(GgError::RebaseConflict)
            } else {
                // On other errors, try to restore stash
                if needs_stash {
                    println!(
                        "{}",
                        style("Attempting to restore stashed changes...").dim()
                    );
                    let _ = git::run_git_command(&["stash", "pop"]);
                }
                Err(e)
            }
        }
    }
}

/// Update a local branch to match its remote counterpart (fast-forward only)
fn update_local_branch(branch: &str) -> Result<()> {
    // Check if the local branch exists
    let local_exists = git::run_git_command(&["rev-parse", "--verify", branch]).is_ok();

    if !local_exists {
        // Branch doesn't exist locally, nothing to update
        return Ok(());
    }

    // Check if remote branch exists
    let remote_ref = format!("origin/{}", branch);
    if git::run_git_command(&["rev-parse", "--verify", &remote_ref]).is_err() {
        // Remote branch doesn't exist
        return Ok(());
    }

    // Get current branch so we can return to it
    let current = git::run_git_command(&["rev-parse", "--abbrev-ref", "HEAD"])?;

    // Switch to the target branch, update it, then switch back
    git::run_git_command(&["checkout", branch])?;

    // Try to fast-forward. If it fails (diverged), that's okay - we'll just use origin/
    let ff_result = git::run_git_command(&["merge", "--ff-only", &remote_ref]);

    // Switch back to original branch
    let _ = git::run_git_command(&["checkout", &current]);

    ff_result.map(|_| ())
}

/// Continue a paused rebase
pub fn continue_rebase() -> Result<()> {
    let repo = git::open_repo()?;

    if !git::is_rebase_in_progress(&repo) {
        return Err(GgError::NoRebaseInProgress);
    }

    // Check for unstaged changes before continuing
    let statuses = repo.statuses(None)?;
    let has_unstaged = statuses.iter().any(|s| {
        let flags = s.status();
        // Check for modified/deleted files that aren't staged
        flags.is_wt_modified() || flags.is_wt_deleted()
    });

    if has_unstaged {
        return Err(GgError::Other(
            "You have unstaged changes. Stage them with `git add` before running `gg continue`."
                .to_string(),
        ));
    }

    // Check for unresolved conflicts
    let has_conflicts = statuses.iter().any(|s| {
        let flags = s.status();
        flags.is_conflicted()
    });

    if has_conflicts {
        return Err(GgError::Other(
            "You have unresolved conflicts. Resolve them and stage with `git add` before running `gg continue`.".to_string()
        ));
    }

    match git::rebase_continue() {
        Ok(_) => {
            println!(
                "{} Rebase continued successfully",
                style("OK").green().bold()
            );
            Ok(())
        }
        Err(e) => {
            let error_str = e.to_string();
            if error_str.contains("CONFLICT") || error_str.contains("conflict") {
                println!(
                    "{} More conflicts detected. Resolve and run `gg continue` again.",
                    style("!").yellow().bold()
                );
                Err(GgError::RebaseConflict)
            } else {
                // Provide more helpful error message
                eprintln!("{} Failed to continue rebase", style("Error:").red().bold());
                eprintln!("  {}", error_str);
                eprintln!();
                eprintln!("{}", style("You are still in rebase state.").yellow());
                eprintln!("  • Resolve any remaining issues");
                eprintln!("  • Run `git rebase --continue` manually to continue");
                eprintln!("  • Or run `gg abort` to cancel the rebase");
                eprintln!();
                eprintln!("  Hint: Run `git status` to see the current state");
                Err(e)
            }
        }
    }
}

/// Abort a paused rebase
pub fn abort_rebase() -> Result<()> {
    let repo = git::open_repo()?;

    if !git::is_rebase_in_progress(&repo) {
        return Err(GgError::NoRebaseInProgress);
    }

    git::rebase_abort()?;

    println!("{} Rebase aborted", style("OK").green().bold());

    Ok(())
}
