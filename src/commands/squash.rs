//! `gg sc` / `gg squash` - Squash changes into the current commit

use std::process::Command;

use console::style;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack;
use crate::stack::Stack;

/// Run the squash command
pub fn run(all: bool) -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "squash")?;

    let config = Config::load(repo.commondir())?;

    // Verify we're on a stack
    let stack = Stack::load(&repo, &config)?;

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

    // Check if we're not at stack head (current_position is Some and not last)
    let needs_rebase = stack
        .current_position
        .map(|p| p < stack.len() - 1)
        .unwrap_or(false);

    // If we need to rebase after amend, ensure there are no UNSTAGED changes
    // Staged changes are fine (they'll be committed), but unstaged changes
    // would be lost during the rebase
    if needs_rebase {
        let statuses = repo.statuses(None)?;
        let has_unstaged = statuses.iter().any(|s| {
            let flags = s.status();
            // Check for unstaged changes (WT_* flags)
            flags.is_wt_modified()
                || flags.is_wt_deleted()
                || flags.is_wt_renamed()
                || flags.is_wt_typechange()
        });

        if has_unstaged {
            return Err(GgError::Other(
                "Unstaged changes detected. Please stage or stash them before squashing."
                    .to_string(),
            ));
        }
    }

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
            let stdout = String::from_utf8_lossy(&rebase_result.stdout);

            if stderr.contains("CONFLICT")
                || stderr.contains("conflict")
                || stdout.contains("CONFLICT")
                || stdout.contains("conflict")
            {
                eprintln!("{}", style("Rebase conflict detected.").yellow().bold());
                eprintln!(
                    "  Resolve conflicts, stage the changes with `git add`, then run `gg continue`"
                );
                eprintln!("  Or run `gg abort` to cancel the rebase");
                return Err(GgError::RebaseConflict);
            }

            let error_msg = if !stderr.is_empty() {
                stderr.to_string()
            } else if !stdout.is_empty() {
                stdout.to_string()
            } else {
                "Unknown error (no output from git)".to_string()
            };
            return Err(GgError::Other(format!("Rebase failed: {}", error_msg)));
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
            // Update nav context to reflect our new position after rebase
            // entry.position is 1-indexed, so we need to subtract 1 for 0-indexed storage
            let git_dir = repo.path();
            stack::save_nav_context(git_dir, &branch_name, entry.position - 1, entry.oid)?;
        }
    }

    Ok(())
}
