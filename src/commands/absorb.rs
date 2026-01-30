//! `gg absorb` - Absorb staged changes into the appropriate commits

use std::process::Command;

use console::style;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack::Stack;

/// Run the absorb command
pub fn run() -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load(repo.path())?;

    // Check if there are staged changes
    let statuses = repo.statuses(None)?;
    let has_staged = statuses.iter().any(|s| {
        let status = s.status();
        status.is_index_new()
            || status.is_index_modified()
            || status.is_index_deleted()
            || status.is_index_renamed()
            || status.is_index_typechange()
    });

    if !has_staged {
        // Check for unstaged changes
        let has_unstaged = statuses.iter().any(|s| {
            let status = s.status();
            status.is_wt_new()
                || status.is_wt_modified()
                || status.is_wt_deleted()
                || status.is_wt_renamed()
                || status.is_wt_typechange()
        });

        if has_unstaged {
            println!(
                "{}",
                style("No staged changes. Stage changes with `git add` first, or use `git add -p` for interactive staging.").dim()
            );
        } else {
            println!("{}", style("No changes to absorb.").dim());
        }
        return Ok(());
    }

    // Load stack to get the base
    let stack = Stack::load(&repo, &config)?;

    if stack.is_empty() {
        return Err(GgError::Other(
            "Stack is empty. Use `git commit` to create commits first.".to_string(),
        ));
    }

    println!("{}", style("Running git-absorb...").dim());

    // Try to run git-absorb
    // First check if it's installed
    let check = Command::new("git")
        .args(["absorb", "--version"])
        .output();

    if check.is_err() || !check.unwrap().status.success() {
        println!(
            "{} git-absorb is not installed.",
            style("Error:").red().bold()
        );
        println!();
        println!("Install it with:");
        println!("  cargo install git-absorb");
        println!();
        println!("Or on macOS:");
        println!("  brew install git-absorb");
        return Err(GgError::Other("git-absorb not installed".to_string()));
    }

    // Run git-absorb with the stack base
    let base_ref = format!("{}^", stack.base);
    let output = Command::new("git")
        .args(["absorb", "--base", &base_ref, "--and-rebase"])
        .output()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stdout.is_empty() {
            for line in stdout.lines() {
                println!("  {}", line);
            }
        }

        println!(
            "{} Changes absorbed into stack",
            style("OK").green().bold()
        );
        println!(
            "{}",
            style("  Run `gg ls` to review and `gg sync` to push changes.").dim()
        );
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        // git-absorb may output useful info even on "failure"
        if !stdout.is_empty() {
            for line in stdout.lines() {
                println!("  {}", line);
            }
        }

        if stderr.contains("Could not find") || stderr.contains("No commit found") {
            println!(
                "{} Could not automatically determine where to absorb changes.",
                style("Warning:").yellow()
            );
            println!("  The changes may be too ambiguous or span multiple commits.");
            println!("  Try using `gg mv <pos>` and `gg sc` to manually squash changes.");
        } else if !stderr.is_empty() {
            for line in stderr.lines() {
                println!("  {}", style(line).dim());
            }
        }

        // git-absorb returns non-zero even when it works but has nothing to do
        if stdout.contains("No changes absorbed") || stderr.contains("Nothing to absorb") {
            println!(
                "{}",
                style("No changes could be absorbed automatically.").dim()
            );
        }
    }

    Ok(())
}
