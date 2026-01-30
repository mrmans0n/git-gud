//! `gg lint` - Run lint commands on each commit in the stack

use std::process::Command;

use console::style;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack::Stack;

/// Run the lint command
pub fn run(until: Option<usize>) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let config = Config::load(git_dir)?;

    // Require clean working directory
    git::require_clean_working_directory(&repo)?;

    // Get lint commands from config
    let lint_commands = &config.defaults.lint;
    if lint_commands.is_empty() {
        println!(
            "{}",
            style("No lint commands configured. Add them to .git/gg/config.json").dim()
        );
        println!();
        println!("Example configuration:");
        println!("  {{");
        println!("    \"defaults\": {{");
        println!("      \"lint\": [\"cargo fmt\", \"cargo clippy -- -D warnings\"]");
        println!("    }}");
        println!("  }}");
        return Ok(());
    }

    // Load stack
    let stack = Stack::load(&repo, &config)?;

    if stack.is_empty() {
        println!("{}", style("Stack is empty. Nothing to lint.").dim());
        return Ok(());
    }

    // Determine the end position
    let end_pos =
        until.unwrap_or_else(|| stack.current_position.map(|p| p + 1).unwrap_or(stack.len()));

    if end_pos > stack.len() {
        return Err(GgError::Other(format!(
            "Position {} is out of range (max: {})",
            end_pos,
            stack.len()
        )));
    }

    println!(
        "{}",
        style(format!(
            "Running lint on commits 1-{} ({} lint commands)",
            end_pos,
            lint_commands.len()
        ))
        .dim()
    );

    // Remember current branch/HEAD
    let original_branch = git::current_branch_name(&repo);
    let original_head = repo.head()?.peel_to_commit()?.id();

    let mut had_changes = false;

    // Process each commit from first to end_pos
    for i in 0..end_pos {
        let entry = &stack.entries[i];

        println!();
        println!(
            "{} Linting [{}] {} {}",
            style("→").cyan(),
            entry.position,
            style(&entry.short_sha).yellow(),
            entry.title
        );

        // Checkout this commit
        let commit = repo.find_commit(entry.oid)?;
        git::checkout_commit(&repo, &commit)?;

        // Run lint commands
        for cmd in lint_commands {
            print!("  Running: {} ... ", style(cmd).dim());

            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            let output = Command::new(parts[0]).args(&parts[1..]).output()?;

            if output.status.success() {
                println!("{}", style("OK").green());
            } else {
                println!("{}", style("FAILED").red());
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.is_empty() {
                    for line in stderr.lines().take(5) {
                        println!("    {}", style(line).dim());
                    }
                }
            }
        }

        // Check if lint made changes
        if !git::is_working_directory_clean(&repo)? {
            println!("  {} Lint made changes, squashing...", style("!").yellow());

            // Stage all changes
            let status = Command::new("git").args(["add", "-A"]).status()?;

            if !status.success() {
                return Err(GgError::Other("Failed to stage changes".to_string()));
            }

            // Amend the commit
            let status = Command::new("git")
                .args(["commit", "--amend", "--no-edit"])
                .status()?;

            if !status.success() {
                return Err(GgError::Other("Failed to amend commit".to_string()));
            }

            had_changes = true;
            println!("  {} Changes squashed", style("OK").green());
        }
    }

    // Return to original position
    println!();
    if let Some(branch) = original_branch {
        if had_changes {
            // If we made changes, we need to rebase the remaining commits
            // This is complex - for now, stay at the last linted commit
            println!(
                "{}",
                style("Lint made changes. Review with `gg ls` and sync with `gg sync`.").dim()
            );
        } else {
            git::checkout_branch(&repo, &branch)?;
        }
    } else {
        // Return to original detached HEAD if no changes
        if !had_changes {
            let commit = repo.find_commit(original_head)?;
            git::checkout_commit(&repo, &commit)?;
        }
    }

    println!("{} Linted {} commits", style("OK").green().bold(), end_pos);

    Ok(())
}
