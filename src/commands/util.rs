use std::process::{Command, exit};

use git2::{Commit, Repository, Revwalk};
use uuid::Uuid;

pub(crate) fn get_all_commits_from_main(repository: &Repository) -> Revwalk {
    let mut revwalk = repository.revwalk().expect("Could not create revwalk");

    // Get the commit ids for HEAD and main
    let head = repository
        .revparse_single("HEAD")
        .expect("Could not find HEAD");
    let main = repository
        .revparse_single("main")
        .or_else(|_| repository.revparse_single("master"))
        .expect("Could not find main branch");

    // Push the range to the revwalk
    revwalk
        .push_range(&format!("{}..{}", main.id(), head.id()))
        .expect("Could not push range between main..head commits");
    revwalk
}

pub(crate) fn get_metadata_id_from_commit(commit: &Commit) -> Option<String> {
    let message = commit.message().unwrap_or("");
    for line in message.lines() {
        if line.starts_with("gg-id:") {
            let parts: Vec<&str> = line.splitn(2, ": ").collect();
            if let Some(id_part) = parts.get(1) {
                return Some(id_part.to_string());
            }
        }
    }
    return None;
}

pub(crate) fn write_metadata_id_to_head(repository: &Repository) -> String {
    let head = repository.head()
        .expect("Could not access HEAD")
        .peel_to_commit()
        .expect("HEAD is not a commit");

    // Check if the commit already has a token, if so, return that.
    if let Some(metadata_id) = get_metadata_id_from_commit(&head) {
        return metadata_id;
    }

    let head_message_raw = head.message().unwrap_or("");

    // Sanitize head_message: " -> \"
    let head_message = head_message_raw.replace('"', "\"");

    let uuid = Uuid::new_v4().to_string();

    // Write this message to commit
    let new_message = format!("\"{head_message}\n\ngg-id: {uuid}\"");

    let output = Command::new("git")
        .args(&["commit", "--amend", "-m", &new_message])
        .output()
        .expect("Failed to execute git command");

    if !output.status.success() {
        let error_message = String::from_utf8_lossy(&output.stderr);
        eprintln!("Could not amend commit. Error: {error_message}");
        exit(1);
    }

    return uuid.to_string();
}