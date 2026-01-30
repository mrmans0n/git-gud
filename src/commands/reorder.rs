//! `gg reorder` - Reorder commits in the stack

use std::io::Write;

use console::style;
use dialoguer::Editor;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack::Stack;

/// Run the reorder command
pub fn run() -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load(repo.path())?;

    // Require clean working directory
    git::require_clean_working_directory(&repo)?;

    // Load stack
    let stack = Stack::load(&repo, &config)?;

    if stack.len() < 2 {
        println!("{}", style("Need at least 2 commits to reorder.").dim());
        return Ok(());
    }

    // Build the todo list for editing
    let mut todo_content = String::new();
    todo_content.push_str("# Reorder commits by rearranging lines.\n");
    todo_content.push_str("# Lines starting with '#' are comments.\n");
    todo_content.push_str("# Delete a line to drop that commit.\n");
    todo_content
        .push_str("# The first commit will be at the bottom of the stack (closest to base).\n");
    todo_content.push_str("#\n");

    for entry in &stack.entries {
        let gg_id = entry.gg_id.as_deref().unwrap_or(&entry.short_sha);
        todo_content.push_str(&format!("{} {} {}\n", entry.short_sha, gg_id, entry.title));
    }

    // Open editor for user to reorder
    let edited = Editor::new()
        .extension(".txt")
        .edit(&todo_content)
        .map_err(|e| GgError::Other(format!("Editor failed: {}", e)))?;

    let edited = match edited {
        Some(content) => content,
        None => {
            println!("{}", style("Reorder cancelled.").dim());
            return Ok(());
        }
    };

    // Parse the edited content
    let new_order: Vec<&str> = edited
        .lines()
        .filter(|line| !line.trim().starts_with('#') && !line.trim().is_empty())
        .map(|line| line.split_whitespace().next().unwrap_or(""))
        .filter(|sha| !sha.is_empty())
        .collect();

    if new_order.is_empty() {
        println!("{}", style("No commits in reorder list. Aborting.").dim());
        return Ok(());
    }

    // Check if order actually changed
    let old_order: Vec<&str> = stack.entries.iter().map(|e| e.short_sha.as_str()).collect();
    if new_order == old_order {
        println!("{}", style("Order unchanged.").dim());
        return Ok(());
    }

    // Validate all SHAs are present
    for sha in &new_order {
        if !old_order
            .iter()
            .any(|s| s.starts_with(*sha) || sha.starts_with(*s))
        {
            return Err(GgError::Other(format!("Unknown commit SHA: {}", sha)));
        }
    }

    println!("{}", style("Reordering commits...").dim());

    // Perform interactive rebase with the new order
    // We'll create a git-rebase-todo file and run the rebase

    // First, start a rebase
    let base_ref = repo
        .revparse_single(&stack.base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", stack.base)))?;

    // Build the rebase todo
    let mut rebase_todo = String::new();
    for sha in &new_order {
        // Find the full SHA
        let full_sha = stack
            .entries
            .iter()
            .find(|e| e.short_sha.starts_with(*sha) || sha.starts_with(&e.short_sha))
            .map(|e| e.oid.to_string())
            .unwrap_or_else(|| sha.to_string());
        rebase_todo.push_str(&format!("pick {}\n", full_sha));
    }

    // Use environment variables to control the rebase
    // GIT_SEQUENCE_EDITOR allows us to provide the todo list
    let todo_file = std::env::temp_dir().join("gg-rebase-todo");
    std::fs::write(&todo_file, &rebase_todo)?;

    let editor_script = format!("#!/bin/sh\ncat {} > \"$1\"", todo_file.display());
    let script_file = std::env::temp_dir().join("gg-rebase-editor.sh");
    {
        let mut f = std::fs::File::create(&script_file)?;
        f.write_all(editor_script.as_bytes())?;
    }

    // Make script executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_file)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_file, perms)?;
    }

    // Run the rebase
    let output = std::process::Command::new("git")
        .env("GIT_SEQUENCE_EDITOR", script_file.to_str().unwrap())
        .args(["rebase", "-i", &base_ref.id().to_string()])
        .output()?;

    // Clean up temp files
    let _ = std::fs::remove_file(&todo_file);
    let _ = std::fs::remove_file(&script_file);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("CONFLICT") || stderr.contains("conflict") {
            return Err(GgError::RebaseConflict);
        }
        return Err(GgError::Other(format!("Rebase failed: {}", stderr)));
    }

    println!(
        "{} Reordered {} commits",
        style("OK").green().bold(),
        new_order.len()
    );

    Ok(())
}
