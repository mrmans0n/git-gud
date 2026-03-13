//! `gg split` - Split a commit into two

use std::process::Command;

use console::style;
use dialoguer::{Editor, MultiSelect};

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::stack::{self, Stack};

/// Options for the split command
#[derive(Debug, Default)]
pub struct SplitOptions {
    /// Target commit: position (1-indexed), short SHA, or GG-ID. None = current HEAD.
    pub target: Option<String>,
    /// Files to include in the new (first/lower) commit. Empty = interactive selection.
    pub files: Vec<String>,
    /// Commit message for the new (first/lower) commit. None = prompt via editor.
    pub message: Option<String>,
    /// If true, don't prompt for the remainder commit message (keep original).
    pub no_edit: bool,
}

/// Information about a file changed in a commit
#[derive(Debug, Clone)]
struct ChangedFile {
    /// File path relative to repo root
    path: String,
    /// Lines added
    additions: usize,
    /// Lines deleted
    deletions: usize,
}

impl std::fmt::Display for ChangedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:<50} (+{} -{})",
            self.path, self.additions, self.deletions
        )
    }
}

/// Run the split command
pub fn run(options: SplitOptions) -> Result<()> {
    let repo = git::open_repo()?;
    let _lock = git::acquire_operation_lock(&repo, "split")?;
    let config = Config::load(repo.commondir())?;

    git::require_clean_working_directory(&repo)?;

    let stack = Stack::load(&repo, &config)?;

    // Resolve target commit position (1-indexed)
    let target_pos = match &options.target {
        Some(target) => stack::resolve_target(&stack, target)?,
        None => {
            // Default to current position or stack head
            stack.current_position.map(|p| p + 1).unwrap_or(stack.len())
        }
    };

    let entry = stack.get_entry_by_position(target_pos).ok_or_else(|| {
        GgError::Other(format!(
            "Commit at position {} not found in stack",
            target_pos
        ))
    })?;

    let target_oid = entry.oid;
    let target_commit = repo.find_commit(target_oid)?;
    let original_gg_id = entry.gg_id.clone();

    println!(
        "Splitting commit {}: {} ({})",
        target_pos,
        style(git::get_commit_title(&target_commit)).bold(),
        style(git::short_sha(&target_commit)).yellow()
    );

    // Get parent commit
    let parent_commit = target_commit
        .parent(0)
        .map_err(|_| GgError::Other("Cannot split the root commit (no parent)".to_string()))?;

    // Get list of changed files with stats
    let changed_files = get_changed_files(&repo, &parent_commit, &target_commit)?;

    if changed_files.is_empty() {
        return Err(GgError::Other(
            "Commit has no file changes to split".to_string(),
        ));
    }

    if changed_files.len() < 2 && options.files.is_empty() {
        return Err(GgError::Other(
            "Commit only has 1 file. Use hunk-level splitting (-i) in a future version."
                .to_string(),
        ));
    }

    // Determine which files go to the new (first/lower) commit
    let selected_files = if options.files.is_empty() {
        select_files_interactive(&changed_files)?
    } else {
        validate_file_selection(&options.files, &changed_files)?
    };

    if selected_files.is_empty() {
        return Err(GgError::Other(
            "No files selected, nothing to split".to_string(),
        ));
    }

    let all_selected = selected_files.len() == changed_files.len();
    if all_selected {
        println!(
            "{}",
            style("⚠ All files selected — the original commit will become empty.").yellow()
        );
    }

    // Get commit messages
    let new_commit_message = get_new_commit_message(&options, &target_commit)?;
    let remainder_message = get_remainder_message(&options, &target_commit)?;

    // === Perform the split ===

    // 1. Build the tree for the new (first/lower) commit:
    //    Start from parent tree, add selected file blobs from target tree.
    let parent_tree = parent_commit.tree()?;
    let target_tree = target_commit.tree()?;
    let first_tree = build_partial_tree(&repo, &parent_tree, &target_tree, &selected_files)?;

    // 2. Create the first (new, lower) commit
    let sig = git::get_signature(&repo)?;
    let new_gg_id = git::generate_gg_id();
    let first_message = git::set_gg_id_in_message(&new_commit_message, &new_gg_id);
    let first_oid = repo.commit(
        None, // don't update any ref
        &sig,
        &sig,
        &first_message,
        &first_tree,
        &[&parent_commit],
    )?;
    let first_commit = repo.find_commit(first_oid)?;

    // 3. Create the second (remainder, upper) commit
    //    Tree = original target tree (all changes). Parent = first commit.
    //    This means the diff of second commit = only the non-selected files.
    let remainder_msg = if let Some(gg_id) = &original_gg_id {
        git::set_gg_id_in_message(&remainder_message, gg_id)
    } else {
        remainder_message.clone()
    };
    let second_oid = repo.commit(
        None,
        &sig,
        &sig,
        &remainder_msg,
        &target_tree,
        &[&first_commit],
    )?;
    let second_commit = repo.find_commit(second_oid)?;

    // 4. Rebase descendants onto second commit
    let num_rebased = rebase_descendants(
        &repo,
        &stack,
        &config,
        target_pos,
        &target_commit,
        &second_commit,
    )?;

    // Print results
    println!("{} Split complete!", style("✔").green().bold());
    println!(
        "  New commit {} (before): {} {}",
        target_pos,
        style(git::short_sha(&first_commit)).yellow(),
        style(git::get_commit_title(&first_commit)).dim()
    );
    println!(
        "  Original commit {} (after): {} {}",
        target_pos + 1,
        style(git::short_sha(&second_commit)).yellow(),
        style(git::get_commit_title(&second_commit)).dim()
    );
    if num_rebased > 0 {
        println!(
            "  Rebased {} descendant commit{}.",
            num_rebased,
            if num_rebased == 1 { "" } else { "s" }
        );
    }

    Ok(())
}

