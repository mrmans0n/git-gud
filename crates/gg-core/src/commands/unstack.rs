//! `gg unstack` — Split a stack into two independent stacks.
//!
//! The selected entry and all descendants become a new independent stack;
//! lower entries remain in the original stack.

use std::io::Write;

use console::style;
use git2::{BranchType, Oid, Repository};

use super::unstack_tui::{self, UnstackEntry};
use crate::config::{Config, StackConfig};
use crate::error::{GgError, Result};
use crate::git;
use crate::immutability::{self, ImmutabilityPolicy};
use crate::operations::{OperationKind, SnapshotScope};
use crate::output::{
    print_json, UnstackEntryJson, UnstackResponse, UnstackResultJson, OUTPUT_VERSION,
};
use crate::stack::{self, Stack, StackEntry};

const AUTO_NAME_ATTEMPT_LIMIT: usize = 100;

/// Options for the unstack command.
#[derive(Debug, Default)]
pub struct UnstackOptions {
    /// First entry to move into the new stack: position, GG-ID, or SHA.
    pub target: Option<String>,
    /// New stack name. If omitted, a unique `<old-stack>-N` name is generated.
    pub name: Option<String>,
    /// Disable interactive target picker.
    pub no_tui: bool,
    /// Override the immutability check for merged/base-ancestor commits.
    pub force: bool,
    /// Output as JSON.
    pub json: bool,
    /// Create or reuse a managed worktree for the new stack.
    pub worktree: bool,
}

