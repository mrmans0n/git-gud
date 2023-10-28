use std::process::{Command, exit};

use fancy::printcoln;
use git2::Repository;

pub fn squash_to_previous_commit(repository: Repository) {
    // TODO revisit this, can't make the amend work with git2 :(

    let output = Command::new("git")
        .args(&["commit", "--all", "--amend", "--no-edit"])
        .output()
        .expect("Failed to execute git command");

    if !output.status.success() {
        let error_message = String::from_utf8_lossy(&output.stderr);
        eprintln!("Could not amend commit. Error: {}", error_message);
        exit(1);
    }

    // Print success message
    let head = repository.head().expect("Failed to access HEAD").peel_to_commit().expect("HEAD is not a commit");
    print!("📦💥🤛 ");
    printcoln!(
        "Squashed to: [b]{}[:]",
        head.summary().unwrap_or("<no summary>")
    );
}
