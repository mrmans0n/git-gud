//! `gg split` - Split a commit into two

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use console::{style, Term};
use dialoguer::Editor;
use serde_json::json;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::immutability::{self, ImmutabilityPolicy};
use crate::operations::{self, OperationKind, SnapshotScope};
use crate::output;
use crate::stack::{self, Stack};

use super::split_protocol::{
    describe_hunk, plan_token, read_plan, SplitApplyResponse, SplitApplyResult,
    SplitCommitIdentity, SplitDescribeResponse, SplitTargetIdentity, SPLIT_PROTOCOL_VERSION,
};
pub use super::split_protocol::{DiffHunk, DiffLine};

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
    /// If true, disable TUI and use sequential prompt instead.
    pub no_tui: bool,
    /// If true, override the immutability check.
    pub force: bool,
    /// If true, describe the target commit's hunks without changing the repository.
    pub describe: bool,
    /// Apply a previously described structured split plan.
    pub plan_json: Option<PathBuf>,
    /// If true, emit machine-readable JSON output.
    pub json: bool,
}

struct ResolvedSplitTarget<'repo> {
    stack: Stack,
    target_pos: usize,
    target_commit: git2::Commit<'repo>,
    parent_commit: git2::Commit<'repo>,
    original_gg_id: Option<String>,
}

struct SplitSelection {
    selected_indices: Vec<usize>,
    non_hunk_files: Vec<String>,
    first_message: String,
    remainder_message: String,
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

#[derive(Debug, Clone, Copy)]
struct TargetDiffFile {
    oid: git2::Oid,
    mode: i32,
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
    if options.describe {
        let response = describe(&options)?;
        output::print_json(&response);
        return Ok(());
    }
    if let Some(path) = options.plan_json.as_deref() {
        let response = apply_plan(&options, path)?;
        output::print_json(&response);
        return Ok(());
    }

    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;
    // Acquire the operation lock up-front but defer writing the op-log
    // record until after all validation (including the immutability guard)
    // so refused operations never leak into `gg undo --list` (design §4.6).
    let _lock = git::acquire_operation_lock(&repo, "split")?;

    git::require_clean_working_directory(&repo)?;

    let ResolvedSplitTarget {
        stack,
        target_pos,
        target_commit,
        parent_commit,
        original_gg_id,
    } = resolve_target(&repo, &config, options.target.as_deref(), false)?;

    // Immutability pre-flight: splitting rewrites the target commit and every
    // commit above it (they get a new parent). Guard against splitting a
    // merged or base-ancestor commit unless the user explicitly overrides.
    {
        let targets: Vec<usize> = (target_pos..=stack.len()).collect();
        let policy = ImmutabilityPolicy::for_stack(&repo, &stack)?;
        let report = policy.check_positions(&stack, &targets);
        immutability::guard(report, options.force)?;
    }

    println!(
        "Splitting commit {}: {} ({})",
        target_pos,
        style(git::get_commit_title(&target_commit)).bold(),
        style(git::short_sha(&target_commit)).yellow()
    );

    // Get list of changed files with stats
    let changed_files = get_changed_files(&repo, &parent_commit, &target_commit)?;

    if changed_files.is_empty() {
        return Err(GgError::Other(
            "Commit has no file changes to split".to_string(),
        ));
    }

    // If the TUI provides commit messages inline, they're stored here to skip the editor
    let mut tui_commit_message: Option<String> = None;
    let mut tui_remainder_message: Option<String> = None;

    // === Hunk-level splitting (always) ===
    let mut hunks = get_hunks(&repo, &parent_commit, &target_commit)?;

    // Filter hunks to specified files if any.
    // Track files that have no textual hunks (binary, rename-only, mode-only)
    // so they can be included wholesale from the target tree.
    let total_hunks_before_filter = hunks.len();
    let mut non_hunk_files: Vec<String> = Vec::new();
    if !options.files.is_empty() {
        validate_file_selection(&options.files, &changed_files)?;
        let hunk_file_paths: std::collections::HashSet<&str> =
            hunks.iter().map(|h| h.file_path.as_str()).collect();
        for file in &options.files {
            if !hunk_file_paths.contains(file.as_str()) {
                non_hunk_files.push(file.clone());
            }
        }
        hunks.retain(|h| options.files.contains(&h.file_path));
    }

    if hunks.is_empty() && non_hunk_files.is_empty() {
        // If there are changed files but no textual hunks and no FILES specified,
        // the commit only has non-textual changes. Guide the user to the file-args path.
        if options.files.is_empty() && !changed_files.is_empty() {
            let file_list: Vec<&str> = changed_files.iter().map(|f| f.path.as_str()).collect();
            return Err(GgError::Other(format!(
                "No textual hunks to split. The commit only contains non-textual changes \
                 (binary, mode-only, or renames).\n\
                 To split by file, specify files explicitly: gg split {}",
                file_list.join(" ")
            )));
        }
        return Err(GgError::Other("No hunks found to split".to_string()));
    }

    let selected_indices = if !options.files.is_empty() {
        // File args provided — auto-select all hunks from those files
        (0..hunks.len()).collect()
    } else {
        // No file args — interactive hunk selection
        let is_tty = atty::is(atty::Stream::Stdin) && atty::is(atty::Stream::Stdout);
        let use_tui = !options.no_tui && is_tty;

        if use_tui {
            let commit_title = git::get_commit_title(&target_commit);
            let original_msg = git::strip_gg_id_from_message(target_commit.message().unwrap_or(""));
            match super::split_tui::select_hunks_tui(
                hunks.clone(),
                &commit_title,
                &original_msg,
                options.no_edit,
            )? {
                Some(result) => {
                    tui_commit_message = Some(result.commit_message);
                    tui_remainder_message = result.remainder_message;
                    result.selected_indices
                }
                None => {
                    // User aborted
                    return Err(GgError::Other("Selection aborted".to_string()));
                }
            }
        } else {
            select_hunks_interactive(&mut hunks)?
        }
    };

    if selected_indices.is_empty() && non_hunk_files.is_empty() {
        return Err(GgError::Other(
            "No hunks selected, nothing to split".to_string(),
        ));
    }

    let all_selected = selected_indices.len() == total_hunks_before_filter
        && non_hunk_files.len() + options.files.len() >= changed_files.len();
    if all_selected {
        println!(
            "{}",
            style("⚠ All hunks selected — the original commit will become empty.").yellow()
        );
    }

    // Get commit messages
    // Priority: -m flag > TUI inline message > editor prompt
    let new_commit_message = if options.message.is_some() {
        get_new_commit_message(&options, &target_commit)?
    } else if let Some(msg) = tui_commit_message {
        msg
    } else {
        get_new_commit_message(&options, &target_commit)?
    };
    // Priority for remainder message: TUI inline > editor prompt
    let remainder_message = if let Some(msg) = tui_remainder_message {
        msg
    } else {
        get_remainder_message(&options, &target_commit)?
    };

