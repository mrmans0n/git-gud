//! `gg rebase` - Rebase the stack onto an updated base branch

use console::style;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack::Stack;

/// Run the rebase command
pub fn run(target: Option<String>) -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load(repo.path())?;

    // Require clean working directory
    git::require_clean_working_directory(&repo)?;

    // Load stack
    let stack = Stack::load(&repo, &config)?;

    // Determine target branch
    let target_branch = target.unwrap_or_else(|| stack.base.clone());

    println!(
        "{}",
        style(format!("Rebasing stack onto {}...", target_branch)).dim()
    );

    // Fetch the latest from remote first
    let fetch_result = git::run_git_command(&["fetch", "origin", &target_branch]);
    if let Err(e) = fetch_result {
        println!(
            "{} Could not fetch {}: {}",
            style("Warning:").yellow(),
            target_branch,
            e
        );
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
            Ok(())
        }
        Err(e) => {
            let error_str = e.to_string();
            if error_str.contains("CONFLICT") || error_str.contains("conflict") {
                println!(
                    "{} Rebase conflict detected.",
                    style("!").yellow().bold()
                );
                println!("  Resolve conflicts, then run `gg continue`");
                println!("  Or run `gg abort` to cancel the rebase");
                Err(GgError::RebaseConflict)
            } else {
                Err(e)
            }
        }
    }
}

/// Continue a paused rebase
pub fn continue_rebase() -> Result<()> {
    let repo = git::open_repo()?;

    if !git::is_rebase_in_progress(&repo) {
        return Err(GgError::NoRebaseInProgress);
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

    println!(
        "{} Rebase aborted",
        style("OK").green().bold()
    );

    Ok(())
}
