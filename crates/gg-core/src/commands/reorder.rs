//! `gg reorder` - Reorder commits in the stack

use std::io::Write;

use console::style;
use dialoguer::Editor;

use super::reorder_tui::{self, ReorderEntry};
use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack::Stack;

/// Options for the reorder command
#[derive(Debug, Default)]
pub struct ReorderOptions {
    /// New order specified as positions (1-indexed), e.g., "3,1,2" or "3 1 2"
    /// Position 1 = bottom of stack (closest to base)
    pub order: Option<String>,
    /// If true, disable TUI and use editor fallback
    pub no_tui: bool,
}

/// Run the reorder command
pub fn run(options: ReorderOptions) -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;

    // Require clean working directory
    git::require_clean_working_directory(&repo)?;

    // Load stack
    let stack = Stack::load(&repo, &config)?;

    if stack.len() < 2 {
        println!("{}", style("Need at least 2 commits to reorder.").dim());
        return Ok(());
    }

    // Get the new order - from CLI, TUI, or editor
    let new_order = if let Some(order_str) = options.order {
        parse_order_from_string(&order_str, &stack)?
    } else {
        let is_tty = atty::is(atty::Stream::Stdin) && atty::is(atty::Stream::Stdout);
        let use_tui = !options.no_tui && is_tty;
        if use_tui {
            get_order_from_tui(&stack)?
        } else {
            get_order_from_editor(&stack)?
        }
    };

    // Handle cancellation
    let new_order = match new_order {
        Some(order) => order,
        None => {
            println!("{}", style("Reorder cancelled.").dim());
            return Ok(());
        }
    };

    if new_order.is_empty() {
        println!("{}", style("No commits in reorder list. Aborting.").dim());
        return Ok(());
    }

    // Check if order actually changed (and no drops)
    let old_order: Vec<&str> = stack.entries.iter().map(|e| e.short_sha.as_str()).collect();
    if new_order.len() == old_order.len() && new_order == old_order {
        println!("{}", style("Order unchanged.").dim());
        return Ok(());
    }

    let dropped_count = stack.len() - new_order.len();
    if dropped_count > 0 {
        println!(
            "{}",
            style(format!(
                "Reordering {} commits, dropping {}...",
                new_order.len(),
                dropped_count
            ))
            .dim()
        );
    } else {
        println!("{}", style("Reordering commits...").dim());
    }

    // Perform the rebase with the new order
    perform_reorder(&repo, &stack, &new_order)?;

    // Ensure GG metadata reflects the new stack order
    let rewritten_stack = Stack::load(&repo, &config)?;
    git::normalize_stack_metadata(&repo, &rewritten_stack)?;

    if dropped_count > 0 {
        println!(
            "{} Arranged stack: {} commits kept, {} dropped",
            style("OK").green().bold(),
            new_order.len(),
            dropped_count
        );
    } else {
        println!(
            "{} Reordered {} commits",
            style("OK").green().bold(),
            new_order.len()
        );
    }

    Ok(())
}

/// Parse order from a string like "3,1,2" or "3 1 2" (positions) or "abc123,def456" (SHAs)
fn parse_order_from_string(order_str: &str, stack: &Stack) -> Result<Option<Vec<String>>> {
    // Split by comma, space, or both
    let parts: Vec<&str> = order_str
        .split([',', ' '])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if parts.is_empty() {
        return Err(GgError::Other("Empty order string".to_string()));
    }

    // Check if the first part looks like a number (position) or a SHA
    let is_positions = parts[0].parse::<usize>().is_ok();

    let new_order: Vec<String> = if is_positions {
        // Parse as positions (1-indexed)
        let mut result = Vec::new();
        for part in &parts {
            let pos: usize = part.parse().map_err(|_| {
                GgError::Other(format!(
                    "Invalid position: '{}'. Use 1-{}",
                    part,
                    stack.len()
                ))
            })?;

            if pos == 0 || pos > stack.len() {
                return Err(GgError::Other(format!(
                    "Position {} out of range. Use 1-{}",
                    pos,
                    stack.len()
                )));
            }

            let entry = &stack.entries[pos - 1];
            result.push(entry.short_sha.clone());
        }
        result
    } else {
        // Parse as SHAs
        let mut result = Vec::new();
        for part in &parts {
            // Find matching entry
            let entry = stack
                .entries
                .iter()
                .find(|e| {
                    e.short_sha.starts_with(part)
                        || part.starts_with(&e.short_sha)
                        || e.gg_id.as_ref().map(|id| id == part).unwrap_or(false)
                })
                .ok_or_else(|| GgError::Other(format!("Unknown commit: '{}'", part)))?;
            result.push(entry.short_sha.clone());
        }
        result
    };

    // Validate all commits are accounted for
    if new_order.len() != stack.len() {
        return Err(GgError::Other(format!(
            "Order must include all {} commits, got {}",
            stack.len(),
            new_order.len()
        )));
    }

    // Check for duplicates
    let mut seen = std::collections::HashSet::new();
    for sha in &new_order {
        if !seen.insert(sha) {
            return Err(GgError::Other(format!(
                "Duplicate commit in order: {}",
                sha
            )));
        }
    }

    Ok(Some(new_order))
}