    let result = execute_split(
        &repo,
        &config,
        ResolvedSplitTarget {
            stack,
            target_pos,
            target_commit,
            parent_commit,
            original_gg_id,
        },
        &hunks,
        SplitSelection {
            selected_indices,
            non_hunk_files,
            first_message: new_commit_message,
            remainder_message,
        },
        std::env::args().skip(1).collect(),
    )?;

    let first_commit = repo.find_commit(git2::Oid::from_str(&result.first.sha)?)?;
    let second_commit = repo.find_commit(git2::Oid::from_str(&result.remainder.sha)?)?;
    let num_rebased = result.rewritten_descendants.len();

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

/// Apply a versioned split plan after validating it against current repository state.
pub fn apply_plan(_options: &SplitOptions, path: &Path) -> Result<SplitApplyResponse> {
    let plan = read_plan(path)?;
    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;
    let _lock = git::acquire_operation_lock(&repo, "split")?;

    git::require_clean_working_directory(&repo)?;

    let target_selector = plan.target.gg_id.as_deref().unwrap_or(&plan.target.sha);
    let resolved = match resolve_target(&repo, &config, Some(target_selector), true) {
        Ok(resolved) => resolved,
        Err(GgError::Other(message))
            if message
                == format!(
                    "Could not find commit matching '{}' in stack",
                    target_selector
                ) =>
        {
            return Err(stale_split_plan("target identity changed"));
        }
        Err(error) => return Err(error),
    };
    let current_target = SplitTargetIdentity {
        gg_id: resolved.original_gg_id.clone(),
        sha: resolved.target_commit.id().to_string(),
        tree: resolved.target_commit.tree_id().to_string(),
    };
    if current_target != plan.target {
        return Err(stale_split_plan("target identity changed"));
    }

    let hunks = get_hunks(&repo, &resolved.parent_commit, &resolved.target_commit)?;
    let described_hunks = hunks
        .iter()
        .enumerate()
        .map(|(index, hunk)| describe_hunk(index, hunk))
        .collect::<Vec<_>>();
    if plan_token(&current_target, &described_hunks) != plan.plan_token {
        return Err(stale_split_plan("hunks changed"));
    }

    let mut indices_by_id: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, hunk) in described_hunks.iter().enumerate() {
        indices_by_id.entry(&hunk.id).or_default().push(index);
    }
    let mut selected_indices = Vec::with_capacity(plan.selected_hunk_ids.len());
    for id in &plan.selected_hunk_ids {
        match indices_by_id.get(id.as_str()).map(Vec::as_slice) {
            Some([index]) => selected_indices.push(*index),
            Some(_) => {
                return Err(GgError::Other(format!(
                    "Split plan hunk ID '{id}' is not unique"
                )))
            }
            None => {
                return Err(GgError::Other(format!(
                    "Split plan contains unknown hunk ID '{id}'"
                )))
            }
        }
    }

    if selected_indices.is_empty() {
        return Err(GgError::Other(
            "Split plan must select at least one hunk".into(),
        ));
    }
    let changed_files = get_changed_files(&repo, &resolved.parent_commit, &resolved.target_commit)?;
    let hunk_files = hunks
        .iter()
        .map(|hunk| hunk.file_path.as_str())
        .collect::<HashSet<_>>();
    let parent_tree = resolved.parent_commit.tree()?;
    let target_tree = resolved.target_commit.tree()?;
    let has_remainder_changes = changed_files
        .iter()
        .any(|file| !hunk_files.contains(file.path.as_str()))
        || hunk_files
            .iter()
            .any(|path| has_regular_file_mode_change(&parent_tree, &target_tree, path));
    if selected_indices.len() == described_hunks.len() && !has_remainder_changes {
        return Err(GgError::Other(
            "No changes would remain in the remainder commit".into(),
        ));
    }

    let first_message = plan.first_message.trim().to_string();
    let remainder_message = plan.remainder_message.trim().to_string();
    if first_message.is_empty() || remainder_message.is_empty() {
        return Err(GgError::Other(
            "Split plan commit messages must not be empty".into(),
        ));
    }

    let selected_paths = selected_indices
        .iter()
        .map(|index| hunks[*index].file_path.as_str())
        .collect::<HashSet<_>>();
    validate_path_dependent_selection(&target_tree, &changed_files, &selected_paths)?;

    let result = execute_split(
        &repo,
        &config,
        resolved,
        &hunks,
        SplitSelection {
            selected_indices,
            // Structured plans select textual hunks only. All non-textual files
            // therefore remain in the upper commit.
            non_hunk_files: vec![],
            first_message,
            remainder_message,
        },
        std::env::args().skip(1).collect(),
    )?;

