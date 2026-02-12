//! Navigation commands: `gg mv`, `gg first`, `gg last`, `gg prev`, `gg next`

use console::style;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack::{self, Stack, StackEntry};

/// Move to a specific position, entry ID, or SHA
pub fn move_to(target: &str) -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "nav")?;

    if git::is_rebase_in_progress(&repo) {
        return Err(GgError::Other(
            "A rebase is in progress. Run `gg continue` to continue or `gg abort` to cancel."
                .to_string(),
        ));
    }

    let config = Config::load(repo.commondir())?;
    let stack = Stack::load(&repo, &config)?;

    if stack.is_empty() {
        return Err(GgError::Other("Stack is empty".to_string()));
    }

    // Try to parse target as position (1-indexed number)
    if let Ok(pos) = target.parse::<usize>() {
        if let Some(entry) = stack.get_entry_by_position(pos) {
            return checkout_entry(&repo, &stack, entry);
        } else {
            return Err(GgError::Other(format!(
                "Position {} is out of range (1-{})",
                pos,
                stack.len()
            )));
        }
    }

    // Try to find by GG-ID
    if let Some(entry) = stack.get_entry_by_gg_id(target) {
        return checkout_entry(&repo, &stack, entry);
    }

    // Try to find by SHA prefix
    for entry in &stack.entries {
        if entry.short_sha.starts_with(target) || entry.oid.to_string().starts_with(target) {
            return checkout_entry(&repo, &stack, entry);
        }
    }

    Err(GgError::Other(format!(
        "Could not find commit matching '{}' in stack",
        target
    )))
}

/// Move to the first commit in the stack
pub fn first() -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "nav")?;

    if git::is_rebase_in_progress(&repo) {
        return Err(GgError::Other(
            "A rebase is in progress. Run `gg continue` to continue or `gg abort` to cancel."
                .to_string(),
        ));
    }

    let config = Config::load(repo.commondir())?;
    let stack = Stack::load(&repo, &config)?;

    if let Some(entry) = stack.first() {
        checkout_entry(&repo, &stack, entry)
    } else {
        Err(GgError::Other("Stack is empty".to_string()))
    }
}

/// Move to the last commit (stack head)
pub fn last() -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "nav")?;

    let config = Config::load(repo.commondir())?;
    let stack = Stack::load(&repo, &config)?;

    // Check if a rebase is in progress
    if git::is_rebase_in_progress(&repo) {
        return Err(GgError::Other(
            "A rebase is in progress. Run `gg continue` to continue or `gg abort` to cancel."
                .to_string(),
        ));
    }

    if let Some(entry) = stack.last() {
        // Check if we're in detached HEAD and if the current commit has changed
        let needs_rebase = check_and_rebase_if_modified(&repo, &stack)?;

        // For last, we should checkout the branch, not detach
        git::checkout_branch(&repo, &stack.branch_name())?;
        // Clear the saved stack since we're back on the branch (per-worktree state)
        stack::clear_current_stack(repo.path())?;

        if needs_rebase {
            println!(
                "{} Moved to stack head (rebased after modifications)",
                style("OK").green().bold()
            );
        } else {
            println!(
                "{} Moved to stack head: [{}] {} {}",
                style("OK").green().bold(),
                entry.position,
                style(&entry.short_sha).yellow(),
                entry.title
            );
        }
        Ok(())
    } else {
        Err(GgError::Other("Stack is empty".to_string()))
    }
}

/// Move to the previous commit
pub fn prev() -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "nav")?;

    if git::is_rebase_in_progress(&repo) {
        return Err(GgError::Other(
            "A rebase is in progress. Run `gg continue` to continue or `gg abort` to cancel."
                .to_string(),
        ));
    }

    let config = Config::load(repo.commondir())?;
    let stack = Stack::load(&repo, &config)?;

    if let Some(entry) = stack.prev() {
        checkout_entry(&repo, &stack, entry)
    } else {
        Err(GgError::Other(
            "Already at the first commit in the stack".to_string(),
        ))
    }
}