/// Get the list of files changed between two commits
fn get_changed_files(
    repo: &git2::Repository,
    parent: &git2::Commit,
    target: &git2::Commit,
) -> Result<Vec<ChangedFile>> {
    let parent_tree = parent.tree()?;
    let target_tree = target.tree()?;

    let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&target_tree), None)?;

    let mut files = Vec::new();
    let num_deltas = diff.deltas().len();
    for i in 0..num_deltas {
        let delta = diff.get_delta(i).unwrap();
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        files.push(ChangedFile {
            path,
            additions: 0,
            deletions: 0,
        });
    }

    // Get per-file line stats from patches
    diff.foreach(
        &mut |_delta, _progress| true,
        None, // binary callback
        Some(&mut |_delta, _hunk| true),
        Some(&mut |delta, _hunk, line| {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            if let Some(file) = files.iter_mut().find(|f| f.path == path) {
                match line.origin() {
                    '+' => file.additions += 1,
                    '-' => file.deletions += 1,
                    _ => {}
                }
            }
            true
        }),
    )?;

    Ok(files)
}

/// Interactive file selection using dialoguer MultiSelect
fn select_files_interactive(changed_files: &[ChangedFile]) -> Result<Vec<String>> {
    let items: Vec<String> = changed_files.iter().map(|f| f.to_string()).collect();

    println!();
    println!(
        "Select files for the {} (the rest stays in the original):",
        style("new commit (inserted BEFORE the original in the stack)").bold()
    );

    let selections = MultiSelect::new()
        .items(&items)
        .interact()
        .map_err(|e| GgError::Other(format!("Selection failed: {}", e)))?;

    if selections.is_empty() {
        return Ok(vec![]);
    }

    Ok(selections
        .iter()
        .map(|&i| changed_files[i].path.clone())
        .collect())
}

/// Validate that CLI-provided file paths match changed files
fn validate_file_selection(files: &[String], changed_files: &[ChangedFile]) -> Result<Vec<String>> {
    let mut selected = Vec::new();
    for file in files {
        let found = changed_files.iter().any(|cf| cf.path == *file);
        if !found {
            return Err(GgError::Other(format!(
                "File '{}' is not in the commit's changed files",
                file
            )));
        }
        selected.push(file.clone());
    }
    Ok(selected)
}

