mod commands;

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
    #[command(name = "feature", alias = "f", about = "Creates a new branch")]
    Feature {
        branch_name: String,
    },
    #[command(name = "ls", alias = "list", about = "Lists all commits")]
    Ls,
    #[command(name = "squash", alias = "sc", about = "Squash all changes in the previous commit")]
    Squash,
}

fn check_if_in_repo() -> Repository {
    // Try to open a repo in the current repo
    let maybe_repo = Repository::open(".");

    if let Err(err) = maybe_repo {
        eprintln!("Not in a git repository: {}", err.to_string());
        exit(1);
    }

    return maybe_repo.unwrap();
}

fn main() {
    let args = Cli::parse();
    println!("{:?}", args);

    match args.command {
        Commands::Feature { branch_name } => {
            let repo = check_if_in_repo();
            commands::feature::create_branch_off_of_main(repo, branch_name);
        }
        Commands::Ls => {
            let repo = check_if_in_repo();
            commands::ls::list_commits_off_of_main(repo);
        }
        Commands::Squash => {
            let repo = check_if_in_repo();
            commands::squash::squash_to_previous_commit(repo);
        }
    }
}