    Ok(SplitApplyResponse {
        version: SPLIT_PROTOCOL_VERSION,
        result,
    })
}

fn stale_split_plan(reason: &str) -> GgError {
    GgError::Other(format!("stale split plan: {reason}"))
}

fn validate_path_dependent_selection(
    target_tree: &git2::Tree,
    changed_files: &[ChangedFile],
    selected_paths: &HashSet<&str>,
) -> Result<()> {
    for (index, first) in changed_files.iter().enumerate() {
        for second in &changed_files[index + 1..] {
            if !paths_have_prefix_dependency(&first.path, &second.path) {
                continue;
            }

            let first_selected = selected_paths.contains(first.path.as_str());
            let second_selected = selected_paths.contains(second.path.as_str());
            if first_selected == second_selected {
                continue;
            }

            let (selected, dependent) = if first_selected {
                (&first.path, &second.path)
            } else {
                (&second.path, &first.path)
            };
            let selected_is_target_leaf = target_tree
                .get_path(Path::new(selected))
                .ok()
                .is_some_and(|entry| entry.kind() != Some(git2::ObjectType::Tree));
            if selected_is_target_leaf {
                return Err(GgError::Other(format!(
                    "path-dependent split selection: selecting '{selected}' would also apply the unselected change '{dependent}'; select both sides together or leave '{selected}' in the remainder"
                )));
            }
        }
    }
    Ok(())
}

fn paths_have_prefix_dependency(first: &str, second: &str) -> bool {
    second
        .strip_prefix(first)
        .is_some_and(|suffix| suffix.starts_with('/'))
        || first
            .strip_prefix(second)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn execute_split(
    repo: &git2::Repository,
    config: &Config,
    resolved: ResolvedSplitTarget<'_>,
    hunks: &[DiffHunk],
    selection: SplitSelection,
    command_args: Vec<String>,
) -> Result<SplitApplyResult> {
    let ResolvedSplitTarget {
        stack,
        target_pos,
        target_commit,
        parent_commit,
        original_gg_id,
    } = resolved;
    let original_sha = target_commit.id().to_string();
    let parent_tree = parent_commit.tree()?;
    let target_tree = target_commit.tree()?;
    let first_tree = build_tree_from_hunks(
        repo,
        &parent_tree,
        &target_tree,
        hunks,
        &selection.selected_indices,
        &selection.non_hunk_files,
    )?;

    // All validation passed. From here on, failures retain an interrupted
    // operation marker so partial mutations are never offered as undoable.
    let mut guard = git::begin_recorded_op(
        repo,
        config,
        OperationKind::Split,
        command_args,
        None,
        SnapshotScope::AllUserBranches,
    )?;
    guard.set_pending_plan(json!({
        "split": {
            "branch_name": stack.branch_name(),
            "remainder_position": target_pos + 1,
            "remainder_gg_id": original_gg_id.clone(),
        }
    }));
    let operation_id = guard.id().to_owned();

    let sig = git::get_signature(repo)?;
    let new_gg_id = git::generate_gg_id();
    let first_message = git::set_gg_id_in_message(&selection.first_message, &new_gg_id);
    let first_oid = repo.commit(
        None,
        &sig,
        &sig,
        &first_message,
        &first_tree,
        &[&parent_commit],
    )?;
    let first_commit = repo.find_commit(first_oid)?;

    let remainder_message = if let Some(gg_id) = &original_gg_id {
        git::set_gg_id_in_message(&selection.remainder_message, gg_id)
    } else {
        selection.remainder_message
    };
    let second_oid = repo.commit(
        None,
        &sig,
        &sig,
        &remainder_message,
        &target_tree,
        &[&first_commit],
    )?;
    let second_commit = repo.find_commit(second_oid)?;

    if let Err(error) = rebase_descendants(
        repo,
        &stack,
        config,
        target_pos,
        &target_commit,
        &second_commit,
    ) {
        if matches!(error, GgError::RebaseConflict) {
            let _ = operations::remember_interrupted_rebase_operation(repo, guard.id());
        }
        return Err(error);
    }

    let rewritten_stack = Stack::load(repo, config)?;
    let first = split_identity_at(&rewritten_stack, target_pos)?;
    let remainder = split_identity_at(&rewritten_stack, target_pos + 1)?;
    let rewritten_descendants = ((target_pos + 2)..=rewritten_stack.len())
        .map(|position| split_identity_at(&rewritten_stack, position))
        .collect::<Result<Vec<_>>>()?;

    guard.finalize_with_scope(repo, config, SnapshotScope::AllUserBranches, vec![], false)?;

    Ok(SplitApplyResult {
        operation_id,
        original_sha,
        first,
        remainder,
        rewritten_descendants,
    })
}

fn split_identity_at(stack: &Stack, position: usize) -> Result<SplitCommitIdentity> {
    let entry = stack.get_entry_by_position(position).ok_or_else(|| {
        GgError::Other(format!(
            "Split result commit at position {position} was not found"
        ))
    })?;
    Ok(SplitCommitIdentity {
        sha: entry.oid.to_string(),
        gg_id: entry.gg_id.clone(),
    })
}

fn resolve_target<'repo>(
    repo: &'repo git2::Repository,
    config: &Config,
    target: Option<&str>,
    check_immutability: bool,
) -> Result<ResolvedSplitTarget<'repo>> {
    let mut stack = Stack::load(repo, config)?;
    // Best-effort refresh of mr_state for the immutability guard (catches
    // squash-merged PRs that base-ancestor would otherwise miss).
    immutability::refresh_mr_state_for_guard(repo, &mut stack);

    let target_pos = match target {
        Some(target) => stack::resolve_target(&stack, target)?,
        None => stack
            .current_position
            .map(|position| position + 1)
            .unwrap_or(stack.len()),
    };

    let entry = stack.get_entry_by_position(target_pos).ok_or_else(|| {
        GgError::Other(format!(
            "Commit at position {} not found in stack",
            target_pos
        ))
    })?;
    let target_commit = repo.find_commit(entry.oid)?;
    let parent_commit = target_commit
        .parent(0)
        .map_err(|_| GgError::Other("Cannot split the root commit (no parent)".to_string()))?;
    let original_gg_id = entry.gg_id.clone();

    if check_immutability {
        let targets: Vec<usize> = (target_pos..=stack.len()).collect();
        let policy = ImmutabilityPolicy::for_stack(repo, &stack)?;
        let report = policy.check_positions(&stack, &targets);
        immutability::guard(report, false)?;
    }

    Ok(ResolvedSplitTarget {
        stack,
        target_pos,
        target_commit,
        parent_commit,
        original_gg_id,
    })
}

