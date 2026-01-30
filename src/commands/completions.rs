//! `gg completions` - Generate shell completions

use std::io;

use clap::CommandFactory;
use clap_complete::{generate, Shell};

use crate::error::Result;

// We need access to the CLI struct, so we'll recreate it minimally here
// or import from main. For simplicity, let's generate based on the command structure.

#[derive(clap::Parser)]
#[command(name = "gg")]
struct Cli {
    #[clap(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    #[command(name = "co", alias = "sw", alias = "checkout", alias = "switch")]
    Checkout {
        stack_name: Option<String>,
        #[arg(short, long)]
        base: Option<String>,
    },
    #[command(name = "ls", alias = "list")]
    List {
        #[arg(short, long)]
        all: bool,
        #[arg(short, long)]
        refresh: bool,
    },
    #[command(name = "sync", alias = "diff")]
    Sync {
        #[arg(short, long)]
        draft: bool,
        #[arg(short, long)]
        force: bool,
    },
    #[command(name = "mv", alias = "move")]
    Move { target: String },
    #[command(name = "first")]
    First,
    #[command(name = "last")]
    Last,
    #[command(name = "prev", alias = "previous")]
    Prev,
    #[command(name = "next")]
    Next,
    #[command(name = "sc", alias = "squash")]
    Squash {
        #[arg(short, long)]
        all: bool,
    },
    #[command(name = "reorder")]
    Reorder,
    #[command(name = "land", alias = "merge")]
    Land {
        #[arg(short, long)]
        all: bool,
        #[arg(short, long)]
        squash: bool,
    },
    #[command(name = "clean")]
    Clean {
        #[arg(short, long)]
        all: bool,
    },
    #[command(name = "rebase")]
    Rebase { target: Option<String> },
    #[command(name = "continue")]
    Continue,
    #[command(name = "abort")]
    Abort,
    #[command(name = "lint")]
    Lint {
        #[arg(short, long)]
        until: Option<usize>,
    },
    #[command(name = "absorb")]
    Absorb,
    #[command(name = "completions")]
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

/// Run the completions command
pub fn run(shell: Shell) -> Result<()> {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    generate(shell, &mut cmd, name, &mut io::stdout());
    Ok(())
}