/// Run the unstack command.
pub fn run(options: UnstackOptions) -> Result<()> {
    let repo = git::open_repo()?;
    let mut config = Config::load_with_global(repo.commondir())?;

    let _lock = git::acquire_operation_lock(&repo, "unstack")?;
    git::require_clean_working_directory(&repo)?;

    let mut stack_obj = Stack::load(&repo, &config)?;
    immutability::refresh_mr_state_for_guard(&repo, &mut stack_obj);

    if stack_obj.is_empty() {
        return Err(GgError::Other("Stack is empty.".to_string()));
    }
    if stack_obj.len() < 2 {
        return Err(GgError::Other(
            "Need at least 2 commits to unstack.".to_string(),
        ));
    }

    // Resolve split position
    let split_position = resolve_split_position(&stack_obj, &options)?;
    if split_position == 1 {
        return Err(GgError::Other(
            "Cannot unstack at position 1: the original stack would be empty.".to_string(),
        ));
    }

    // Resolve new stack name
    let new_stack_name = resolve_new_stack_name(&repo, &config, &stack_obj, &options)?;

    // Immutability guard: check entries from split_position to tip
    let targets: Vec<usize> = (split_position..=stack_obj.len()).collect();
    let policy = ImmutabilityPolicy::for_stack(&repo, &stack_obj)?;
    let report = policy.check_positions(&stack_obj, &targets);
    immutability::guard(report, options.force)?;

    // Collect info before mutation
    let original_stack = stack_obj.name.clone();
    let original_branch = stack_obj.branch_name();
    let new_branch = git::format_stack_branch(&stack_obj.username, &new_stack_name);
    let lower_tip_oid = stack_obj
        .get_entry_by_position(split_position - 1)
        .expect("split_position > 1")
        .oid;

    let moved_entries: Vec<UnstackEntryJson> = stack_obj.entries[split_position - 1..]
        .iter()
        .map(entry_to_json)
        .collect();
    let remaining_entries: Vec<UnstackEntryJson> = stack_obj.entries[..split_position - 1]
        .iter()
        .map(entry_to_json)
        .collect();

    // Begin recorded operation
    let guard = git::begin_recorded_op(
        &repo,
        &config,
        OperationKind::Unstack,
        std::env::args().skip(1).collect(),
        Some(original_stack.clone()),
        SnapshotScope::AllUserBranches,
    )?;

    if !options.json {
        println!(
            "{} Unstacking {} at position #{}...",
            style("→").cyan(),
            style(&original_stack).cyan(),
            split_position
        );
    }

    // Create the new stack by rebasing upper commits onto base
    let new_tip = rebase_upper_stack(&repo, &stack_obj, &new_branch, split_position)?;

    // Move the original stack branch down to the lower tip
    set_branch_target(
        &repo,
        &original_branch,
        lower_tip_oid,
        "gg unstack: truncate original stack",
    )?;

    // Set the new stack branch to the rebased tip
    set_branch_target(
        &repo,
        &new_branch,
        new_tip,
        "gg unstack: create new stack branch",
    )?;

    // Migrate config (MR mappings, base override)
    let migrated_review_mappings = migrate_config(
        &mut config,
        &original_stack,
        &new_stack_name,
        &moved_entries,
        options.worktree,
    );

    // Delete old entry branches for moved entries
    let deleted_entry_branches = delete_old_entry_branches(&repo, &stack_obj, split_position)?;

    config.save(repo.commondir())?;

    // Normalize metadata on the lower stack
    git::checkout_branch(&repo, &original_branch)?;
    let lower_stack = Stack::load(&repo, &config)?;
    git::normalize_stack_metadata(&repo, &lower_stack)?;

    // Normalize metadata on the new (upper) stack and leave HEAD there
    git::checkout_branch(&repo, &new_branch)?;
    let upper_stack = Stack::load(&repo, &config)?;
    git::normalize_stack_metadata(&repo, &upper_stack)?;

    // Handle worktree mode: checkout old stack branch, create worktree for new stack
    let worktree_path = if options.worktree {
        // Switch back to the original (lower) stack in the current worktree
        git::checkout_branch(&repo, &original_branch)?;
        // Create or reuse a managed worktree for the new upper stack
        let path = super::checkout::ensure_stack_worktree(
            &repo,
            &mut config,
            &new_stack_name,
            &new_branch,
        )?;
        config.save(repo.commondir())?;
        Some(path.to_string_lossy().to_string())
    } else {
        None
    };

    // Finalize the operation record
    guard.finalize_with_scope(
        &repo,
        &config,
        SnapshotScope::AllUserBranches,
        vec![],
        false,
    )?;

    let sync_required = migrated_review_mappings > 0;

    if options.json {
        print_json(&UnstackResponse {
            version: OUTPUT_VERSION,
            unstack: UnstackResultJson {
                original_stack,
                new_stack: new_stack_name,
                split_position,
                remaining_entries,
                moved_entries,
                deleted_entry_branches,
                migrated_review_mappings,
                sync_required,
                worktree_path,
            },
        });
    } else if let Some(path) = &worktree_path {
        println!(
            "{} Unstacked {} at position #{}",
            style("OK").green().bold(),
            style(&original_stack).cyan(),
            split_position
        );
        println!(
            "  {}: {} entries",
            style(&original_stack).cyan(),
            split_position - 1
        );
        println!(
            "  {}: {} entries (worktree: {})",
            style(&new_stack_name).cyan(),
            stack_obj.len() - split_position + 1,
            style(path).yellow()
        );
        if sync_required {
            println!(
                "\n{} Run `gg sync` to push the new stack and update PR/MR targets.",
                style("hint:").yellow()
            );
        }
    } else {
        println!(
            "{} Unstacked {} at position #{}",
            style("OK").green().bold(),
            style(&original_stack).cyan(),
            split_position
        );
        println!(
            "  {}: {} entries",
            style(&original_stack).cyan(),
            split_position - 1
        );
        println!(
            "  {}: {} entries",
            style(&new_stack_name).cyan(),
            stack_obj.len() - split_position + 1
        );
        if sync_required {
            println!(
                "\n{} Run `gg sync` to push the new stack and update PR/MR targets.",
                style("hint:").yellow()
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_split_position(stack: &Stack, options: &UnstackOptions) -> Result<usize> {
    // 1. Explicit --target
    if let Some(target) = &options.target {
        return stack::resolve_target(stack, target);
    }

    // 2. TUI
    let is_tty = atty::is(atty::Stream::Stdin) && atty::is(atty::Stream::Stdout);
    let use_tui = !options.no_tui && !options.json && is_tty;

    if use_tui {
        let entries: Vec<UnstackEntry> = stack
            .entries
            .iter()
            .map(|e| UnstackEntry {
                short_sha: e.short_sha.clone(),
                title: e.title.clone(),
            })
            .collect();

        // Default cursor: current position if available, otherwise position 2 (index 1)
        let initial = stack.current_position.map(|p| p.max(1)).unwrap_or(1);

        return unstack_tui::select_split_point(entries, initial)?
            .ok_or_else(|| GgError::Other("Unstack cancelled.".to_string()));
    }

    Err(GgError::Other(
        "No target specified. Use --target <position|GG-ID|SHA>, or run in a terminal for the picker."
            .to_string(),
    ))
}

fn resolve_new_stack_name(
    repo: &Repository,
    config: &Config,
    stack: &Stack,
    options: &UnstackOptions,
) -> Result<String> {
    if let Some(name) = &options.name {
        let sanitized = git::sanitize_stack_name(name)?;
        return validate_new_stack_name(repo, config, stack, &sanitized);
    }
    generate_new_stack_name(repo, config, stack)
}

fn validate_new_stack_name(
    repo: &Repository,
    config: &Config,
    stack: &Stack,
    name: &str,
) -> Result<String> {
    if name == stack.name {
        return Err(GgError::Other(format!(
            "New stack name must differ from the original stack '{}'.",
            stack.name
        )));
    }
    if config.get_stack(name).is_some() {
        return Err(GgError::Other(format!(
            "Stack config for '{}' already exists.",
            name
        )));
    }
    let branch_name = git::format_stack_branch(&stack.username, name);
    if repo.find_branch(&branch_name, BranchType::Local).is_ok() {
        return Err(GgError::Other(format!("Stack '{}' already exists.", name)));
    }
    Ok(name.to_string())
}

fn generate_new_stack_name(repo: &Repository, config: &Config, stack: &Stack) -> Result<String> {
    let prefix = stack.name.trim_end_matches('-');
    let prefix = if prefix.is_empty() { "unstack" } else { prefix };

    for suffix in 2..(2 + AUTO_NAME_ATTEMPT_LIMIT) {
        let candidate = format!("{}-{}", prefix, suffix);
        let Ok(candidate) = git::sanitize_stack_name(&candidate) else {
            continue;
        };
        if validate_new_stack_name(repo, config, stack, &candidate).is_ok() {
            return Ok(candidate);
        }
    }
    Err(GgError::Other(format!(
        "Could not generate a valid unused stack name after {} attempts. Use --name.",
        AUTO_NAME_ATTEMPT_LIMIT
    )))
}

/// Rebase the upper portion of the stack onto the base branch.
///
/// Uses `git rebase --onto <base> <lower-tip> <new-branch>` so the upper
/// commits are replayed cleanly onto the base.
fn rebase_upper_stack(
    repo: &Repository,
    stack: &Stack,
    new_branch: &str,
    split_position: usize,
) -> Result<Oid> {
    let original_branch = stack.branch_name();
    let base_ref = repo
        .revparse_single(&stack.base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", stack.base)))
        .map_err(|_| GgError::NoBaseBranch)?;

    let stack_tip = stack.last().expect("stack is non-empty").oid;
    let lower_tip = stack
        .get_entry_by_position(split_position - 1)
        .expect("split_position > 1")
        .oid;

    // Create the new branch at the current stack tip (before rebase)
    let tip_commit = repo.find_commit(stack_tip)?;
    repo.branch(new_branch, &tip_commit, false)
        .map_err(|e| GgError::Other(format!("Failed to create branch '{}': {}", new_branch, e)))?;

    // Build a rebase todo that picks only the upper commits
    let upper_entries = &stack.entries[split_position - 1..];
    let mut rebase_todo = String::new();
    for entry in upper_entries {
        rebase_todo.push_str(&format!("pick {}\n", entry.oid));
    }

    let unique_id = std::process::id();
    let todo_file = std::env::temp_dir().join(format!("gg-unstack-todo-{}", unique_id));
    std::fs::write(&todo_file, &rebase_todo)?;

    let editor_script = format!("#!/bin/sh\ncat {} > \"$1\"", todo_file.display());
    let script_file = std::env::temp_dir().join(format!("gg-unstack-editor-{}.sh", unique_id));
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
        .args([
            "rebase",
            "-i",
            "--onto",
            &base_ref.id().to_string(),
            &lower_tip.to_string(),
            new_branch,
        ])
        .output()?;

    let _ = std::fs::remove_file(&todo_file);
    let _ = std::fs::remove_file(&script_file);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        cleanup_failed_unstack_rebase(repo, &original_branch, new_branch)?;
        if stderr.contains("CONFLICT") || stderr.contains("conflict") {
            return Err(GgError::RebaseConflict);
        }
        return Err(GgError::Other(format!("Rebase failed: {}", stderr)));
    }

    // Read the new tip from the rebased branch
    let new_branch_ref = repo
        .find_branch(new_branch, BranchType::Local)
        .map_err(|e| GgError::Other(format!("Could not find new branch after rebase: {}", e)))?;
    let new_tip = new_branch_ref
        .get()
        .target()
        .ok_or_else(|| GgError::Other("New branch has no target after rebase".to_string()))?;

    Ok(new_tip)
}

fn cleanup_failed_unstack_rebase(
    repo: &Repository,
    original_branch: &str,
    new_branch: &str,
) -> Result<()> {
    let abort_output = std::process::Command::new("git")
        .args(["rebase", "--abort"])
        .output()?;
    if !abort_output.status.success() && git::is_rebase_in_progress(repo) {
        return Err(GgError::Other(format!(
            "Rebase failed, and cleanup could not abort the rebase: {}",
            String::from_utf8_lossy(&abort_output.stderr)
        )));
    }

    if git::checkout_branch(repo, original_branch).is_err() {
        if let Ok(head) = repo.head() {
            if let Some(oid) = head.target() {
                repo.set_head_detached(oid).map_err(|e| {
                    GgError::Other(format!(
                        "Rebase failed, and cleanup could not detach HEAD before deleting temporary branch '{}': {}",
                        new_branch, e
                    ))
                })?;
            }
        }
    }

    if let Ok(mut branch) = repo.find_branch(new_branch, BranchType::Local) {
        branch.delete().map_err(|e| {
            GgError::Other(format!(
                "Rebase failed, and cleanup could not delete temporary branch '{}': {}",
                new_branch, e
            ))
        })?;
    }

    Ok(())
}

fn set_branch_target(repo: &Repository, branch_name: &str, oid: Oid, message: &str) -> Result<()> {
    repo.reference(&format!("refs/heads/{}", branch_name), oid, true, message)?;
    Ok(())
}

fn migrate_config(
    config: &mut Config,
    original_stack: &str,
    new_stack: &str,
    moved_entries: &[UnstackEntryJson],
    worktree_mode: bool,
) -> usize {
    let original_base = config
        .get_stack(original_stack)
        .and_then(|s| s.base.clone());
    let original_worktree_path = config
        .get_stack(original_stack)
        .and_then(|s| s.worktree_path.clone());

    let mut new_config = StackConfig {
        base: original_base,
        ..StackConfig::default()
    };

    for entry in moved_entries {
        let Some(gg_id) = &entry.gg_id else {
            continue;
        };
        if let Some(mr) = config.get_mr_for_entry(original_stack, gg_id) {
            new_config.mrs.insert(gg_id.clone(), mr);
            config.remove_mr_for_entry(original_stack, gg_id);
        }
    }

    // In worktree mode, the current worktree stays with the old (lower) stack,
    // so preserve the old worktree_path on it. The new stack gets its worktree
    // assigned later by ensure_stack_worktree.
    // In normal mode, HEAD moves to the new stack, so migrate the worktree_path.
    if !worktree_mode {
        if let Some(wt_path) = original_worktree_path {
            new_config.worktree_path = Some(wt_path);
            if let Some(old_stack) = config.stacks.get_mut(original_stack) {
                old_stack.worktree_path = None;
            }
        }
    }

    let migrated = new_config.mrs.len();
    config.stacks.insert(new_stack.to_string(), new_config);
    migrated
}

fn delete_old_entry_branches(
    repo: &Repository,
    stack: &Stack,
    split_position: usize,
) -> Result<Vec<String>> {
    let mut deleted = Vec::new();
    for entry in &stack.entries[split_position - 1..] {
        let Some(branch_name) = stack.entry_branch_name(entry) else {
            continue;
        };
        if let Ok(mut branch) = repo.find_branch(&branch_name, BranchType::Local) {
            branch.delete()?;
            deleted.push(branch_name);
        }
    }
    Ok(deleted)
}

fn entry_to_json(entry: &StackEntry) -> UnstackEntryJson {
    UnstackEntryJson {
        position: entry.position,
        sha: entry.short_sha.clone(),
        title: entry.title.clone(),
        gg_id: entry.gg_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_unstack_options_default() {
        let opts = UnstackOptions::default();
        assert!(opts.target.is_none());
        assert!(opts.name.is_none());
        assert!(!opts.no_tui);
        assert!(!opts.force);
        assert!(!opts.json);
        assert!(!opts.worktree);
    }

    fn temp_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        (dir, repo)
    }

    fn test_stack(name: &str) -> Stack {
        Stack {
            name: name.to_string(),
            username: "nacho".to_string(),
            base: "main".to_string(),
            entries: vec![],
            current_position: None,
        }
    }

    #[test]
    fn explicit_name_rejects_existing_config_entry() {
        let (_dir, repo) = temp_repo();
        let stack = test_stack("feature");
        let mut config = Config::default();
        config
            .stacks
            .insert("existing".to_string(), StackConfig::default());
        let options = UnstackOptions {
            name: Some("existing".to_string()),
            ..UnstackOptions::default()
        };

        let err = resolve_new_stack_name(&repo, &config, &stack, &options).unwrap_err();

        assert!(err
            .to_string()
            .contains("Stack config for 'existing' already exists"));
    }

    #[test]
    fn generated_name_skips_existing_config_entries() {
        let (_dir, repo) = temp_repo();
        let stack = test_stack("feature");
        let mut config = Config::default();
        config
            .stacks
            .insert("feature-2".to_string(), StackConfig::default());

        let name = generate_new_stack_name(&repo, &config, &stack).unwrap();

        assert_eq!(name, "feature-3");
    }

    #[test]
    fn generated_name_falls_back_for_trailing_dash_stack_names() {
        let (_dir, repo) = temp_repo();
        let stack = test_stack("feature-");
        let config = Config::default();

        let name = generate_new_stack_name(&repo, &config, &stack).unwrap();

        assert_eq!(name, "feature-2");
    }

    #[test]
    fn generated_name_errors_after_bounded_invalid_candidates() {
        let (_dir, repo) = temp_repo();
        let stack = test_stack("bad@name");
        let config = Config::default();

        let err = generate_new_stack_name(&repo, &config, &stack).unwrap_err();

        assert!(err
            .to_string()
            .contains("Could not generate a valid unused stack name after 100 attempts"));
    }

    #[test]
    fn migrate_config_preserves_inherited_base() {
        let mut config = Config::default();
        config.defaults.base = Some("main".to_string());
        config.stacks.insert(
            "feature".to_string(),
            StackConfig {
                base: None,
                mrs: HashMap::from([("c-abc1234".to_string(), 42)]),
                worktree_path: None,
            },
        );
        let moved_entries = vec![UnstackEntryJson {
            position: 2,
            sha: "abc1234".to_string(),
            title: "Upper commit".to_string(),
            gg_id: Some("c-abc1234".to_string()),
        }];

        let migrated = migrate_config(&mut config, "feature", "feature-2", &moved_entries, false);

        assert_eq!(migrated, 1);
        assert_eq!(config.get_stack("feature-2").unwrap().base, None);
        assert_eq!(config.get_base_for_stack("feature-2"), Some("main"));
        assert_eq!(config.get_mr_for_entry("feature-2", "c-abc1234"), Some(42));
        assert_eq!(config.get_mr_for_entry("feature", "c-abc1234"), None);
    }
}
