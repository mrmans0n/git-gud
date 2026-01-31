//! git-gud (gg) - A stacked-diffs CLI tool for GitLab
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
    about = "A stacked-diffs CLI tool for GitLab",
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

        /// Refresh MR status from GitLab
        #[arg(short, long)]
        refresh: bool,
    },

    /// Sync stack with GitLab (push branches and create/update MRs)
    #[command(name = "sync", alias = "diff")]
    Sync {
        /// Create new MRs as drafts
        #[arg(short, long)]
        draft: bool,

        /// Force push even if remote is ahead
        #[arg(short, long)]
        force: bool,
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
    Reorder,

    /// Land (merge) approved MRs starting from the first commit
    #[command(name = "land", alias = "merge")]
    Land {
        /// Land all approved MRs in sequence
        #[arg(short, long)]
        all: bool,

        /// Squash commits when merging
        #[arg(short, long)]
        squash: bool,
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
        None => commands::ls::run(false, false),

        Some(Commands::Checkout { stack_name, base }) => commands::checkout::run(stack_name, base),
        Some(Commands::List { all, refresh }) => commands::ls::run(all, refresh),
        Some(Commands::Sync { draft, force }) => commands::sync::run(draft, force),
        Some(Commands::Move { target }) => commands::nav::move_to(&target),
        Some(Commands::First) => commands::nav::first(),
        Some(Commands::Last) => commands::nav::last(),
        Some(Commands::Prev) => commands::nav::prev(),
        Some(Commands::Next) => commands::nav::next(),
        Some(Commands::Squash { all }) => commands::squash::run(all),
        Some(Commands::Reorder) => commands::reorder::run(),
        Some(Commands::Land { all, squash }) => commands::land::run(all, squash),
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