/// Describe a split target without changing repository state.
pub fn describe(options: &SplitOptions) -> Result<SplitDescribeResponse> {
    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;
    let resolved = resolve_target(&repo, &config, options.target.as_deref(), true)?;
    let hunks = get_hunks(&repo, &resolved.parent_commit, &resolved.target_commit)?;
    let described_hunks = hunks
        .iter()
        .enumerate()
        .map(|(index, hunk)| describe_hunk(index, hunk))
        .collect::<Vec<_>>();
    let hunk_files = hunks
        .iter()
        .map(|hunk| hunk.file_path.as_str())
        .collect::<HashSet<_>>();
    let non_textual_files =
        get_changed_files(&repo, &resolved.parent_commit, &resolved.target_commit)?
            .into_iter()
            .filter(|file| !hunk_files.contains(file.path.as_str()))
            .map(|file| file.path)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    let target = SplitTargetIdentity {
        gg_id: resolved.original_gg_id,
        sha: resolved.target_commit.id().to_string(),
        tree: resolved.target_commit.tree_id().to_string(),
    };
    let remainder_message =
        git::strip_gg_id_from_message(resolved.target_commit.message().unwrap_or(""));

    Ok(SplitDescribeResponse {
        version: SPLIT_PROTOCOL_VERSION,
        plan_token: plan_token(&target, &described_hunks),
        target,
        hunks: described_hunks,
        non_textual_files,
        first_message: format!(
            "Split from: {}",
            git::get_commit_title(&resolved.target_commit)
        ),
        remainder_message,
    })
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

/// Insert a nested file entry by reconstructing intermediate directory trees
fn insert_nested_entry(
    repo: &git2::Repository,
    root_builder: &mut git2::TreeBuilder,
    parent_tree: &git2::Tree,
    file_path: &str,
    target_file: TargetDiffFile,
) -> Result<()> {
    let parts: Vec<&str> = file_path.split('/').collect();

    // Recursively rebuild the tree hierarchy from the top directory down
    let new_subtree_oid = rebuild_subtree(repo, parent_tree, &parts, 0, target_file)?;

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
    target_file: TargetDiffFile,
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
        builder
            .insert(filename, target_file.oid, target_file.mode)
            .map_err(|e| GgError::Other(format!("Failed to insert entry: {}", e)))?;
    } else {
        // Intermediate directory - recurse
        let child_oid = rebuild_subtree(repo, parent_tree, parts, depth + 1, target_file)?;
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
        None => Ok(default_msg),
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
        let rewritten_stack = Stack::load(repo, config)?;
        git::normalize_stack_metadata(repo, &rewritten_stack)?;
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
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(rebase_descendants_error(&stdout, &stderr));
    }

    // Re-attach HEAD if needed
    git::ensure_branch_attached(repo, &branch_name)?;

    // Normalize GG metadata while we're still on the branch
    let rewritten_stack = Stack::load(repo, config)?;
    git::normalize_stack_metadata(repo, &rewritten_stack)?;

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

fn rebase_descendants_error(stdout: &str, stderr: &str) -> GgError {
    if stderr.contains("CONFLICT")
        || stderr.contains("conflict")
        || stdout.contains("CONFLICT")
        || stdout.contains("conflict")
    {
        GgError::RebaseConflict
    } else {
        GgError::Other(format!("Rebase failed: {}", stderr))
    }
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

// ============================================================================
// Hunk-level splitting functions
// ============================================================================

/// Extract all diff hunks between parent and target commits
fn get_hunks(
    repo: &git2::Repository,
    parent: &git2::Commit,
    target: &git2::Commit,
) -> Result<Vec<DiffHunk>> {
    let parent_tree = parent.tree()?;
    let target_tree = target.tree()?;

    let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&target_tree), None)?;

    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut current_file_path;
    let mut current_hunk: Option<DiffHunk> = None;

    let mut non_reconstructable_paths = HashSet::new();
    for delta in diff.deltas() {
        let old_file = delta.old_file();
        let new_file = delta.new_file();
        if !diff_file_supports_hunk_reconstruction(repo, &old_file)?
            || !diff_file_supports_hunk_reconstruction(repo, &new_file)?
        {
            let path = new_file
                .path()
                .or_else(|| old_file.path())
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default();
            non_reconstructable_paths.insert(path);
        }
    }

    // We need to iterate patch-by-patch to avoid borrow checker issues with foreach
    let num_deltas = diff.deltas().len();

    for delta_idx in 0..num_deltas {
        let delta = diff
            .get_delta(delta_idx)
            .ok_or_else(|| GgError::Other(format!("Failed to get delta {}", delta_idx)))?;

        let old_file = delta.old_file();
        let new_file = delta.new_file();
        let old_path = old_file.path().map(Path::to_path_buf);
        let new_path = new_file.path().map(Path::to_path_buf);

        current_file_path = new_path
            .as_deref()
            .or(old_path.as_deref())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        if non_reconstructable_paths.contains(&current_file_path) {
            continue;
        }

        // Get the patch for this delta
        let patch = git2::Patch::from_diff(&diff, delta_idx)?;
        if let Some(patch) = patch {
            let num_hunks = patch.num_hunks();

            for hunk_idx in 0..num_hunks {
                let (hunk, num_lines) = patch.hunk(hunk_idx)?;

                // Save previous hunk if any
                if let Some(h) = current_hunk.take() {
                    hunks.push(h);
                }

                // Start new hunk
                let mut diff_hunk = DiffHunk {
                    file_path: current_file_path.clone(),
                    header: format!(
                        "@@ -{},{} +{},{} @@",
                        hunk.old_start(),
                        hunk.old_lines(),
                        hunk.new_start(),
                        hunk.new_lines()
                    ),
                    lines: Vec::new(),
                    old_start: hunk.old_start(),
                    old_lines: hunk.old_lines(),
                    new_start: hunk.new_start(),
                    new_lines: hunk.new_lines(),
                };

                // Add lines
                for line_idx in 0..num_lines {
                    let line = patch.line_in_hunk(hunk_idx, line_idx)?;
                    let origin = line.origin();
                    if origin == '+' || origin == '-' || origin == ' ' {
                        diff_hunk.lines.push(DiffLine {
                            origin,
                            content: String::from_utf8_lossy(line.content()).to_string(),
                        });
                    }
                }

                current_hunk = Some(diff_hunk);
            }
        }
    }

    // Don't forget the last hunk
    if let Some(h) = current_hunk.take() {
        hunks.push(h);
    }

    Ok(hunks)
}

fn diff_file_supports_hunk_reconstruction(
    repo: &git2::Repository,
    file: &git2::DiffFile<'_>,
) -> Result<bool> {
    if !file.exists() {
        return Ok(true);
    }
    if !matches!(
        file.mode(),
        git2::FileMode::Blob | git2::FileMode::BlobGroupWritable | git2::FileMode::BlobExecutable
    ) {
        return Ok(false);
    }
    let Ok(blob) = repo.find_blob(file.id()) else {
        return Ok(false);
    };

    let content = blob.content();
    Ok(std::str::from_utf8(content).is_ok() && !content.contains(&b'\r'))
}

/// Print help for interactive hunk selection
fn print_hunk_help() {
    println!();
    println!(
        "  {} - include this hunk in the new commit",
        style("y").green().bold()
    );
    println!(
        "  {} - skip this hunk (stays in remainder)",
        style("n").red().bold()
    );
    println!(
        "  {} - include all remaining hunks in this file",
        style("a").cyan().bold()
    );
    println!(
        "  {} - skip all remaining hunks in this file",
        style("d").cyan().bold()
    );
    println!(
        "  {} - split this hunk into smaller hunks",
        style("s").yellow().bold()
    );
    println!(
        "  {} - stop; all remaining hunks stay in remainder",
        style("q").magenta().bold()
    );
    println!("  {} - show this help", style("?").white().bold());
    println!();
}