/// Move to the next commit
pub fn next() -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "nav")?;

    let config = Config::load(repo.commondir())?;
    let stack = Stack::load(&repo, &config)?;

    // Check if a rebase is in progress
    if git::is_rebase_in_progress(&repo) {
        return Err(GgError::Other(
            "A rebase is in progress. Run `gg continue` to continue or `gg abort` to cancel."
                .to_string(),
        ));
    }

    // Check if we need to rebase due to modifications
    let needs_rebase = check_and_rebase_if_modified(&repo, &stack)?;

    // If we're at the last commit, we might just need to checkout the branch
    let current_pos = stack
        .current_position
        .unwrap_or(stack.len().saturating_sub(1));

    if current_pos >= stack.len().saturating_sub(1) {
        // At stack head, ensure we're on the branch
        git::checkout_branch(&repo, &stack.branch_name())?;
        stack::clear_current_stack(repo.path())?;
        if needs_rebase {
            println!(
                "{} Already at stack head (rebased)",
                style("OK").green().bold()
            );
        } else {
            println!("{} Already at stack head", style("OK").green().bold());
        }
        return Ok(());
    }

    // Reload stack after potential rebase
    let stack = if needs_rebase {
        Stack::load(&repo, &config)?
    } else {
        stack
    };

    if let Some(entry) = stack.next() {
        // If next is the last entry, checkout branch instead of detaching
        if entry.position == stack.len() {
            git::checkout_branch(&repo, &stack.branch_name())?;
            stack::clear_current_stack(repo.path())?;
            println!(
                "{} Moved to stack head: [{}] {} {}",
                style("OK").green().bold(),
                entry.position,
                style(&entry.short_sha).yellow(),
                entry.title
            );
            Ok(())
        } else {
            checkout_entry(&repo, &stack, entry)
        }
    } else {
        Err(GgError::Other(
            "Already at the last commit in the stack".to_string(),
        ))
    }
}

/// Checkout a specific entry (detached HEAD)
fn checkout_entry(repo: &git2::Repository, stack: &Stack, entry: &StackEntry) -> Result<()> {
    // Save the stack branch and navigation context for later use in detached HEAD mode
    stack::save_nav_context(
        repo.path(),
        &stack.branch_name(),
        entry.position - 1,
        entry.oid,
    )?;

    let commit = repo.find_commit(entry.oid)?;
    git::checkout_commit(repo, &commit)?;

    println!(
        "{} Moved to: [{}] {} {}",
        style("OK").green().bold(),
        entry.position,
        style(&entry.short_sha).yellow(),
        entry.title
    );

    // Show hint about returning to stack head
    if entry.position < stack.len() {
        println!(
            "{}",
            style("  Use `gg last` to return to stack head, or `gg next` to move forward.").dim()
        );
    }

    // Show warning about detached HEAD
    println!(
        "{}",
        style("  Note: HEAD is detached. Use `gg sc` to squash changes into this commit.").dim()
    );

    Ok(())
}

/// Check if the current HEAD has been modified from the original commit in the stack
/// If modified and there are commits after this one, rebase them onto the new HEAD
/// Returns true if a rebase was performed
fn check_and_rebase_if_modified(repo: &git2::Repository, stack: &Stack) -> Result<bool> {
    use std::process::Command;

    // Don't try to rebase if a rebase is already in progress
    if git::is_rebase_in_progress(repo) {
        return Ok(false);
    }

    // Try to read saved navigation context (branch, position, original_oid)
    let git_dir = repo.path();
    let nav_context = match stack::read_nav_context(git_dir) {
        Some(ctx) => ctx,
        None => return Ok(false), // No saved context, nothing to check
    };

    let (_saved_branch, saved_position, original_oid) = nav_context;

    // Check if we're in the middle of the stack (have commits after us)
    if saved_position >= stack.len() - 1 {
        // At or past stack head, no need to rebase - just clear nav context
        stack::clear_current_stack(git_dir)?;
        return Ok(false);
    }

    // Get the current HEAD commit
    let current_head = repo.head()?.peel_to_commit()?;
    let current_oid = current_head.id();

    // Check if we're still at a position that matches our saved context
    // If current HEAD matches ANY position in the stack, and it's not the saved position,
    // then we've already moved (e.g., via a previous rebase) - clear stale nav context
    if let Some(current_stack_pos) = stack.current_position {
        if current_stack_pos != saved_position {
            // We've moved to a different position - nav context is stale, clear it
            stack::clear_current_stack(repo.path())?;
            return Ok(false);
        }
    }

    // If they're the same, no modification occurred
    if current_oid == original_oid {
        return Ok(false);
    }

    // HEAD has been modified! We need to rebase subsequent commits
    println!(
        "{}",
        style(format!(
            "Detected modification at position {}. Rebasing {} subsequent commits...",
            saved_position + 1,
            stack.len() - saved_position - 1
        ))
        .yellow()
    );

    // Get the branch name to rebase
    let branch_name = stack.branch_name();

    // Use git rebase --onto to rebase the remaining commits
    // git rebase --onto <new_base> <old_base> <branch>
    let rebase_result = Command::new("git")
        .args([
            "rebase",
            "--onto",
            &current_oid.to_string(),
            &original_oid.to_string(),
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
        return Err(GgError::Other(format!(
            "Failed to rebase stack: {}",
            error_msg
        )));
    }

    println!(
        "{} Successfully rebased stack onto modified commit",
        style("OK").green().bold()
    );

    Ok(true)
}
