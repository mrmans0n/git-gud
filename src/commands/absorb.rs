//! `gg absorb` - Absorb staged changes into the appropriate commits
//!
//! Uses the git-absorb library to automatically determine which commits
//! staged changes should be absorbed into, then creates fixup commits
//! and optionally rebases them.

use console::style;
use slog::{o, Drain, Logger};

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack::Stack;

/// Options for the absorb command
#[derive(Debug, Default)]
pub struct AbsorbOptions {
    /// Show what would be done without making changes
    pub dry_run: bool,
    /// Automatically rebase after creating fixup commits
    pub and_rebase: bool,
    /// Absorb whole files rather than individual hunks
    pub whole_file: bool,
    /// Create at most one fixup per commit
    pub one_fixup_per_commit: bool,
}

/// Run the absorb command
pub fn run(options: AbsorbOptions) -> Result<()> {
    let repo = git::open_repo()?;
    let gg_config = Config::load(repo.path())?;

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
    let stack = Stack::load(&repo, &gg_config)?;

    if stack.is_empty() {
        return Err(GgError::Other(
            "Stack is empty. Use `git commit` to create commits first.".to_string(),
        ));
    }

    // Determine the base reference for absorb
    // We want to absorb into commits between base and HEAD
    let base_ref = stack.base.clone();

    if options.dry_run {
        println!(
            "{} (dry-run mode)",
            style("Analyzing changes to absorb...").dim()
        );
    } else {
        println!("{}", style("Absorbing staged changes...").dim());
    }

    // Create a slog logger for git-absorb
    // Use a quiet logger that only shows errors
    let logger = create_logger(options.dry_run);

    // Configure git-absorb
    let rebase_options: Vec<&str> = Vec::new();
    let absorb_config = git_absorb::Config {
        dry_run: options.dry_run,
        force_author: false,
        force_detach: false,
        base: Some(&base_ref),
        and_rebase: options.and_rebase,
        rebase_options: &rebase_options,
        whole_file: options.whole_file,
        one_fixup_per_commit: options.one_fixup_per_commit,
        message: None,
    };

    // Run git-absorb
    match git_absorb::run(&logger, &absorb_config) {
        Ok(()) => {
            if options.dry_run {
                println!(
                    "{} Dry-run complete. Run without --dry-run to apply changes.",
                    style("OK").green().bold()
                );
            } else {
                println!("{} Changes absorbed into stack", style("OK").green().bold());
                if !options.and_rebase {
                    println!(
                        "{}",
                        style("  Fixup commits created. Run `git rebase -i --autosquash` or use `gg absorb --and-rebase` to automatically rebase.").dim()
                    );
                } else {
                    println!(
                        "{}",
                        style("  Run `gg ls` to review and `gg sync --force` to push changes.")
                            .dim()
                    );
                }
            }
            Ok(())
        }
        Err(e) => {
            let error_msg = e.to_string();

            // Handle common error cases with helpful messages
            if error_msg.contains("could not find")
                || error_msg.contains("no commit found")
                || error_msg.contains("nothing to absorb")
            {
                println!(
                    "{} Could not automatically determine where to absorb changes.",
                    style("Warning:").yellow()
                );
                println!("  The staged changes may not match any existing commit hunks.");
                println!();
                println!("  Suggestions:");
                println!("    • Use `gg mv <pos>` to navigate to a commit, then `gg sc` to squash");
                println!("    • Create a new commit with `git commit`");
                println!("    • Try `gg absorb --whole-file` to match by file instead of hunk");
                Ok(())
            } else if error_msg.contains("uncommitted changes") {
                Err(GgError::Other(
                    "Please stage or stash your changes before running absorb.".to_string(),
                ))
            } else {
                Err(GgError::Other(format!("git-absorb failed: {}", error_msg)))
            }
        }
    }
}

/// Create a slog logger for git-absorb output
fn create_logger(verbose: bool) -> Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    if verbose {
        Logger::root(drain, o!())
    } else {
        // Filter to only show warnings and errors
        let drain = slog::LevelFilter::new(drain, slog::Level::Warning).fuse();
        Logger::root(drain, o!())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_absorb_options_default() {
        let opts = AbsorbOptions::default();
        assert!(!opts.dry_run);
        assert!(!opts.and_rebase);
        assert!(!opts.whole_file);
        assert!(!opts.one_fixup_per_commit);
    }
}