/// Try to split a hunk into smaller sub-hunks
/// Returns None if the hunk cannot be split further
pub fn try_split_hunk(hunk: &DiffHunk) -> Option<Vec<DiffHunk>> {
    // Find points where we can split: context lines between change groups
    // A change group is a sequence of + and - lines
    // We can split when we see a change line after one or more context lines
    // that follow a previous change group

    let mut split_points = Vec::new();
    let mut had_change = false;
    let mut context_after_change = 0;

    for (i, line) in hunk.lines.iter().enumerate() {
        if line.origin == '+' || line.origin == '-' {
            // If we had a previous change and then context, this is a new group
            if had_change && context_after_change > 0 {
                // Split point is at the start of this new change group
                split_points.push(i);
            }
            had_change = true;
            context_after_change = 0;
        } else {
            // Context line
            if had_change {
                context_after_change += 1;
            }
        }
    }

    // Need at least one valid split point to actually split
    if split_points.is_empty() {
        return None;
    }

    // Create sub-hunks
    // split_points contains indices where a new change group starts
    // We split just before each split point
    let mut sub_hunks = Vec::new();
    let mut start = 0;
    let mut old_line = hunk.old_start;
    let mut new_line = hunk.new_start;

    for &split_at in &split_points {
        if split_at <= start {
            continue;
        }

        // Create sub-hunk from start to split_at
        let sub_lines: Vec<DiffLine> = hunk.lines[start..split_at].to_vec();
        let (old_count, new_count) = count_hunk_lines(&sub_lines);

        if old_count > 0 || new_count > 0 {
            sub_hunks.push(DiffHunk {
                file_path: hunk.file_path.clone(),
                header: format!(
                    "@@ -{},{} +{},{} @@",
                    old_line, old_count, new_line, new_count
                ),
                lines: sub_lines,
                old_start: old_line,
                old_lines: old_count,
                new_start: new_line,
                new_lines: new_count,
            });
        }

        // Update line numbers for next sub-hunk
        for line in &hunk.lines[start..split_at] {
            match line.origin {
                '-' => old_line += 1,
                '+' => new_line += 1,
                ' ' => {
                    old_line += 1;
                    new_line += 1;
                }
                _ => {}
            }
        }
        start = split_at;
    }

    // Don't forget the last segment
    if start < hunk.lines.len() {
        let sub_lines: Vec<DiffLine> = hunk.lines[start..].to_vec();
        let (old_count, new_count) = count_hunk_lines(&sub_lines);

        if old_count > 0 || new_count > 0 {
            sub_hunks.push(DiffHunk {
                file_path: hunk.file_path.clone(),
                header: format!(
                    "@@ -{},{} +{},{} @@",
                    old_line, old_count, new_line, new_count
                ),
                lines: sub_lines,
                old_start: old_line,
                old_lines: old_count,
                new_start: new_line,
                new_lines: new_count,
            });
        }
    }

    // Only return if we actually split into multiple hunks
    if sub_hunks.len() > 1 {
        Some(sub_hunks)
    } else {
        None
    }
}

/// Count old and new lines in a hunk's line list
fn count_hunk_lines(lines: &[DiffLine]) -> (u32, u32) {
    let mut old_count = 0u32;
    let mut new_count = 0u32;
    for line in lines {
        match line.origin {
            '-' => old_count += 1,
            '+' => new_count += 1,
            ' ' => {
                old_count += 1;
                new_count += 1;
            }
            _ => {}
        }
    }
    (old_count, new_count)
}

/// Display a hunk with colored output
fn display_hunk(hunk: &DiffHunk, is_first_in_file: bool) {
    if is_first_in_file {
        println!("{}", style(format!("--- a/{}", hunk.file_path)).bold());
        println!("{}", style(format!("+++ b/{}", hunk.file_path)).bold());
    }
    println!("{}", style(&hunk.header).cyan());
    for line in &hunk.lines {
        let line_str = format!("{}{}", line.origin, line.content.trim_end_matches('\n'));
        match line.origin {
            '+' => println!("{}", style(line_str).green()),
            '-' => println!("{}", style(line_str).red()),
            _ => println!("{}", line_str),
        }
    }
}

/// Interactive hunk selection (git add -p style)
/// Returns indices of selected hunks
/// When a hunk is split and sub-hunks are selected, the original hunk is replaced
/// with a merged hunk containing only the selected sub-hunk lines.
fn select_hunks_interactive(hunks: &mut Vec<DiffHunk>) -> Result<Vec<usize>> {
    let term = Term::stdout();
    let mut selected: Vec<usize> = Vec::new();
    let mut i = 0;
    let mut last_file_path = String::new();

    // Track which files to skip entirely
    let mut skip_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Track which files to include entirely (remaining hunks)
    let mut include_files: std::collections::HashSet<String> = std::collections::HashSet::new();

    println!();
    println!(
        "Select hunks for the {} (the rest stays in the original):",
        style("new commit (inserted BEFORE the original in the stack)").bold()
    );
    println!();

    while i < hunks.len() {
        // Clone the hunk to avoid borrow conflicts when splicing for 's' action
        let hunk = hunks[i].clone();

        // Check if we should auto-handle this file
        if skip_files.contains(&hunk.file_path) {
            i += 1;
            continue;
        }
        if include_files.contains(&hunk.file_path) {
            selected.push(i);
            i += 1;
            continue;
        }

        // Display hunk
        let is_first = hunk.file_path != last_file_path;
        display_hunk(&hunk, is_first);
        last_file_path = hunk.file_path.clone();

        // Prompt
        print!(
            "Include this hunk? [{}]es/[{}]o/[{}]ll file/[{}]one file/[{}]plit/[{}]uit/[{}]help: ",
            style("y").green(),
            style("n").red(),
            style("a").cyan(),
            style("d").cyan(),
            style("s").yellow(),
            style("q").magenta(),
            style("?").white()
        );
        io::stdout().flush().ok();

        let ch = term
            .read_char()
            .map_err(|e| GgError::Other(format!("Failed to read input: {}", e)))?;
        println!();

        match ch.to_ascii_lowercase() {
            'y' => {
                selected.push(i);
                i += 1;
            }
            'n' => {
                i += 1;
            }
            'a' => {
                // Include all remaining hunks in this file
                let current_file = hunk.file_path.clone();
                selected.push(i);
                include_files.insert(current_file);
                i += 1;
            }
            'd' => {
                // Skip all remaining hunks in this file
                let current_file = hunk.file_path.clone();
                skip_files.insert(current_file);
                i += 1;
            }
            's' => {
                // Try to split the hunk into sub-hunks
                if let Some(sub_hunks) = try_split_hunk(&hunk) {
                    println!(
                        "{}",
                        style(format!("Split into {} sub-hunks", sub_hunks.len())).dim()
                    );

                    // Splice: replace the current hunk with the sub-hunks
                    // The main loop will then process each sub-hunk individually with y/n
                    hunks.splice(i..=i, sub_hunks);
                    // Don't increment i - the loop continues with the first sub-hunk
                } else {
                    println!("{}", style("This hunk cannot be split further.").yellow());
                    // Re-prompt for same hunk
                }
            }
            'q' => {
                // Stop - all remaining hunks are unselected
                break;
            }
            '?' => {
                print_hunk_help();
                // Re-prompt for same hunk
            }
            _ => {
                println!("Unknown option. Press ? for help.");
                // Re-prompt for same hunk
            }
        }
    }

    Ok(selected)
}

