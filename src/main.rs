use std::process::exit;
use clap::{Parser, Subcommand};
use git2::Repository;

#[derive(Parser, Debug)]
#[command(author = "Nacho Lopez", version)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Feature {
        branch_name: String,
    },
    Ls,
}

fn main() {
    // Try to open a repo in the current repo
    let repo = Repository::open(".");
    match repo {
        Ok(_) => {}
        Err(_) => {
            println!("Not in a git repository.");
            exit(1);
        }
    }

    let args = Cli::parse();
    println!("{:?}", args);

    match args.command {
        Commands::Feature { branch_name } => {
            println!("Feature! {}", branch_name);
        }
        Commands::Ls => {
            println!("Ls!");
        }
    }
}
