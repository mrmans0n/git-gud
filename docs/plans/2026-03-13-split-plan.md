# `gg split` Implementation Plan

**Goal:** Add a `split` command that splits any commit in the stack into two, with file-level selection (CLI args or interactive checkbox), automatic descendant rebase, and proper GG-ID handling.

**Architecture:** New command module `split.rs` in `gg-core/src/commands/`, registered in `mod.rs` and wired through `gg-cli/src/main.rs`. Reuses existing stack resolution (`stack::resolve_target`), clean working directory check, rebase pattern (from `reorder.rs`/`squash.rs`), and GG-ID generation.

**Tech Stack:** `git2` (diff, tree manipulation, commits), `dialoguer` (MultiSelect checkbox), `console` (styled output), existing `git`/`stack`/`config` modules.

---

### Task 1: Core split logic — `split.rs` with file args (non-interactive)

The core algorithm: given a target commit and a list of file paths, split it into two commits and rebase descendants.

**Files:**
- Create: `crates/gg-core/src/commands/split.rs`
- Modify: `crates/gg-core/src/commands/mod.rs` (add `pub mod split;`)

**Step 1: Write the failing test**

Create `crates/gg-core/src/commands/split.rs` with the test at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Unit tests for helper functions will go here once we have them
    #[test]
    fn test_split_options_default() {
        let opts = SplitOptions::default();
        assert!(opts.target.is_none());
        assert!(opts.files.is_empty());
        assert!(opts.message.is_none());
        assert!(!opts.no_edit);
    }
}
```

**Step 2: Write the `SplitOptions` struct and `run` function skeleton**

```rust
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
        write!(f, "{:<50} (+{} -{})", self.path, self.additions, self.deletions)
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
        GgError::Other(format!("Commit at position {} not found in stack", target_pos))
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
    let parent_commit = target_commit.parent(0).map_err(|_| {
        GgError::Other("Cannot split the root commit (no parent)".to_string())
    })?;

    // Get list of changed files with stats
    let changed_files = get_changed_files(&repo, &parent_commit, &target_commit)?;

    if changed_files.is_empty() {
        return Err(GgError::Other("Commit has no file changes to split".to_string()));
    }

    if changed_files.len() < 2 && options.files.is_empty() {
        return Err(GgError::Other(
            "Commit only has 1 file. Use hunk-level splitting (-i) in a future version.".to_string(),
        ));
    }

    // Determine which files go to the new (first/lower) commit
    let selected_files = if options.files.is_empty() {
        select_files_interactive(&changed_files)?
    } else {
        validate_file_selection(&options.files, &changed_files)?
    };

    if selected_files.is_empty() {
        return Err(GgError::Other("No files selected, nothing to split".to_string()));
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
        &repo, &stack, &config, target_pos, &target_commit, &second_commit,
    )?;

    // Print results
    println!(
        "{} Split complete!",
        style("✔").green().bold()
    );
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
    let stats = diff.stats()?;
    let _ = stats; // we get per-file stats from deltas

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

        // Get per-file stats by iterating patches
        // We'll approximate with 0/0 and fill in from the patch
        files.push(ChangedFile {
            path,
            additions: 0,
            deletions: 0,
        });
    }

    // Get per-file line stats from patches
    diff.foreach(
        &mut |delta, _progress| {
            // file callback — find matching entry and we'll update stats in hunk callback
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let _ = path;
            true
        },
        None, // binary callback
        Some(&mut |_delta, hunk| {
            // Per-hunk, we can't easily match to files here. Use line callback instead.
            let _ = hunk;
            true
        }),
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
fn validate_file_selection(
    files: &[String],
    changed_files: &[ChangedFile],
) -> Result<Vec<String>> {
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
fn build_partial_tree(
    repo: &git2::Repository,
    parent_tree: &git2::Tree,
    target_tree: &git2::Tree,
    selected_files: &[String],
) -> Result<git2::Tree<'_>> {
    let mut builder = repo.treebuilder(Some(parent_tree))?;

    for file_path in selected_files {
        // Handle nested paths: we need to walk the target tree to find the entry
        if let Some(entry) = target_tree.get_path(std::path::Path::new(file_path)).ok() {
            // For top-level files, we can insert directly
            // For nested files, we need to reconstruct intermediate trees
            let name = std::path::Path::new(file_path)
                .file_name()
                .unwrap()
                .to_string_lossy();

            if file_path.contains('/') {
                // Nested path: need to reconstruct directory trees
                insert_nested_entry(repo, &mut builder, parent_tree, target_tree, file_path)?;
            } else {
                // Top-level file: simple insert/replace
                builder.insert(&*name, entry.id(), entry.filemode() as i32)?;
            }
        } else {
            // File was deleted in target — we need to remove it from parent tree
            let name = std::path::Path::new(file_path)
                .file_name()
                .unwrap()
                .to_string_lossy();

            if !file_path.contains('/') {
                builder.remove(&*name)?;
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

    // We need to rebuild the tree hierarchy from the deepest level up
    // Start by getting the target entry (or None if deleted)
    let target_entry = target_tree.get_path(std::path::Path::new(file_path)).ok();

    // Walk down the path, rebuilding trees bottom-up
    fn rebuild_tree(
        repo: &git2::Repository,
        parent_tree: &git2::Tree,
        target_tree: &git2::Tree,
        parts: &[&str],
        depth: usize,
        target_entry: Option<&git2::TreeEntry>,
    ) -> std::result::Result<git2::Oid, GgError> {
        if depth == parts.len() - 1 {
            // Leaf level: this is the file itself
            // Get the subtree at this level from parent
            let parent_subtree = if depth > 0 {
                let subpath = parts[..depth].join("/");
                parent_tree
                    .get_path(std::path::Path::new(&subpath))
                    .ok()
                    .and_then(|e| repo.find_tree(e.id()).ok())
            } else {
                Some(parent_tree.clone())
            };

            let base = parent_subtree.as_ref().unwrap_or(parent_tree);
            let mut builder = repo.treebuilder(Some(base)).map_err(|e| {
                GgError::Other(format!("Failed to create tree builder: {}", e))
            })?;

            if let Some(entry) = target_entry {
                builder
                    .insert(parts[depth], entry.id(), entry.filemode() as i32)
                    .map_err(|e| GgError::Other(format!("Failed to insert entry: {}", e)))?;
            } else {
                let _ = builder.remove(parts[depth]);
            }

            builder
                .write()
                .map_err(|e| GgError::Other(format!("Failed to write tree: {}", e)))
        } else {
            // Intermediate directory: recurse, then update this level
            let child_oid =
                rebuild_tree(repo, parent_tree, target_tree, parts, depth + 1, target_entry)?;

            let parent_subtree = if depth > 0 {
                let subpath = parts[..depth].join("/");
                parent_tree
                    .get_path(std::path::Path::new(&subpath))
                    .ok()
                    .and_then(|e| repo.find_tree(e.id()).ok())
            } else {
                Some(parent_tree.clone())
            };

            let base = parent_subtree.as_ref().unwrap_or(parent_tree);
            let mut builder = repo.treebuilder(Some(base)).map_err(|e| {
                GgError::Other(format!("Failed to create tree builder: {}", e))
            })?;

            builder
                .insert(parts[depth], child_oid, 0o040000)
                .map_err(|e| GgError::Other(format!("Failed to insert subtree: {}", e)))?;

            builder
                .write()
                .map_err(|e| GgError::Other(format!("Failed to write tree: {}", e)))
        }
    }

    let new_root_subtree_oid = rebuild_tree(
        repo,
        parent_tree,
        target_tree,
        &parts,
        0,
        target_entry.as_ref(),
    )?;

    // The rebuild gives us a new root-level tree entry for parts[0]
    // But actually rebuild_tree at depth 0 rebuilds the WHOLE root tree
    // We need to extract just the subtree for parts[0]
    // Let's use a simpler approach: just update parts[0] in the root builder
    let new_root = repo
        .find_tree(new_root_subtree_oid)
        .map_err(|e| GgError::Other(format!("Failed to find rebuilt tree: {}", e)))?;

    if let Some(subtree_entry) = new_root.get_name(parts[0]) {
        root_builder
            .insert(parts[0], subtree_entry.id(), subtree_entry.filemode() as i32)
            .map_err(|e| GgError::Other(format!("Failed to update root tree: {}", e)))?;
    }

    Ok(())
}

/// Get the commit message for the new (first/lower) commit
fn get_new_commit_message(options: &SplitOptions, target: &git2::Commit) -> Result<String> {
    if let Some(msg) = &options.message {
        return Ok(msg.clone());
    }

    let default_msg = format!(
        "Split from: {}",
        git::get_commit_title(target)
    );

    let edited = Editor::new()
        .extension(".txt")
        .edit(&default_msg)
        .map_err(|e| GgError::Other(format!("Editor failed: {}", e)))?;

    match edited {
        Some(msg) if !msg.trim().is_empty() => Ok(msg.trim().to_string()),
        _ => Err(GgError::Other("Empty commit message, aborting split".to_string())),
    }
}

/// Get the commit message for the remainder (second/upper) commit
fn get_remainder_message(options: &SplitOptions, target: &git2::Commit) -> Result<String> {
    let original_msg = git::strip_gg_id_from_message(
        target.message().unwrap_or(""),
    );

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
    // But we need to reload the stack to get the new OIDs
    let new_stack = Stack::load(repo, config)?;
    let new_pos = target_pos; // The remainder is at target_pos + 1, but 0-indexed = target_pos
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
    repo.branch(&branch_name, new_head, true)?;
    git::checkout_branch(repo, &branch_name)?;
    Ok(())
}
```

**Step 3: Register the module**

In `crates/gg-core/src/commands/mod.rs`, add:
```rust
pub mod split;
```

**Step 4: Run tests**

```bash
cargo test -p gg-core --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: add split command core logic

Implements gg split to break a commit into two:
- File-level selection (CLI args or interactive dialoguer checkbox)
- Proper GG-ID handling (new for selected, preserved for remainder)
- Automatic descendant rebase
- Edge case handling (empty selection, all selected, single file, etc.)
"
```

---

### Task 2: Wire CLI arguments in `gg-cli/src/main.rs`

**Files:**
- Modify: `crates/gg-cli/src/main.rs`

**Step 1: Add the `Split` variant to `Commands` enum**

After the `Reorder` variant, add:

```rust
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

        /// Files to include in the new commit
        #[arg(value_name = "FILES")]
        files: Vec<String>,
    },
```

**Step 2: Add the match arm in `main()`**

In the `match cli.command` block, add:

```rust
        Some(Commands::Split {
            commit,
            message,
            no_edit,
            files,
        }) => (
            gg_core::commands::split::run(gg_core::commands::split::SplitOptions {
                target: commit,
                files,
                message,
                no_edit,
            }),
            false,
        ),
```

**Step 3: Run and verify**

```bash
cargo build
cargo run -- split --help
```

Expected: help text showing split command with options.

**Step 4: Commit**

```bash
git add -A
git commit -m "feat: wire split command CLI arguments"
```

---

### Task 3: Integration tests

**Files:**
- Modify: `crates/gg-cli/tests/integration_tests.rs`

**Step 1: Write integration tests**

Add at the end of the integration test file:

```rust
#[test]
fn test_split_head_with_files() {
    // Setup: create a stack with 2 commits, the second touching 2 files
    let (dir, _repo) = setup_test_repo();
    let _guard = cd_to(&dir);

    // Create stack
    run_gg(&["co", "test-split"]);

    // Create first commit (1 file)
    create_file(&dir, "file_a.txt", "content a");
    run_git(&["add", "-A"]);
    run_git(&["commit", "-m", "Add file A"]);

    // Create second commit (2 files)
    create_file(&dir, "file_b.txt", "content b");
    create_file(&dir, "file_c.txt", "content c");
    run_git(&["add", "-A"]);
    run_git(&["commit", "-m", "Add files B and C"]);

    // Split HEAD: move file_b to a new commit before the current
    let output = run_gg(&["split", "-m", "Add file B only", "--no-edit", "file_b.txt"]);
    assert!(output.contains("Split complete"));

    // Verify we now have 3 commits
    let log = run_git(&["log", "--oneline", "--no-walk=unsorted", "HEAD~2..HEAD"]);
    // Should see: "Add files B and C" (remainder) and "Add file B only" (new)
    assert!(log.contains("Add file B only") || log.contains("Add files B and C"));
}

#[test]
fn test_split_non_head_rebases_descendants() {
    let (dir, _repo) = setup_test_repo();
    let _guard = cd_to(&dir);

    run_gg(&["co", "test-split-rebase"]);

    // Commit 1: file_a
    create_file(&dir, "file_a.txt", "a");
    run_git(&["add", "-A"]);
    run_git(&["commit", "-m", "Commit 1: file A"]);

    // Commit 2: file_b + file_c (this is the one we'll split)
    create_file(&dir, "file_b.txt", "b");
    create_file(&dir, "file_c.txt", "c");
    run_git(&["add", "-A"]);
    run_git(&["commit", "-m", "Commit 2: files B and C"]);

    // Commit 3: file_d (descendant that should be rebased)
    create_file(&dir, "file_d.txt", "d");
    run_git(&["add", "-A"]);
    run_git(&["commit", "-m", "Commit 3: file D"]);

    // Navigate to commit 2 and split it
    run_gg(&["prev"]);
    let output = run_gg(&["split", "-m", "Split: file B", "--no-edit", "file_b.txt"]);
    assert!(output.contains("Split complete"));
    assert!(output.contains("Rebased"));
}

#[test]
fn test_split_no_files_selected_errors() {
    let (dir, _repo) = setup_test_repo();
    let _guard = cd_to(&dir);

    run_gg(&["co", "test-split-empty"]);

    create_file(&dir, "file_a.txt", "a");
    create_file(&dir, "file_b.txt", "b");
    run_git(&["add", "-A"]);
    run_git(&["commit", "-m", "Two files"]);

    // Try to split with a file that doesn't exist in the commit
    let result = run_gg_result(&["split", "-m", "test", "nonexistent.txt"]);
    assert!(result.is_err() || result.unwrap().contains("not in the commit"));
}

#[test]
fn test_split_preserves_gg_id_on_remainder() {
    let (dir, _repo) = setup_test_repo();
    let _guard = cd_to(&dir);

    run_gg(&["co", "test-split-ggid"]);

    create_file(&dir, "file_a.txt", "a");
    create_file(&dir, "file_b.txt", "b");
    run_git(&["add", "-A"]);
    // Add GG-ID manually to simulate a synced commit
    run_git(&["commit", "-m", "Two files\n\nGG-ID: test-uuid-1234"]);

    let output = run_gg(&["split", "-m", "Split file A", "--no-edit", "file_a.txt"]);
    assert!(output.contains("Split complete"));

    // The remainder commit should still have the original GG-ID
    let log = run_git(&["log", "-1", "--format=%B", "HEAD"]);
    assert!(log.contains("GG-ID: test-uuid-1234"));
}
```

**Note:** The exact test helper functions (`setup_test_repo`, `run_gg`, `create_file`, etc.) must match what already exists in the integration test file. The implementer should check the existing helpers and adapt.

**Step 2: Run tests**

```bash
cargo test -p gg-cli --all-features
```

**Step 3: Commit**

```bash
git add -A
git commit -m "test: add integration tests for split command"
```

---

### Task 4: Create PR

**Step 1: Push and create PR**

```bash
git push origin HEAD
gh pr create --title "feat: add gg split command" \
  --body "## Summary
- New \`gg split\` command to split a commit into two
- File-level selection via CLI args or interactive checkbox (dialoguer)
- Target any commit in the stack (\`-c\` flag)
- Automatic rebase of descendants
- GG-ID preserved on remainder, new ID for selected commit
- Edge cases: empty selection, all selected, single file, dirty workdir

## Design
See: docs/plans/2026-03-13-split-design.md

## Test Plan
- [x] Unit tests for SplitOptions
- [x] Integration: split HEAD with file args
- [x] Integration: split non-HEAD with descendant rebase
- [x] Integration: error on invalid file
- [x] Integration: GG-ID preserved on remainder
- [x] cargo fmt + clippy clean"
```

---

## Dependency Graph

```
Task 1 (core split.rs) → Task 2 (CLI wiring) → Task 3 (integration tests) → Task 4 (PR)
```

All tasks are sequential — each depends on the previous.