/// Build a tree from selected hunks
/// This applies selected hunks to the parent tree content
fn build_tree_from_hunks<'a>(
    repo: &'a git2::Repository,
    parent_tree: &git2::Tree,
    target_tree: &git2::Tree,
    hunks: &[DiffHunk],
    selected_indices: &[usize],
    non_hunk_files: &[String],
) -> Result<git2::Tree<'a>> {
    let target_diff_files = target_diff_files(repo, parent_tree, target_tree)?;

    // Group hunks by file
    let mut file_hunks: BTreeMap<String, Vec<(usize, &DiffHunk)>> = BTreeMap::new();
    for (idx, hunk) in hunks.iter().enumerate() {
        file_hunks
            .entry(hunk.file_path.clone())
            .or_default()
            .push((idx, hunk));
    }

    // For each file, determine what to do:
    // - All hunks selected: use target tree entry unless metadata must remain
    // - No hunks selected: use parent tree entry
    // - Partial: apply selected hunks to parent content
    let mut accumulated_tree = repo.find_tree(parent_tree.id())?;

    for (file_path, file_hunk_list) in &file_hunks {
        let file_indices: Vec<usize> = file_hunk_list.iter().map(|(idx, _)| *idx).collect();
        let selected_in_file: Vec<usize> = file_indices
            .iter()
            .filter(|idx| selected_indices.contains(idx))
            .copied()
            .collect();

        if selected_in_file.is_empty() {
            // No hunks selected for this file - keep parent version (no change to builder)
            continue;
        }

        let tree_oid = {
            let mut builder = repo.treebuilder(Some(&accumulated_tree))?;
            if selected_in_file.len() == file_hunk_list.len() {
                let target_file = target_diff_files.get(file_path).ok_or_else(|| {
                    GgError::Other(format!("Missing diff identity for '{file_path}'"))
                })?;
                let has_mode_change =
                    has_regular_file_mode_change(&accumulated_tree, target_tree, file_path);
                if has_mode_change {
                    let selected_hunks = file_hunk_list
                        .iter()
                        .map(|(_, hunk)| *hunk)
                        .collect::<Vec<_>>();
                    apply_hunks_to_tree(
                        repo,
                        &mut builder,
                        &accumulated_tree,
                        file_path,
                        &selected_hunks,
                    )?;
                } else if let Some(target_file) = target_file {
                    update_tree_entry(
                        repo,
                        &mut builder,
                        &accumulated_tree,
                        file_path,
                        *target_file,
                    )?;
                } else {
                    remove_tree_entry(repo, &mut builder, &accumulated_tree, file_path)?;
                }
            } else {
                // Partial selection - apply hunks
                let selected_hunks: Vec<&DiffHunk> = file_hunk_list
                    .iter()
                    .filter(|(idx, _)| selected_indices.contains(idx))
                    .map(|(_, h)| *h)
                    .collect();

                apply_hunks_to_tree(
                    repo,
                    &mut builder,
                    &accumulated_tree,
                    file_path,
                    &selected_hunks,
                )?;
            }
            builder.write()?
        };
        accumulated_tree = repo.find_tree(tree_oid)?;
    }

    // Include non-hunk files (binary, rename-only, mode-only) wholesale from target tree
    let mut sorted_non_hunk_files = non_hunk_files.to_vec();
    sorted_non_hunk_files.sort();
    sorted_non_hunk_files.dedup();
    for file_path in &sorted_non_hunk_files {
        let tree_oid = {
            let mut builder = repo.treebuilder(Some(&accumulated_tree))?;
            let target_file = target_diff_files.get(file_path).ok_or_else(|| {
                GgError::Other(format!("Missing diff identity for '{file_path}'"))
            })?;
            if let Some(target_file) = target_file {
                update_tree_entry(
                    repo,
                    &mut builder,
                    &accumulated_tree,
                    file_path,
                    *target_file,
                )?;
            } else {
                remove_tree_entry(repo, &mut builder, &accumulated_tree, file_path)?;
            }
            builder.write()?
        };
        accumulated_tree = repo.find_tree(tree_oid)?;
    }

    Ok(accumulated_tree)
}

fn has_regular_file_mode_change(
    parent_tree: &git2::Tree,
    target_tree: &git2::Tree,
    file_path: &str,
) -> bool {
    let parent_mode = parent_tree
        .get_path(Path::new(file_path))
        .ok()
        .map(|entry| entry.filemode());
    let target_mode = target_tree
        .get_path(Path::new(file_path))
        .ok()
        .map(|entry| entry.filemode());
    target_mode.is_some_and(|target_mode| {
        matches!(target_mode, 0o100644 | 0o100755)
            && match parent_mode {
                Some(mode @ (0o100644 | 0o100755)) => mode != target_mode,
                None => target_mode == 0o100755,
                _ => false,
            }
    })
}

fn target_diff_files(
    repo: &git2::Repository,
    parent_tree: &git2::Tree,
    target_tree: &git2::Tree,
) -> Result<HashMap<String, Option<TargetDiffFile>>> {
    let diff = repo.diff_tree_to_tree(Some(parent_tree), Some(target_tree), None)?;
    let mut files: HashMap<String, Option<TargetDiffFile>> = HashMap::new();
    for delta in diff.deltas() {
        let old_file = delta.old_file();
        let new_file = delta.new_file();
        let path = new_file
            .path()
            .or_else(|| old_file.path())
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        let target_file = new_file.exists().then(|| TargetDiffFile {
            oid: new_file.id(),
            mode: new_file.mode().into(),
        });
        files
            .entry(path)
            .and_modify(|existing| {
                if target_file.is_some() {
                    *existing = target_file;
                }
            })
            .or_insert(target_file);
    }
    Ok(files)
}

/// Update a tree entry to use the target version
fn update_tree_entry(
    repo: &git2::Repository,
    builder: &mut git2::TreeBuilder,
    parent_tree: &git2::Tree,
    file_path: &str,
    target_file: TargetDiffFile,
) -> Result<()> {
    let path = std::path::Path::new(file_path);

    if path.parent().is_some() && path.parent() != Some(std::path::Path::new("")) {
        // Nested path
        insert_nested_entry(repo, builder, parent_tree, file_path, target_file)?;
    } else {
        // Top-level file
        let name = path.file_name().unwrap().to_string_lossy();
        builder.insert(&*name, target_file.oid, target_file.mode)?;
    }
    Ok(())
}

/// Remove a tree entry (for deletions)
fn remove_tree_entry(
    repo: &git2::Repository,
    builder: &mut git2::TreeBuilder,
    parent_tree: &git2::Tree,
    file_path: &str,
) -> Result<()> {
    let path = std::path::Path::new(file_path);
    let name = path.file_name().unwrap().to_string_lossy();

    if path.parent().is_none() || path.parent() == Some(std::path::Path::new("")) {
        let _ = builder.remove(&*name);
    } else {
        // For nested paths, we need to rebuild the tree without this entry
        // This is complex - for now, use the same approach as insert_nested_entry but with None
        insert_nested_entry_for_deletion(repo, builder, parent_tree, file_path)?;
    }
    Ok(())
}

/// Insert nested entry for a deletion
fn insert_nested_entry_for_deletion(
    repo: &git2::Repository,
    root_builder: &mut git2::TreeBuilder,
    parent_tree: &git2::Tree,
    file_path: &str,
) -> Result<()> {
    if parent_tree.get_path(Path::new(file_path)).is_err() {
        return Ok(());
    }

    let parts: Vec<&str> = file_path.split('/').collect();
    match rebuild_subtree_for_deletion(repo, parent_tree, &parts, 0)? {
        Some(new_subtree_oid) => {
            root_builder
                .insert(parts[0], new_subtree_oid, 0o040000)
                .map_err(|e| GgError::Other(format!("Failed to update root tree: {}", e)))?;
        }
        None => {
            let _ = root_builder.remove(parts[0]);
        }
    }
    Ok(())
}

