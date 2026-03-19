//! `gg drop` - Remove commits from the stack

use std::io::Write;

use console::style;
use dialoguer::Confirm;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::output::{print_json, DropResponse, DropResultJson, DroppedEntryJson, OUTPUT_VERSION};
use crate::stack::{self, Stack};

/// Options for the drop command
#[derive(Debug, Default)]
pub struct DropOptions {
    /// Targets to drop: position (1-indexed), short SHA, or GG-ID
    pub targets: Vec<String>,
    /// Skip confirmation prompt
    pub force: bool,
    /// Output as JSON
    pub json: bool,
}

/// Run the drop command
pub fn run(options: DropOptions) -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;

    // Acquire operation lock
    let _lock = git::acquire_operation_lock(&repo, "drop")?;

    // Require clean working directory
    git::require_clean_working_directory(&repo)?;

    // Load stack
    let stack_obj = Stack::load(&repo, &config)?;

    if stack_obj.is_empty() {
        return Err(GgError::Other("Stack is empty".to_string()));
    }

    if options.targets.is_empty() {
        return Err(GgError::Other(
            "No targets specified. Provide positions (1-indexed), short SHAs, or GG-IDs."
                .to_string(),
        ));
    }

    // Resolve targets to positions (1-indexed)
    let mut drop_positions: Vec<usize> = Vec::new();
    for target in &options.targets {
        let pos = stack::resolve_target(&stack_obj, target)?;
        if !drop_positions.contains(&pos) {
            drop_positions.push(pos);
        }
    }

    // Validate: can't drop ALL commits
    if drop_positions.len() >= stack_obj.len() {
        return Err(GgError::Other(
            "Cannot drop all commits from the stack. At least one commit must remain.".to_string(),
        ));
    }

    // Build list of entries to drop for display and JSON
    let mut dropped_entries: Vec<DroppedEntryJson> = Vec::new();
    for &pos in &drop_positions {
        let entry = stack_obj
            .get_entry_by_position(pos)
            .ok_or_else(|| GgError::Other(format!("Position {} out of range", pos)))?;
        dropped_entries.push(DroppedEntryJson {
            position: pos,
            sha: entry.short_sha.clone(),
            title: entry.title.clone(),
        });
    }

    // Show what will be dropped
    if !options.json && !options.force {
        println!(
            "{} Will drop {} commit(s):",
            style("Drop").red().bold(),
            dropped_entries.len()
        );
        for entry in &dropped_entries {
            println!(
                "  {} {} {}",
                style(format!("#{}", entry.position)).dim(),
                style(&entry.sha).yellow(),
                entry.title
            );
        }
        println!();

        // Ask for confirmation
        let confirmed = Confirm::new()
            .with_prompt("Proceed with drop?")
            .default(false)
            .interact()
            .unwrap_or(false);

        if !confirmed {
            println!("{}", style("Drop cancelled.").dim());
            return Ok(());
        }
    }

    // Build the rebase todo list, omitting dropped commits
    let drop_indices: Vec<usize> = drop_positions.iter().map(|p| p - 1).collect();
    let kept_entries: Vec<&crate::stack::StackEntry> = stack_obj
        .entries
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop_indices.contains(i))
        .map(|(_, e)| e)
        .collect();

    // Perform rebase omitting dropped commits
    let base_ref = repo
        .revparse_single(&stack_obj.base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", stack_obj.base)))?;

    let mut rebase_todo = String::new();
    for entry in &kept_entries {
        rebase_todo.push_str(&format!("pick {}\n", entry.oid));
    }

    let unique_id = std::process::id();
    let todo_file = std::env::temp_dir().join(format!("gg-drop-todo-{}", unique_id));
    std::fs::write(&todo_file, &rebase_todo)?;

    let editor_script = format!("#!/bin/sh\ncat {} > \"$1\"", todo_file.display());
    let script_file = std::env::temp_dir().join(format!("gg-drop-editor-{}.sh", unique_id));
    {
        let mut f = std::fs::File::create(&script_file)?;
        f.write_all(editor_script.as_bytes())?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_file)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_file, perms)?;
    }

    let output = std::process::Command::new("git")
        .env("GIT_SEQUENCE_EDITOR", script_file.to_str().unwrap())
        .args(["rebase", "-i", &base_ref.id().to_string()])
        .output()?;

    let _ = std::fs::remove_file(&todo_file);
    let _ = std::fs::remove_file(&script_file);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("CONFLICT") || stderr.contains("conflict") {
            return Err(GgError::RebaseConflict);
        }
        return Err(GgError::Other(format!("Rebase failed: {}", stderr)));
    }

    // Clean up per-commit branches for dropped commits
    for entry in &dropped_entries {
        // Find the matching stack entry to get the GG-ID for branch name
        if let Some(stack_entry) = stack_obj.get_entry_by_position(entry.position) {
            if let Some(branch_name) = stack_obj.entry_branch_name(stack_entry) {
                // Delete local branch (ignore errors if it doesn't exist)
                let _ = repo
                    .find_branch(&branch_name, git2::BranchType::Local)
                    .and_then(|mut b| b.delete());
            }
        }
    }

    let remaining = stack_obj.len() - dropped_entries.len();

    if options.json {
        print_json(&DropResponse {
            version: OUTPUT_VERSION,
            drop: DropResultJson {
                dropped: dropped_entries,
                remaining,
            },
        });
    } else {
        println!(
            "{} Dropped {} commit(s), {} remaining",
            style("OK").green().bold(),
            drop_positions.len(),
            remaining
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drop_options_default() {
        let opts = DropOptions::default();
        assert!(opts.targets.is_empty());
        assert!(!opts.force);
        assert!(!opts.json);
    }

    #[test]
    fn test_drop_options_with_values() {
        let opts = DropOptions {
            targets: vec!["1".to_string(), "c-abc1234".to_string()],
            force: true,
            json: false,
        };
        assert_eq!(opts.targets.len(), 2);
        assert!(opts.force);
    }
}
