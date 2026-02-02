//! git-gud (gg) - A stacked-diffs CLI tool for GitHub and GitLab
//!
//! Entry point for the CLI application.

mod commands;
mod config;
mod error;
mod gh;
mod git;
mod glab;
mod provider;
mod stack;

use std::process::exit;

use clap::{Parser, Subcommand};
use console::style;

#[derive(Parser, Debug)]
#[command(
    name = "gg",
    author = "Nacho Lopez",
    version,
    about = "A stacked-diffs CLI tool for GitHub and GitLab",
    long_about = None
)]
struct Cli {
    #[clap(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create a new stack or switch to an existing one
    #[command(name = "co", alias = "sw", alias = "checkout", alias = "switch")]
    Checkout {
        /// Stack name to create or switch to
        stack_name: Option<String>,

        /// Base branch to use (default: main/master/trunk)
        #[arg(short, long)]
        base: Option<String>,
    },

    /// List current stack or all stacks
    #[command(name = "ls", alias = "list")]
    List {
        /// Show all stacks, not just current
        #[arg(short, long)]
        all: bool,

        /// Refresh PR/MR status from remote
        #[arg(short, long)]
        refresh: bool,

        /// List remote stacks (branches on origin not yet checked out locally)
        #[arg(long)]
        remote: bool,
    },

    /// Sync stack with remote (push branches and create/update PRs/MRs)
    #[command(name = "sync", alias = "diff")]
    Sync {
        /// Create new MRs as drafts
        #[arg(short, long)]
        draft: bool,

        /// Force push even if remote is ahead
        #[arg(short, long)]
        force: bool,

        /// Update PR/MR titles and descriptions to match commit messages
        #[arg(long)]
        update_descriptions: bool,
    },

    /// Move to a specific commit in the stack
    #[command(name = "mv", alias = "move")]
    Move {
        /// Position (1-indexed), entry ID, or commit SHA
        target: String,
    },

    /// Move to the first commit in the stack
    #[command(name = "first")]
    First,

    /// Move to the last commit in the stack (stack head)
    #[command(name = "last")]
    Last,

    /// Move to the previous commit in the stack
    #[command(name = "prev", alias = "previous")]
    Prev,

    /// Move to the next commit in the stack
    #[command(name = "next")]
    Next,

    /// Squash staged changes into the current commit
    #[command(name = "sc", alias = "squash")]
    Squash {
        /// Squash all changes (staged and unstaged)
        #[arg(short, long)]
        all: bool,
    },

    /// Reorder commits in the stack
    #[command(name = "reorder")]
    Reorder {
        /// New order as positions (1-indexed) or SHAs, e.g., "3,1,2" or "3 1 2"
        /// Position 1 = bottom of stack (closest to base)
        #[arg(short, long, value_name = "ORDER")]
        order: Option<String>,
    },

    /// Land (merge) approved MRs starting from the first commit
    #[command(name = "land", alias = "merge")]
    Land {
        /// Land all approved MRs in sequence
        #[arg(short, long)]
        all: bool,

        /// Disable squash when merging (default: squash enabled)
        #[arg(long = "no-squash")]
        no_squash: bool,

        /// Wait for CI to pass and approvals before merging
        #[arg(short, long)]
        wait: bool,

        /// Automatically clean up stack after landing all PRs/MRs
        #[arg(short, long, conflicts_with = "no_clean")]
        clean: bool,

        /// Disable automatic cleanup after landing (overrides config default)
        #[arg(long = "no-clean", conflicts_with = "clean")]
        no_clean: bool,
    },

    /// Clean up merged stacks
    #[command(name = "clean")]
    Clean {
        /// Clean all merged stacks without prompting
        #[arg(short, long)]
        all: bool,
    },

    /// Rebase the stack onto the updated base branch
    #[command(name = "rebase")]
    Rebase {
        /// Target branch to rebase onto (default: base branch)
        target: Option<String>,
    },

    /// Continue a paused operation (rebase, etc.)
    #[command(name = "continue")]
    Continue,

    /// Abort a paused operation (rebase, etc.)
    #[command(name = "abort")]
    Abort,

    /// Run lint commands on each commit in the stack
    #[command(name = "lint")]
    Lint {
        /// Stop at this commit position (default: current)
        #[arg(short, long)]
        until: Option<usize>,
    },

    /// Set up git-gud config for this repository
    #[command(name = "setup")]
    Setup,

    /// Absorb staged changes into the appropriate commits
    #[command(name = "absorb")]
    Absorb {
        /// Show what would be done without making changes
        #[arg(short = 'n', long)]
        dry_run: bool,

        /// Automatically rebase after creating fixup commits
        #[arg(short, long)]
        and_rebase: bool,

        /// Absorb whole files rather than individual hunks
        #[arg(short, long)]
        whole_file: bool,

        /// Create at most one fixup per commit
        #[arg(long)]
        one_fixup_per_commit: bool,
    },

    /// Generate shell completions
    #[command(name = "completions")]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        // No command = show stacks (like `gg ls`)
        None => commands::ls::run(false, false, false),

        Some(Commands::Checkout { stack_name, base }) => commands::checkout::run(stack_name, base),
        Some(Commands::List {
            all,
            refresh,
            remote,
        }) => commands::ls::run(all, refresh, remote),
        Some(Commands::Sync {
            draft,
            force,
            update_descriptions,
        }) => commands::sync::run(draft, force, update_descriptions),
        Some(Commands::Move { target }) => commands::nav::move_to(&target),
        Some(Commands::First) => commands::nav::first(),
        Some(Commands::Last) => commands::nav::last(),
        Some(Commands::Prev) => commands::nav::prev(),
        Some(Commands::Next) => commands::nav::next(),
        Some(Commands::Squash { all }) => commands::squash::run(all),
        Some(Commands::Reorder { order }) => {
            commands::reorder::run(commands::reorder::ReorderOptions { order })
        }
        Some(Commands::Land {
            all,
            no_squash,
            wait,
            clean,
            no_clean,
        }) => {
            // Determine auto_clean based on flags and config
            let auto_clean = if clean {
                // --clean explicitly passed
                true
            } else if no_clean {
                // --no-clean explicitly passed
                false
            } else {
                // No explicit flag, use config default
                match git::open_repo().and_then(|repo| config::Config::load(repo.path())) {
                    Ok(cfg) => cfg.get_land_auto_clean(),
                    Err(_) => false, // If we can't load config, default to false
                }
            };

            commands::land::run(all, !no_squash, wait, auto_clean)
        }
        Some(Commands::Clean { all }) => commands::clean::run(all),
        Some(Commands::Rebase { target }) => commands::rebase::run(target),
        Some(Commands::Continue) => commands::rebase::continue_rebase(),
        Some(Commands::Abort) => commands::rebase::abort_rebase(),
        Some(Commands::Lint { until }) => commands::lint::run(until),
        Some(Commands::Setup) => commands::setup::run(),
        Some(Commands::Absorb {
            dry_run,
            and_rebase,
            whole_file,
            one_fixup_per_commit,
        }) => commands::absorb::run(commands::absorb::AbsorbOptions {
            dry_run,
            and_rebase,
            whole_file,
            one_fixup_per_commit,
        }),
        Some(Commands::Completions { shell }) => commands::completions::run(shell),
    };

    if let Err(e) = result {
        eprintln!("{} {}", style("error:").red().bold(), e);
        exit(1);
    }
}