fn rebuild_subtree_for_deletion(
    repo: &git2::Repository,
    parent_tree: &git2::Tree,
    parts: &[&str],
    depth: usize,
) -> Result<Option<git2::Oid>> {
    let subpath = parts[..=depth].join("/");
    let current_subtree = if depth == 0 {
        parent_tree
            .get_name(parts[0])
            .and_then(|entry| repo.find_tree(entry.id()).ok())
    } else {
        parent_tree
            .get_path(Path::new(&subpath))
            .ok()
            .and_then(|entry| repo.find_tree(entry.id()).ok())
    };
    let mut builder = repo.treebuilder(current_subtree.as_ref())?;

    if depth == parts.len() - 2 {
        let _ = builder.remove(parts[parts.len() - 1]);
    } else {
        match rebuild_subtree_for_deletion(repo, parent_tree, parts, depth + 1)? {
            Some(child_oid) => {
                builder.insert(parts[depth + 1], child_oid, 0o040000)?;
            }
            None => {
                let _ = builder.remove(parts[depth + 1]);
            }
        }
    }

    if builder.is_empty() {
        Ok(None)
    } else {
        Ok(Some(builder.write()?))
    }
}

/// Apply selected hunks to a file and update the tree
fn apply_hunks_to_tree(
    repo: &git2::Repository,
    builder: &mut git2::TreeBuilder,
    parent_tree: &git2::Tree,
    file_path: &str,
    selected_hunks: &[&DiffHunk],
) -> Result<()> {
    let path = std::path::Path::new(file_path);

    // Get parent file content
    let parent_content = if let Ok(entry) = parent_tree.get_path(path) {
        let blob = repo.find_blob(entry.id())?;
        String::from_utf8_lossy(blob.content()).to_string()
    } else {
        // New file - start empty
        String::new()
    };

    // Apply hunks to get new content
    let new_content = apply_hunks_to_content(&parent_content, selected_hunks)?;

    // Create blob with new content
    let blob_oid = repo.blob(new_content.as_bytes())?;

    // Get file mode (default to regular file)
    let filemode = if let Ok(entry) = parent_tree.get_path(path) {
        entry.filemode()
    } else {
        0o100644 // Regular file
    };

    // Update tree
    if path.parent().is_some() && path.parent() != Some(std::path::Path::new("")) {
        // Nested path - need to rebuild directory structure
        insert_blob_at_path(repo, builder, parent_tree, file_path, blob_oid, filemode)?;
    } else {
        // Top-level file
        let name = path.file_name().unwrap().to_string_lossy();
        builder.insert(&*name, blob_oid, filemode)?;
    }

    Ok(())
}

/// Apply hunks to file content
fn apply_hunks_to_content(parent_content: &str, hunks: &[&DiffHunk]) -> Result<String> {
    // Sort hunks by their position in the file (old_start)
    let mut sorted_hunks: Vec<&DiffHunk> = hunks.to_vec();
    sorted_hunks.sort_by_key(|h| h.old_start);

    let parent_lines: Vec<&str> = parent_content.split_inclusive('\n').collect();
    let mut result = String::new();
    let mut parent_idx = 0usize; // 0-indexed position in parent

    for hunk in sorted_hunks {
        // old_start is 1-indexed
        let hunk_start = (hunk.old_start as usize).saturating_sub(1);

        // Copy lines from parent before this hunk
        while parent_idx < hunk_start && parent_idx < parent_lines.len() {
            result.push_str(parent_lines[parent_idx]);
            parent_idx += 1;
        }

        // Apply the hunk
        for line in &hunk.lines {
            match line.origin {
                '+' => {
                    result.push_str(&line.content);
                }
                '-' => {
                    // Skip (delete) line from parent
                    parent_idx += 1;
                }
                ' ' if parent_idx < parent_lines.len() => {
                    // Context - should match, advance parent
                    result.push_str(parent_lines[parent_idx]);
                    parent_idx += 1;
                }
                _ => {}
            }
        }
    }

    // Copy remaining lines from parent
    while parent_idx < parent_lines.len() {
        result.push_str(parent_lines[parent_idx]);
        parent_idx += 1;
    }

    Ok(result)
}

/// Insert a blob at a nested path
fn insert_blob_at_path(
    repo: &git2::Repository,
    root_builder: &mut git2::TreeBuilder,
    parent_tree: &git2::Tree,
    file_path: &str,
    blob_oid: git2::Oid,
    filemode: i32,
) -> Result<()> {
    let parts: Vec<&str> = file_path.split('/').collect();
    let new_subtree_oid =
        rebuild_subtree_with_blob(repo, parent_tree, &parts, 0, blob_oid, filemode)?;
    root_builder
        .insert(parts[0], new_subtree_oid, 0o040000)
        .map_err(|e| GgError::Other(format!("Failed to update root tree: {}", e)))?;
    Ok(())
}

