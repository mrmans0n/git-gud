//! `gg lint` - Run lint commands on each commit in the stack

use std::process::Command;

use console::style;
use git2::Oid;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack::{Stack, StackEntry};

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

    // Run lint with cleanup on error
    let result = run_lint_on_commits(&repo, stack, lint_commands, end_pos);

    // Try to restore original position on error, but NOT if there's a rebase in progress
    // (user needs to resolve conflicts in place)
    if result.is_err() && !git::is_rebase_in_progress(&repo) {
        restore_original_position(&repo, original_branch.as_deref(), original_head);
    }

    result
}

/// Run lint commands on commits, returning the result
fn run_lint_on_commits(
    repo: &git2::Repository,
    stack: Stack,
    lint_commands: &[String],
    end_pos: usize,
) -> Result<()> {
    let original_branch = git::current_branch_name(repo);
    let original_head = repo.head()?.peel_to_commit()?.id();
    let mut had_changes = false;
    let base_branch = stack.base.clone();
    let stack_branch = stack.branch_name();
    let mut entries = stack.entries.clone();

    // Process each commit from first to end_pos
    let mut i = 0;
    while i < end_pos {
        let entry = entries[i].clone();

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
        git::checkout_commit(repo, &commit)?;

        // Run lint commands
        for cmd in lint_commands {
            print!("  Running: {} ... ", style(cmd).dim());

            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            let output = match Command::new(parts[0]).args(&parts[1..]).output() {
                Ok(output) => output,
                Err(e) => {
                    println!("{}", style("ERROR").red().bold());
                    let error_msg = if e.kind() == std::io::ErrorKind::NotFound {
                        format!(
                            "Command '{}' not found. Make sure it's installed and in your PATH.\n\
                             Note: Shell aliases don't work here. Use the full command (e.g., './gradlew' instead of 'gw').",
                            parts[0]
                        )
                    } else {
                        format!("Failed to run '{}': {}", parts[0], e)
                    };
                    return Err(GgError::Other(error_msg));
                }
            };

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
        if !git::is_working_directory_clean(repo)? {
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

            // Rebase remaining commits in the lint range onto the amended commit
            if i + 1 < end_pos {
                let new_commit_oid = repo.head()?.peel_to_commit()?.id();
                let old_tip_oid = entries[end_pos - 1].oid;

                let new_commit = new_commit_oid.to_string();
                let old_commit = entry.oid.to_string();
                let old_tip = old_tip_oid.to_string();

                let target_branch = original_branch.as_deref().unwrap_or(stack_branch.as_str());

                // Ensure rebase is performed on the stack branch so it updates the branch
                // and avoids leaving the user in detached HEAD after conflicts. We force the
                // branch to the old tip so we only rebase the intended range.
                git::run_git_command(&["branch", "-f", target_branch, &old_tip])?;
                git::checkout_branch(repo, target_branch)?;

                if let Err(e) = git::run_git_command(&[
                    "rebase",
                    "--onto",
                    &new_commit,
                    &old_commit,
                    target_branch,
                ]) {
                    // Check if this is a rebase conflict
                    if git::is_rebase_in_progress(repo) {
                        print_rebase_conflict_help();
                        return Err(GgError::Other(
                            "Rebase conflict occurred. Resolve conflicts and run `gg continue`."
                                .to_string(),
                        ));
                    }
                    return Err(e);
                }

                entries = refresh_stack_entries(repo, &base_branch, None)?;
            }
        }

        i += 1;
    }

    // Return to original position
    println!();
    if let Some(branch) = original_branch {
        if had_changes {
            // Move the stack branch to the current HEAD (last linted commit)
            // and checkout the branch to avoid leaving user in detached HEAD
            git::move_branch_to_head(repo, &branch)?;
            git::checkout_branch(repo, &branch)?;

            if end_pos < stack.len() {
                // Partial lint: remaining commits need rebasing
                println!(
                    "{}",
                    style("Lint made changes. Run `gg rebase` to update remaining commits, then `gg sync`.").dim()
                );
            } else {
                println!(
                    "{}",
                    style("Lint made changes. Review with `gg ls` and sync with `gg sync`.").dim()
                );
            }
        } else {
            git::checkout_branch(repo, &branch)?;
        }
    } else {
        // Return to original detached HEAD if no changes
        if !had_changes {
            let commit = repo.find_commit(original_head)?;
            git::checkout_commit(repo, &commit)?;
        }
    }

    println!("{} Linted {} commits", style("OK").green().bold(), end_pos);

    Ok(())
}

fn refresh_stack_entries(
    repo: &git2::Repository,
    base_branch: &str,
    stack_branch: Option<&str>,
) -> Result<Vec<StackEntry>> {
    let oids = git::get_stack_commit_oids(repo, base_branch, stack_branch)?;

    let mut entries = Vec::with_capacity(oids.len());
    for (i, oid) in oids.iter().enumerate() {
        let commit = repo.find_commit(*oid)?;
        entries.push(StackEntry::from_commit(&commit, i + 1));
    }

    Ok(entries)
}

/// Restore the original branch/HEAD position
fn restore_original_position(
    repo: &git2::Repository,
    original_branch: Option<&str>,
    original_head: Oid,
) {
    println!();
    println!("{} Restoring original position...", style("→").cyan());

    let restored = if let Some(branch) = original_branch {
        git::checkout_branch(repo, branch).is_ok()
    } else if let Ok(commit) = repo.find_commit(original_head) {
        git::checkout_commit(repo, &commit).is_ok()
    } else {
        false
    };

    if restored {
        println!("{} Restored to original position", style("OK").green());
    } else {
        println!(
            "{} Could not restore original position. You may be in detached HEAD.",
            style("Warning:").yellow()
        );
    }
}

/// Get list of files with conflicts
fn get_conflicted_files() -> Vec<String> {
    let output = Command::new("git")
        .args(["diff", "--name-only", "--diff-filter=U"])
        .output();

    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Print helpful message when rebase conflict occurs during lint
fn print_rebase_conflict_help() {
    println!();
    println!(
        "{} Rebase conflict while rebasing after lint changes",
        style("⚠️").yellow()
    );
    println!();

    let conflicted_files = get_conflicted_files();
    if !conflicted_files.is_empty() {
        println!("The following files have conflicts:");
        for file in &conflicted_files {
            println!("  {} {}", style("-").dim(), file);
        }
        println!();
    }

    println!("To resolve:");
    println!("  1. Edit the conflicting files to resolve conflicts");
    println!("  2. {}", style("git add <resolved-files>").cyan());
    println!("  3. {}", style("gg continue").cyan());
    println!();
    println!("To abort and undo lint changes:");
    println!("  {}", style("gg abort").cyan());
}
