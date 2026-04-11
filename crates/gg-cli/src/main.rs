//! git-gud (gg) - A stacked-diffs CLI tool for GitHub and GitLab
//!
//! Entry point for the CLI application.

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

        /// Create or reuse a git worktree for this stack
        #[arg(long = "worktree", short = 'w', alias = "wt")]
        worktree: bool,
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

        /// Output structured JSON
        #[arg(long)]
        json: bool,
    },

    /// Sync stack with remote (push branches and create/update PRs/MRs)
    #[command(name = "sync", alias = "diff")]
    Sync {
        /// Create new PRs/MRs as drafts
        #[arg(short, long)]
        draft: bool,

        /// Output structured JSON
        #[arg(long)]
        json: bool,

        /// Skip checking whether base is behind origin/<base>
        #[arg(long)]
        no_rebase_check: bool,

        /// Force push even if remote is ahead
        #[arg(short, long)]
        force: bool,

        /// Update PR/MR titles and descriptions to match commit messages
        #[arg(long)]
        update_descriptions: bool,

        /// Run lint before sync
        #[arg(short, long, conflicts_with = "no_lint")]
        lint: bool,

        /// Disable lint before sync (overrides config default)
        #[arg(long = "no-lint", conflicts_with = "lint")]
        no_lint: bool,

        /// Sync only up to this commit (position, GG-ID, or SHA)
        #[arg(short, long)]
        until: Option<String>,
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
    #[command(name = "sc", aliases = ["squash", "amend"])]
    Squash {
        /// Squash all changes (staged and unstaged)
        #[arg(short, long)]
        all: bool,
    },

    /// Drop (remove) commits from the stack
    #[command(name = "drop", aliases = ["abandon"])]
    Drop {
        /// Commits to drop: position (1-indexed), short SHA, or GG-ID
        targets: Vec<String>,
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Reorder commits in the stack
    #[command(name = "reorder")]
    Reorder {
        /// New order as positions (1-indexed) or SHAs, e.g., "3,1,2" or "3 1 2"
        /// Position 1 = bottom of stack (closest to base)
        #[arg(short, long, value_name = "ORDER")]
        order: Option<String>,

        /// Disable TUI, use text editor instead
        #[arg(long)]
        no_tui: bool,
    },

    /// Split a commit into two
    #[command(name = "split")]
    Split {
        /// Target commit: position (1-indexed), short SHA, or GG-ID
        #[arg(short, long, value_name = "TARGET")]
        commit: Option<String>,

        /// Message for the new (first) commit
        #[arg(short, long, value_name = "MESSAGE")]
        message: Option<String>,

        /// Don't prompt for the remainder commit message
        #[arg(long)]
        no_edit: bool,

        /// Select hunks interactively (like git add -p)
        #[arg(short, long)]
        interactive: bool,

        /// Disable TUI, use sequential prompt instead
        #[arg(long)]
        no_tui: bool,

        /// Files to include in the new commit
        #[arg(value_name = "FILES")]
        files: Vec<String>,
    },

    /// Land (merge) approved PRs/MRs starting from the first commit
    #[command(name = "land", alias = "merge")]
    Land {
        /// Land all approved PRs/MRs in sequence
        #[arg(short, long)]
        all: bool,

        /// Output structured JSON
        #[arg(long)]
        json: bool,

        /// (GitLab only) Request auto-merge ("merge when pipeline succeeds") instead of merging immediately
        #[arg(long)]
        auto_merge: bool,

        /// Disable squash when merging (default: squash enabled)
        #[arg(long = "no-squash")]
        no_squash: bool,

        /// Wait for CI to pass and approvals before merging
        #[arg(short, long)]
        wait: bool,

        /// Land commits only up to this target (position, GG-ID, or SHA)
        #[arg(short, long)]
        until: Option<String>,

        /// Automatically clean up stack after landing all PRs/MRs
        #[arg(short, long, conflicts_with = "no_clean")]
        clean: bool,

        /// Disable automatic cleanup after landing (overrides config default)
        #[arg(long = "no-clean", conflicts_with = "clean")]
        no_clean: bool,
    },

    /// Clean up merged stacks
    #[command(name = "clean", alias = "wp")]
    Clean {
        /// Clean all merged stacks without prompting
        #[arg(short, long)]
        all: bool,

        /// Output structured JSON
        #[arg(long)]
        json: bool,
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

        /// Output structured JSON
        #[arg(long)]
        json: bool,
    },

    /// Run a command on each commit in the stack
    #[command(name = "run")]
    Run {
        /// Command to run on each commit (use quotes for commands with args)
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,

        /// Stage and amend file changes into each commit
        #[arg(long, group = "mode")]
        amend: bool,

        /// Discard file changes after each commit
        #[arg(long, group = "mode")]
        discard: bool,

        /// Continue running on remaining commits after a failure
        #[arg(long)]
        keep_going: bool,

        /// Stop at this commit position (default: current)
        #[arg(short, long)]
        until: Option<usize>,

        /// Number of parallel jobs (0 = auto, 1 = sequential, default: 1)
        #[arg(short, long, default_value = "1")]
        jobs: usize,

        /// Output structured JSON
        #[arg(long)]
        json: bool,
    },

    /// Set up git-gud config for this repository
    #[command(name = "setup")]
    Setup {
        /// Configure all options (grouped by category)
        #[arg(long)]
        all: bool,
    },

    /// Absorb staged changes into the appropriate commits
    #[command(name = "absorb")]
    Absorb {
        /// Show what would be done without making changes
        #[arg(long)]
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

        /// Do not limit the search to 10 commits back. Searches all commits in the stack.
        #[arg(short = 'n', long = "no-limit")]
        no_limit: bool,

        /// Squash fixup commits directly instead of creating fixup! commits for later rebase.
        #[arg(short = 's', long)]
        squash: bool,
    },

    /// Generate shell completions
    #[command(name = "completions")]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Arrange commits in the stack: reorder and/or drop interactively (alias for reorder)
    #[command(name = "arrange")]
    Arrange {
        /// New order as positions (1-indexed) or SHAs, e.g., "3,1,2" or "3 1 2"
        /// Position 1 = bottom of stack (closest to base)
        #[arg(short, long, value_name = "ORDER")]
        order: Option<String>,

        /// Disable TUI, use text editor instead
        #[arg(long)]
        no_tui: bool,
    },

    /// Reconcile stacks that were pushed without using `gg sync`
    #[command(name = "reconcile")]
    Reconcile {
        /// Show what would be done without making changes
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    let (result, json_mode) = match cli.command {
        // No command = show stacks (like `gg ls`)
        None => (
            gg_core::commands::ls::run(false, false, false, false),
            false,
        ),

        Some(Commands::Checkout {
            stack_name,
            base,
            worktree,
        }) => (
            gg_core::commands::checkout::run(stack_name, base, worktree),
            false,
        ),
        Some(Commands::List {
            all,
            refresh,
            remote,
            json,
        }) => (gg_core::commands::ls::run(all, refresh, remote, json), json),
        Some(Commands::Sync {
            draft,
            json,
            no_rebase_check,
            force,
            update_descriptions,
            lint,
            no_lint,
            until,
        }) => {
            // Determine run_lint based on flags and config
            let run_lint = if lint {
                // --lint explicitly passed
                true
            } else if no_lint {
                // --no-lint explicitly passed
                false
            } else {
                // No explicit flag, use config default
                match gg_core::git::open_repo()
                    .and_then(|repo| gg_core::config::Config::load_with_global(repo.commondir()))
                {
                    Ok(cfg) => cfg.get_sync_auto_lint(),
                    Err(_) => false, // If we can't load config, default to false
                }
            };

            (
                gg_core::commands::sync::run(
                    draft,
                    json,
                    no_rebase_check,
                    force,
                    update_descriptions,
                    run_lint,
                    until,
                ),
                json,
            )
        }
        Some(Commands::Move { target }) => (gg_core::commands::nav::move_to(&target), false),
        Some(Commands::First) => (gg_core::commands::nav::first(), false),
        Some(Commands::Last) => (gg_core::commands::nav::last(), false),
        Some(Commands::Prev) => (gg_core::commands::nav::prev(), false),
        Some(Commands::Next) => (gg_core::commands::nav::next(), false),
        Some(Commands::Squash { all }) => (gg_core::commands::squash::run(all), false),
        Some(Commands::Drop {
            targets,
            force,
            json,
        }) => (
            gg_core::commands::drop_cmd::run(gg_core::commands::drop_cmd::DropOptions {
                targets,
                force,
                json,
            }),
            json,
        ),
        Some(Commands::Reorder { order, no_tui }) => (
            gg_core::commands::reorder::run(gg_core::commands::reorder::ReorderOptions {
                order,
                no_tui,
            }),
            false,
        ),
        Some(Commands::Split {
            commit,
            message,
            no_edit,
            interactive,
            no_tui,
            files,
        }) => (
            gg_core::commands::split::run(gg_core::commands::split::SplitOptions {
                target: commit,
                files,
                message,
                no_edit,
                interactive,
                no_tui,
            }),
            false,
        ),
        Some(Commands::Land {
            all,
            json,
            auto_merge,
            no_squash,
            wait,
            until,
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
                match gg_core::git::open_repo()
                    .and_then(|repo| gg_core::config::Config::load_with_global(repo.commondir()))
                {
                    Ok(cfg) => cfg.get_land_auto_clean(),
                    Err(_) => false, // If we can't load config, default to false
                }
            };

            (
                gg_core::commands::land::run(
                    all, json, !no_squash, wait, auto_clean, auto_merge, until,
                ),
                json,
            )
        }
        Some(Commands::Clean { all, json }) => (gg_core::commands::clean::run(all, json), json),
        Some(Commands::Rebase { target }) => (gg_core::commands::rebase::run(target), false),
        Some(Commands::Continue) => (gg_core::commands::rebase::continue_rebase(), false),
        Some(Commands::Abort) => (gg_core::commands::rebase::abort_rebase(), false),
        Some(Commands::Lint { until, json }) => (
            gg_core::commands::lint::run(until, json, json).map(|_| ()),
            json,
        ),
        Some(Commands::Run {
            command,
            amend,
            discard,
            keep_going,
            until,
            jobs,
            json,
        }) => {
            let change_mode = if amend {
                gg_core::commands::run::ChangeMode::Amend
            } else if discard {
                gg_core::commands::run::ChangeMode::Discard
            } else {
                gg_core::commands::run::ChangeMode::ReadOnly
            };

            match gg_core::commands::run::execute(gg_core::commands::run::RunOptions {
                commands: vec![gg_core::commands::run::RunCommand::Argv(command)],
                change_mode,
                until,
                stop_on_error: !keep_going,
                json,
                emit_json_output: json,
                header_label: None,
                jobs,
            }) {
                Ok(true) => (Ok(()), json),
                // `execute` has already emitted the JSON run payload (when
                // `json` is true) and/or the terminal output (when not).
                // Exiting directly with code 1 avoids the generic error path
                // appending a second `{"error":...}` document to stdout,
                // which would break JSON consumers expecting one object.
                Ok(false) => exit(1),
                Err(e) => (Err(e), json),
            }
        }
        Some(Commands::Setup { all }) => (gg_core::commands::setup::run(all), false),
        Some(Commands::Absorb {
            dry_run,
            and_rebase,
            whole_file,
            one_fixup_per_commit,
            no_limit,
            squash,
        }) => (
            gg_core::commands::absorb::run(gg_core::commands::absorb::AbsorbOptions {
                dry_run,
                and_rebase,
                whole_file,
                one_fixup_per_commit,
                no_limit,
                squash,
            }),
            false,
        ),
        Some(Commands::Arrange { order, no_tui }) => (
            gg_core::commands::reorder::run(gg_core::commands::reorder::ReorderOptions {
                order,
                no_tui,
            }),
            false,
        ),
        Some(Commands::Completions { shell }) => {
            (gg_core::commands::completions::run(shell), false)
        }
        Some(Commands::Reconcile { dry_run }) => {
            (gg_core::commands::reconcile::run(dry_run), false)
        }
    };

    if let Err(e) = result {
        if json_mode {
            gg_core::output::print_json_error(&e.to_string());
        } else {
            eprintln!("{} {}", style("error:").red().bold(), e);
        }
        exit(1);
    }
}