/// Recursively rebuild a subtree with a new blob at the leaf
fn rebuild_subtree_with_blob(
    repo: &git2::Repository,
    parent_tree: &git2::Tree,
    parts: &[&str],
    depth: usize,
    blob_oid: git2::Oid,
    filemode: i32,
) -> std::result::Result<git2::Oid, GgError> {
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
        builder
            .insert(filename, blob_oid, filemode)
            .map_err(|e| GgError::Other(format!("Failed to insert blob: {}", e)))?;
    } else {
        // Intermediate directory - recurse
        let child_oid =
            rebuild_subtree_with_blob(repo, parent_tree, parts, depth + 1, blob_oid, filemode)?;
        builder
            .insert(parts[depth + 1], child_oid, 0o040000)
            .map_err(|e| GgError::Other(format!("Failed to insert subtree: {}", e)))?;
    }

    builder
        .write()
        .map_err(|e| GgError::Other(format!("Failed to write tree: {}", e)))
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

    #[test]
    fn test_rebase_descendants_conflict_stays_continuable() {
        let err = rebase_descendants_error("", "CONFLICT (content): Merge conflict");

        assert!(matches!(err, GgError::RebaseConflict));
    }

    #[test]
    fn test_rebase_descendants_non_conflict_preserves_git_error() {
        let err = rebase_descendants_error("", "fatal: bad revision");

        assert!(matches!(err, GgError::Other(message) if message.contains("bad revision")));
    }

    #[test]
    fn test_apply_hunks_single_addition() {
        let parent = "line1\nline2\nline3\n";
        let hunk = DiffHunk {
            file_path: "test.txt".to_string(),
            header: "@@ -2,1 +2,2 @@".to_string(),
            lines: vec![
                DiffLine {
                    origin: ' ',
                    content: "line2\n".to_string(),
                },
                DiffLine {
                    origin: '+',
                    content: "new line\n".to_string(),
                },
            ],
            old_start: 2,
            old_lines: 1,
            new_start: 2,
            new_lines: 2,
        };

        let result = apply_hunks_to_content(parent, &[&hunk]).unwrap();
        assert_eq!(result, "line1\nline2\nnew line\nline3\n");
    }

    #[test]
    fn test_apply_hunks_single_deletion() {
        let parent = "line1\nline2\nline3\n";
        let hunk = DiffHunk {
            file_path: "test.txt".to_string(),
            header: "@@ -2,1 +2,0 @@".to_string(),
            lines: vec![DiffLine {
                origin: '-',
                content: "line2\n".to_string(),
            }],
            old_start: 2,
            old_lines: 1,
            new_start: 2,
            new_lines: 0,
        };

        let result = apply_hunks_to_content(parent, &[&hunk]).unwrap();
        assert_eq!(result, "line1\nline3\n");
    }

    #[test]
    fn test_apply_hunks_replacement() {
        let parent = "line1\nline2\nline3\n";
        let hunk = DiffHunk {
            file_path: "test.txt".to_string(),
            header: "@@ -2,1 +2,1 @@".to_string(),
            lines: vec![
                DiffLine {
                    origin: '-',
                    content: "line2\n".to_string(),
                },
                DiffLine {
                    origin: '+',
                    content: "modified line2\n".to_string(),
                },
            ],
            old_start: 2,
            old_lines: 1,
            new_start: 2,
            new_lines: 1,
        };

        let result = apply_hunks_to_content(parent, &[&hunk]).unwrap();
        assert_eq!(result, "line1\nmodified line2\nline3\n");
    }

    #[test]
    fn test_apply_hunks_multiple_hunks() {
        let parent = "line1\nline2\nline3\nline4\nline5\n";

        let hunk1 = DiffHunk {
            file_path: "test.txt".to_string(),
            header: "@@ -1,1 +1,2 @@".to_string(),
            lines: vec![
                DiffLine {
                    origin: ' ',
                    content: "line1\n".to_string(),
                },
                DiffLine {
                    origin: '+',
                    content: "after line1\n".to_string(),
                },
            ],
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 2,
        };

        let hunk2 = DiffHunk {
            file_path: "test.txt".to_string(),
            header: "@@ -4,1 +5,1 @@".to_string(),
            lines: vec![
                DiffLine {
                    origin: '-',
                    content: "line4\n".to_string(),
                },
                DiffLine {
                    origin: '+',
                    content: "modified line4\n".to_string(),
                },
            ],
            old_start: 4,
            old_lines: 1,
            new_start: 5,
            new_lines: 1,
        };

        let result = apply_hunks_to_content(parent, &[&hunk1, &hunk2]).unwrap();
        assert_eq!(
            result,
            "line1\nafter line1\nline2\nline3\nmodified line4\nline5\n"
        );
    }

    #[test]
    fn test_apply_hunks_partial_selection() {
        // Test applying only one of multiple hunks
        let parent = "line1\nline2\nline3\n";

        let hunk1 = DiffHunk {
            file_path: "test.txt".to_string(),
            header: "@@ -1,1 +1,2 @@".to_string(),
            lines: vec![
                DiffLine {
                    origin: ' ',
                    content: "line1\n".to_string(),
                },
                DiffLine {
                    origin: '+',
                    content: "new after 1\n".to_string(),
                },
            ],
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 2,
        };

        // Only apply hunk1, not hunk2
        let result = apply_hunks_to_content(parent, &[&hunk1]).unwrap();
        assert_eq!(result, "line1\nnew after 1\nline2\nline3\n");
    }

    #[test]
    fn test_apply_hunks_empty_parent() {
        // New file creation
        let parent = "";
        let hunk = DiffHunk {
            file_path: "test.txt".to_string(),
            header: "@@ -0,0 +1,2 @@".to_string(),
            lines: vec![
                DiffLine {
                    origin: '+',
                    content: "line1\n".to_string(),
                },
                DiffLine {
                    origin: '+',
                    content: "line2\n".to_string(),
                },
            ],
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: 2,
        };

        let result = apply_hunks_to_content(parent, &[&hunk]).unwrap();
        assert_eq!(result, "line1\nline2\n");
    }

    #[test]
    fn test_count_hunk_lines() {
        let lines = vec![
            DiffLine {
                origin: ' ',
                content: "context\n".to_string(),
            },
            DiffLine {
                origin: '-',
                content: "deleted\n".to_string(),
            },
            DiffLine {
                origin: '+',
                content: "added1\n".to_string(),
            },
            DiffLine {
                origin: '+',
                content: "added2\n".to_string(),
            },
            DiffLine {
                origin: ' ',
                content: "context\n".to_string(),
            },
        ];
        let (old, new) = count_hunk_lines(&lines);
        assert_eq!(old, 3); // 2 context + 1 deletion
        assert_eq!(new, 4); // 2 context + 2 additions
    }

    #[test]
    fn test_try_split_hunk_cannot_split() {
        // Contiguous changes cannot be split
        let hunk = DiffHunk {
            file_path: "test.txt".to_string(),
            header: "@@ -1,2 +1,3 @@".to_string(),
            lines: vec![
                DiffLine {
                    origin: '-',
                    content: "old1\n".to_string(),
                },
                DiffLine {
                    origin: '-',
                    content: "old2\n".to_string(),
                },
                DiffLine {
                    origin: '+',
                    content: "new1\n".to_string(),
                },
                DiffLine {
                    origin: '+',
                    content: "new2\n".to_string(),
                },
                DiffLine {
                    origin: '+',
                    content: "new3\n".to_string(),
                },
            ],
            old_start: 1,
            old_lines: 2,
            new_start: 1,
            new_lines: 3,
        };

        assert!(try_split_hunk(&hunk).is_none());
    }

    #[test]
    fn test_try_split_hunk_can_split() {
        // Changes separated by context can be split
        let hunk = DiffHunk {
            file_path: "test.txt".to_string(),
            header: "@@ -1,5 +1,7 @@".to_string(),
            lines: vec![
                DiffLine {
                    origin: '+',
                    content: "new1\n".to_string(),
                },
                DiffLine {
                    origin: ' ',
                    content: "context1\n".to_string(),
                },
                DiffLine {
                    origin: ' ',
                    content: "context2\n".to_string(),
                },
                DiffLine {
                    origin: ' ',
                    content: "context3\n".to_string(),
                },
                DiffLine {
                    origin: '+',
                    content: "new2\n".to_string(),
                },
                DiffLine {
                    origin: ' ',
                    content: "context4\n".to_string(),
                },
                DiffLine {
                    origin: ' ',
                    content: "context5\n".to_string(),
                },
            ],
            old_start: 1,
            old_lines: 5,
            new_start: 1,
            new_lines: 7,
        };

        let result = try_split_hunk(&hunk);
        assert!(result.is_some());
        let sub_hunks = result.unwrap();
        assert!(sub_hunks.len() >= 2);
    }
}
