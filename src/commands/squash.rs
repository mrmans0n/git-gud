//! `gg sc` / `gg squash` - Squash changes into the current commit

use std::process::Command;

use console::style;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack::Stack;

/// Run the squash command
pub fn run(all: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load(repo.path())?;

    // Check if we have changes to squash
    let statuses = repo.statuses(None)?;
    if statuses.is_empty() {
        println!("{}", style("No changes to squash.").dim());
        return Ok(());
    }

    // Get current HEAD commit
    let head = repo.head()?.peel_to_commit()?;
    let head_sha = git::short_sha(&head);
    let head_title = git::get_commit_title(&head);

    // Try to load stack to determine if we need to rebase
    let stack_result = Stack::load(&repo, &config);
    let needs_rebase = if let Ok(ref stack) = stack_result {
        // Check if we're not at stack head (current_position is Some and not last)
        stack
            .current_position
            .map(|p| p < stack.len() - 1)
            .unwrap_or(false)
    } else {
        false
    };

    // Perform the squash using git command (more reliable for amend)
    let mut args = vec!["commit", "--amend", "--no-edit"];
    if all {
        args.push("--all");
    }

    let output = Command::new("git").args(&args).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to amend commit: {}",
            stderr
        )));
    }

    println!(
        "{} Squashed into {} {}",
        style("OK").green().bold(),
        style(&head_sha).yellow(),
        head_title
    );

    // If we need to rebase remaining commits
    if needs_rebase {
        // Check for clean working directory before rebasing
        git::require_clean_working_directory(&repo)?;

        let stack = stack_result.unwrap();
        let remaining = stack.len() - stack.current_position.unwrap() - 1;

        println!(
            "{}",
            style(format!("Rebasing {} commits on top...", remaining)).dim()
        );

        // We need to rebase the remaining commits
        // The tricky part is that we're in detached HEAD state
        // We need to:
        // 1. Remember the stack branch
        // 2. Rebase from current position to stack head onto the new HEAD

        // Get the new HEAD after amend
        let new_head = repo.head()?.peel_to_commit()?;

        // Get the stack branch name
        let branch_name = stack.branch_name();

        // Use git rebase to rebase remaining commits
        // git rebase --onto <new_head> <old_head> <branch>
        let rebase_result = Command::new("git")
            .args([
                "rebase",
                "--onto",
                &new_head.id().to_string(),
                &head.id().to_string(),
                &branch_name,
            ])
            .output()?;

        if !rebase_result.status.success() {
            let stderr = String::from_utf8_lossy(&rebase_result.stderr);
            if stderr.contains("CONFLICT") || stderr.contains("conflict") {
                return Err(GgError::RebaseConflict);
            }
            return Err(GgError::Other(format!("Rebase failed: {}", stderr)));
        }

        println!(
            "{} Rebased {} commits on top",
            style("OK").green().bold(),
            remaining
        );

        // Stay at the same position (now pointing to amended commit)
        // Find our position in the new stack
        let new_stack = Stack::load(&repo, &config)?;
        let our_pos = stack.current_position.unwrap();

        if let Some(entry) = new_stack.get_entry_by_position(our_pos + 1) {
            let commit = repo.find_commit(entry.oid)?;
            git::checkout_commit(&repo, &commit)?;
        }
    }

    Ok(())
}