/// Get order from interactive TUI
fn get_order_from_tui(stack: &Stack) -> Result<Option<Vec<String>>> {
    let entries: Vec<ReorderEntry> = stack
        .entries
        .iter()
        .map(|e| ReorderEntry {
            short_sha: e.short_sha.clone(),
            title: e.title.clone(),
        })
        .collect();

    reorder_tui::reorder_tui(entries)
}

/// Get order from interactive editor
fn get_order_from_editor(stack: &Stack) -> Result<Option<Vec<String>>> {
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
        None => return Ok(None),
    };

    // Parse the edited content
    let new_order: Vec<String> = edited
        .lines()
        .filter(|line| !line.trim().starts_with('#') && !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .filter(|sha| !sha.is_empty())
        .map(|s| s.to_string())
        .collect();

    if new_order.is_empty() {
        return Err(GgError::Other(
            "Cannot drop all commits. At least one must remain.".to_string(),
        ));
    }

    // Validate all SHAs from editor match stack entries
    let valid_shas: Vec<&str> = stack.entries.iter().map(|e| e.short_sha.as_str()).collect();
    for sha in &new_order {
        let is_valid = valid_shas
            .iter()
            .any(|s| s.starts_with(sha.as_str()) || sha.starts_with(*s));
        if !is_valid {
            return Err(GgError::Other(format!("Unknown commit SHA: {}", sha)));
        }
    }

    // Check for duplicates
    let mut seen = std::collections::HashSet::new();
    for sha in &new_order {
        if !seen.insert(sha) {
            return Err(GgError::Other(format!(
                "Duplicate commit in order: {}",
                sha
            )));
        }
    }

    Ok(Some(new_order))
}

/// Perform the actual reorder via git rebase
fn perform_reorder(repo: &git2::Repository, stack: &Stack, new_order: &[String]) -> Result<()> {
    // First, start a rebase
    let base_ref = repo
        .revparse_single(&stack.base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", stack.base)))?;

    // Build the rebase todo
    let mut rebase_todo = String::new();
    for sha in new_order {
        // Find the full SHA
        let full_sha = stack
            .entries
            .iter()
            .find(|e| e.short_sha.starts_with(sha) || sha.starts_with(&e.short_sha))
            .map(|e| e.oid.to_string())
            .unwrap_or_else(|| sha.to_string());
        rebase_todo.push_str(&format!("pick {}\n", full_sha));
    }

    // Use environment variables to control the rebase
    // GIT_SEQUENCE_EDITOR allows us to provide the todo list
    // Use unique filenames to avoid conflicts in parallel test runs
    let unique_id = std::process::id();
    let todo_file = std::env::temp_dir().join(format!("gg-rebase-todo-{}", unique_id));
    std::fs::write(&todo_file, &rebase_todo)?;

    let editor_script = format!("#!/bin/sh\ncat {} > \"$1\"", todo_file.display());
    let script_file = std::env::temp_dir().join(format!("gg-rebase-editor-{}.sh", unique_id));
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

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reorder_options_default() {
        let opts = ReorderOptions::default();
        assert!(opts.order.is_none());
    }

    #[test]
    fn test_reorder_options_with_order() {
        let opts = ReorderOptions {
            order: Some("3,1,2".to_string()),
            ..Default::default()
        };
        assert_eq!(opts.order, Some("3,1,2".to_string()));
        assert!(!opts.no_tui);
    }

    #[test]
    fn test_reorder_options_no_tui() {
        let opts = ReorderOptions {
            no_tui: true,
            ..Default::default()
        };
        assert!(opts.no_tui);
    }

    fn make_test_stack() -> Stack {
        use crate::stack::StackEntry;
        let make_entry = |sha: &str, title: &str, gg_id: &str, pos: usize| StackEntry {
            oid: git2::Oid::zero(),
            short_sha: sha.to_string(),
            title: title.to_string(),
            gg_id: Some(gg_id.to_string()),
            gg_parent: None,
            mr_number: None,
            mr_state: None,
            approved: false,
            ci_status: None,
            position: pos,
            in_merge_train: false,
            merge_train_position: None,
        };
        Stack {
            name: "test".to_string(),
            username: "testuser".to_string(),
            base: "main".to_string(),
            entries: vec![
                make_entry("aaa1111", "commit 1", "c-0000001", 1),
                make_entry("bbb2222", "commit 2", "c-0000002", 2),
                make_entry("ccc3333", "commit 3", "c-0000003", 3),
            ],
            current_position: Some(1),
        }
    }

    #[test]
    fn test_parse_order_rejects_fewer_entries() {
        let stack = make_test_stack();
        // CLI --order must include all commits
        let result = parse_order_from_string("1,2", &stack);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must include all"));
    }

    #[test]
    fn test_parse_order_rejects_duplicates() {
        let stack = make_test_stack();
        let result = parse_order_from_string("1,1,2", &stack);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate"));
    }

    #[test]
    fn test_parse_order_valid() {
        let stack = make_test_stack();
        let result = parse_order_from_string("3,1,2", &stack).unwrap();
        assert_eq!(
            result,
            Some(vec![
                "ccc3333".to_string(),
                "aaa1111".to_string(),
                "bbb2222".to_string()
            ])
        );
    }
}