/// Build a tree that has the parent tree as base, with selected files replaced from target tree
fn build_partial_tree<'a>(
    repo: &'a git2::Repository,
    parent_tree: &git2::Tree,
    target_tree: &git2::Tree,
    selected_files: &[String],
) -> Result<git2::Tree<'a>> {
    // We need to build a tree that contains:
    // - All files from parent_tree EXCEPT selected files
    // - Selected files from target_tree
    //
    // Since we want the FIRST commit to contain the SELECTED changes,
    // we start with parent and add/modify the selected files from target.

    let mut builder = repo.treebuilder(Some(parent_tree))?;

    for file_path in selected_files {
        let path = std::path::Path::new(file_path);

        // Check if the file exists in target (added or modified)
        if let Ok(entry) = target_tree.get_path(path) {
            // File exists in target - add/update it
            if path.parent().is_some() && path.parent() != Some(std::path::Path::new("")) {
                // Nested path - need to handle directory structure
                insert_nested_entry(repo, &mut builder, parent_tree, target_tree, file_path)?;
            } else {
                // Top-level file
                let name = path.file_name().unwrap().to_string_lossy();
                builder.insert(&*name, entry.id(), entry.filemode())?;
            }
        } else {
            // File doesn't exist in target - it was deleted
            // For the first commit, we want to include the deletion
            let name = path.file_name().unwrap().to_string_lossy();
            if path.parent().is_none() || path.parent() == Some(std::path::Path::new("")) {
                let _ = builder.remove(&*name);
            } else {
                insert_nested_entry(repo, &mut builder, parent_tree, target_tree, file_path)?;
            }
        }
    }

    let tree_oid = builder.write()?;
    let tree = repo.find_tree(tree_oid)?;
    Ok(tree)
}

/// Insert a nested file entry by reconstructing intermediate directory trees
fn insert_nested_entry(
    repo: &git2::Repository,
    root_builder: &mut git2::TreeBuilder,
    parent_tree: &git2::Tree,
    target_tree: &git2::Tree,
    file_path: &str,
) -> Result<()> {
    let parts: Vec<&str> = file_path.split('/').collect();

    // Get the target entry (or None if deleted)
    let target_entry = target_tree.get_path(std::path::Path::new(file_path)).ok();

    // Recursively rebuild the tree hierarchy from the top directory down
    let new_subtree_oid = rebuild_subtree(repo, parent_tree, &parts, 0, target_entry.as_ref())?;

    // Update the root builder with the new subtree
    root_builder
        .insert(parts[0], new_subtree_oid, 0o040000)
        .map_err(|e| GgError::Other(format!("Failed to update root tree: {}", e)))?;

    Ok(())
}

/// Recursively rebuild a subtree to include a changed file
fn rebuild_subtree(
    repo: &git2::Repository,
    parent_tree: &git2::Tree,
    parts: &[&str],
    depth: usize,
    target_entry: Option<&git2::TreeEntry>,
) -> std::result::Result<git2::Oid, GgError> {
    // Get the current subtree from parent at this depth
    let subpath = parts[..=depth].join("/");
    let current_subtree = if depth == 0 {
        parent_tree
            .get_name(parts[0])
            .and_then(|e| repo.find_tree(e.id()).ok())
    } else {
        parent_tree
            .get_path(std::path::Path::new(&subpath))
            .ok()
            .and_then(|e| repo.find_tree(e.id()).ok())
    };

    let mut builder = if let Some(ref tree) = current_subtree {
        repo.treebuilder(Some(tree))
            .map_err(|e| GgError::Other(format!("Failed to create tree builder: {}", e)))?
    } else {
        repo.treebuilder(None)
            .map_err(|e| GgError::Other(format!("Failed to create tree builder: {}", e)))?
    };

    if depth == parts.len() - 2 {
        // This is the parent directory of the file
        let filename = parts[parts.len() - 1];
        if let Some(entry) = target_entry {
            builder
                .insert(filename, entry.id(), entry.filemode())
                .map_err(|e| GgError::Other(format!("Failed to insert entry: {}", e)))?;
        } else {
            // File was deleted
            let _ = builder.remove(filename);
        }
    } else {
        // Intermediate directory - recurse
        let child_oid = rebuild_subtree(repo, parent_tree, parts, depth + 1, target_entry)?;
        builder
            .insert(parts[depth + 1], child_oid, 0o040000)
            .map_err(|e| GgError::Other(format!("Failed to insert subtree: {}", e)))?;
    }

    builder
        .write()
        .map_err(|e| GgError::Other(format!("Failed to write tree: {}", e)))
}

