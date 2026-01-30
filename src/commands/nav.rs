//! Navigation commands: `gg mv`, `gg first`, `gg last`, `gg prev`, `gg next`

use console::style;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack::{self, Stack, StackEntry};

/// Move to a specific position, entry ID, or SHA
pub fn move_to(target: &str) -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load(repo.path())?;
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
    let config = Config::load(repo.path())?;
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
    let git_dir = repo.path();
    let config = Config::load(git_dir)?;
    let stack = Stack::load(&repo, &config)?;

    if let Some(entry) = stack.last() {
        // For last, we should checkout the branch, not detach
        git::checkout_branch(&repo, &stack.branch_name())?;
        // Clear the saved stack since we're back on the branch
        stack::clear_current_stack(git_dir)?;
        println!(
            "{} Moved to stack head: [{}] {} {}",
            style("OK").green().bold(),
            entry.position,
            style(&entry.short_sha).yellow(),
            entry.title
        );
        Ok(())
    } else {
        Err(GgError::Other("Stack is empty".to_string()))
    }
}

/// Move to the previous commit
pub fn prev() -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load(repo.path())?;
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
    let git_dir = repo.path();
    let config = Config::load(git_dir)?;
    let stack = Stack::load(&repo, &config)?;

    // If we're at the last commit, we might just need to checkout the branch
    let current_pos = stack.current_position.unwrap_or(stack.len().saturating_sub(1));

    if current_pos >= stack.len().saturating_sub(1) {
        // At stack head, ensure we're on the branch
        git::checkout_branch(&repo, &stack.branch_name())?;
        stack::clear_current_stack(git_dir)?;
        println!(
            "{} Already at stack head",
            style("OK").green().bold()
        );
        return Ok(());
    }

    if let Some(entry) = stack.next() {
        // If next is the last entry, checkout branch instead of detaching
        if entry.position == stack.len() {
            git::checkout_branch(&repo, &stack.branch_name())?;
            stack::clear_current_stack(git_dir)?;
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
fn checkout_entry(
    repo: &git2::Repository,
    stack: &Stack,
    entry: &StackEntry,
) -> Result<()> {
    // Save the stack branch for later use in detached HEAD mode
    stack::save_current_stack(repo.path(), &stack.branch_name())?;

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