/// Get the commit message for the new (first/lower) commit
fn get_new_commit_message(options: &SplitOptions, target: &git2::Commit) -> Result<String> {
    if let Some(msg) = &options.message {
        return Ok(msg.clone());
    }

    let default_msg = format!("Split from: {}", git::get_commit_title(target));

    let edited = Editor::new()
        .extension(".txt")
        .edit(&default_msg)
        .map_err(|e| GgError::Other(format!("Editor failed: {}", e)))?;

    match edited {
        Some(msg) if !msg.trim().is_empty() => Ok(msg.trim().to_string()),
        _ => Err(GgError::Other(
            "Empty commit message, aborting split".to_string(),
        )),
    }
}

/// Get the commit message for the remainder (second/upper) commit
fn get_remainder_message(options: &SplitOptions, target: &git2::Commit) -> Result<String> {
    let original_msg = git::strip_gg_id_from_message(target.message().unwrap_or(""));

    if options.no_edit {
        return Ok(original_msg);
    }

    let edited = Editor::new()
        .extension(".txt")
        .edit(&original_msg)
        .map_err(|e| GgError::Other(format!("Editor failed: {}", e)))?;

    match edited {
        Some(msg) if !msg.trim().is_empty() => Ok(msg.trim().to_string()),
        _ => Ok(original_msg),
    }
}

/// Rebase descendants of the target commit onto the new second commit.
/// Returns the number of rebased commits.
fn rebase_descendants(
    repo: &git2::Repository,
    stack: &Stack,
    config: &Config,
    target_pos: usize,
    original_commit: &git2::Commit,
    second_commit: &git2::Commit,
) -> Result<usize> {
    let remaining = stack.len() - target_pos;
    if remaining == 0 {
        // Target was the stack head — just update branch pointer
        update_branch_after_split(repo, stack, second_commit)?;
        return Ok(0);
    }

    // Use git rebase --onto <second_commit> <original_commit> <branch>
    let branch_name = stack.branch_name();
    let output = Command::new("git")
        .args([
            "rebase",
            "--onto",
            &second_commit.id().to_string(),
            &original_commit.id().to_string(),
            &branch_name,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("CONFLICT") || stderr.contains("conflict") {
            // Abort the rebase to leave repo in clean state
            let _ = Command::new("git").args(["rebase", "--abort"]).output();
            return Err(GgError::Other(
                "Split aborted: merge conflict during descendant rebase. \
                 The original commit has been left unchanged."
                    .to_string(),
            ));
        }
        return Err(GgError::Other(format!("Rebase failed: {}", stderr)));
    }

    // Re-attach HEAD if needed
    git::ensure_branch_attached(repo, &branch_name)?;

    // Navigate back to the position of the remainder commit (target_pos + 1 in new stack)
    let new_stack = Stack::load(repo, config)?;
    let new_pos = target_pos; // The remainder is at target_pos + 1 in the new (larger) stack
    if let Some(entry) = new_stack.get_entry_by_position(new_pos + 1) {
        let git_dir = repo.path();
        stack::save_nav_context(git_dir, &branch_name, entry.position - 1, entry.oid)?;
        let commit = repo.find_commit(entry.oid)?;
        git::checkout_commit(repo, &commit)?;
    }

    Ok(remaining)
}

/// Update branch pointer when splitting the stack head
fn update_branch_after_split(
    repo: &git2::Repository,
    stack: &Stack,
    new_head: &git2::Commit,
) -> Result<()> {
    let branch_name = stack.branch_name();

    // Use git reset --hard to move HEAD and branch pointer together
    // This works even when we're on the branch
    let output = Command::new("git")
        .args(["reset", "--hard", &new_head.id().to_string()])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to update branch: {}",
            stderr
        )));
    }

    // Ensure we're still on the branch (in case we got detached)
    git::ensure_branch_attached(repo, &branch_name)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_options_default() {
        let opts = SplitOptions::default();
        assert!(opts.target.is_none());
        assert!(opts.files.is_empty());
        assert!(opts.message.is_none());
        assert!(!opts.no_edit);
    }
}
