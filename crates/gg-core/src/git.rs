//! Git operations for git-gud
//!
//! Provides utilities for repository discovery, branch management,
//! commit traversal, and rebase operations.

use std::fs::{self, File};
use std::process::Command;
use std::time::Duration;

/// Default timeout in seconds for acquiring the index.lock
const INDEX_LOCK_TIMEOUT_SECS: u64 = 10;

use fs2::FileExt;
#[allow(unused_imports)]
use git2::Branch;
use git2::{BranchType, Commit, Oid, Repository, Signature, Sort};
use regex::Regex;

use crate::error::{GgError, Result};

/// Prefix for GG-ID trailers in commit messages
pub const GG_ID_PREFIX: &str = "GG-ID:";
/// Prefix for GG-Parent trailers in commit messages
pub const GG_PARENT_PREFIX: &str = "GG-Parent:";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MetadataRewriteCounts {
    pub gg_ids_added: usize,
    pub gg_parents_updated: usize,
    pub gg_parents_removed: usize,
}

/// Open the repository at the current directory or its parents
pub fn open_repo() -> Result<Repository> {
    Repository::discover(".").map_err(|_| GgError::NotInRepo)
}

/// Per-clone gg state directory at `<commondir>/gg`.
///
/// Uses `commondir()` (not `path()`) so worktrees share a single operation
/// log and config with the main working copy.
pub fn gg_dir(repo: &Repository) -> std::path::PathBuf {
    repo.commondir().join("gg")
}

/// Operation lock handle that automatically releases on drop
#[derive(Debug)]
pub struct OperationLock {
    _lock_file: File, // gg/operation.lock (existing flock)
}

impl Drop for OperationLock {
    fn drop(&mut self) {
        // gg/operation.lock flock released automatically via File drop
    }
}

/// Acquire an exclusive operation lock to prevent concurrent gg operations.
///
/// This function implements locking in two layers:
/// 1. Checks for `.git/index.lock` — if git is running, waits or fails
///    (prevents gg from conflicting with an active git operation)
/// 2. Acquires `.git/gg/operation.lock` with flock to block other gg instances
///
/// Note: We don't CREATE index.lock ourselves because libgit2 also checks
/// it internally, and creating it would block gg's own git2 operations.
/// This means git can still start while gg is running (best-effort detection).
///
/// Returns a lock handle that will automatically release when dropped.
pub fn acquire_operation_lock(repo: &Repository, operation: &str) -> Result<OperationLock> {
    acquire_operation_lock_with_timeout(repo, operation, INDEX_LOCK_TIMEOUT_SECS)
}

/// Internal version with configurable timeout (for testing)
pub(crate) fn acquire_operation_lock_with_timeout(
    repo: &Repository,
    operation: &str,
    timeout_secs: u64,
) -> Result<OperationLock> {
    // --- Step 1: Check for index.lock (detect if git is running) ---
    // Use repo.path() for per-worktree index.lock
    let git_dir = repo.path();
    let index_lock_path = git_dir.join("index.lock");
    let index_timeout = Duration::from_secs(timeout_secs);
    let index_start = std::time::Instant::now();

    // Wait for any existing index.lock to be released
    let mut warned = false;
    while index_lock_path.exists() {
        if !warned {
            eprintln!(
                "{} Waiting for git operation to complete (index.lock exists)...",
                console::style("Note:").cyan().bold()
            );
            warned = true;
        }
        if index_start.elapsed() >= index_timeout {
            return Err(GgError::GitOperationInProgress(
                "git operation timed out".to_string(),
                index_lock_path.display().to_string(),
            ));
        }
        // Note: narrow TOCTOU gap between exists() check and gg lock acquisition
        // is acceptable for best-effort detection
        std::thread::sleep(Duration::from_millis(100));
    }

    // --- Step 2: gg/operation.lock management (blocks other gg instances) ---
    let common_dir = repo.commondir();
    let gg_dir = common_dir.join("gg");

    // Ensure gg directory exists
    fs::create_dir_all(&gg_dir)?;

    let lock_path = gg_dir.join("operation.lock");

    // Try to open or create lock file
    let lock_file = File::create(&lock_path)?;

    // Try to acquire exclusive lock with timeout
    let timeout = Duration::from_secs(10);
    let start = std::time::Instant::now();

    loop {
        match lock_file.try_lock_exclusive() {
            Ok(()) => {
                // Successfully acquired lock
                // Write operation info for debugging
                let info = format!("operation: {}\npid: {}\n", operation, std::process::id());
                let _ = fs::write(&lock_path, info);

                return Ok(OperationLock {
                    _lock_file: lock_file,
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Lock is held by another process
                if start.elapsed() >= timeout {
                    // Try to read lock info for better error message
                    let lock_info = fs::read_to_string(&lock_path)
                        .unwrap_or_else(|_| "unknown operation".to_string());

                    return Err(GgError::Other(format!(
                        "Another gg operation is currently running.\n\
                         Lock info:\n{}\n\
                         If no other gg process is running, the lock may be stale.\n\
                         You can manually remove: {}",
                        lock_info.trim(),
                        lock_path.display()
                    )));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Acquire the operation lock AND write a `Pending` operation record.
///
/// The returned [`crate::operations::OperationGuard`] must have `finalize`
/// called on the success path. Dropping without finalize leaves the record
/// `Pending` on disk; the sweep promotes it to `Interrupted` on the next
/// lock acquisition (after `PENDING_STALENESS_MS`).
///
/// This is the instrumentation sibling of [`acquire_operation_lock`]. The
/// original function stays unchanged for callers that do not need to
/// record (e.g. `gg continue` / `gg abort`).
///
/// Callers that need to gate recording on pre-mutation validation (e.g.
/// the immutability guard) should instead use [`acquire_operation_lock`]
/// combined with [`begin_recorded_op`] so that rejected operations never
/// pollute the op log.
pub fn acquire_operation_lock_and_record(
    repo: &Repository,
    config: &crate::config::Config,
    kind: crate::operations::OperationKind,
    args: Vec<String>,
    stack_name: Option<String>,
    scope: crate::operations::SnapshotScope<'_>,
) -> Result<(OperationLock, crate::operations::OperationGuard)> {
    let op_name = format!("{kind:?}").to_lowercase();
    let lock = acquire_operation_lock(repo, &op_name)?;
    let guard = begin_recorded_op(repo, config, kind, args, stack_name, scope)?;
    Ok((lock, guard))
}

/// Write a `Pending` operation record, assuming the caller already holds
/// the operation lock (via [`acquire_operation_lock`]).
///
/// This is the half of [`acquire_operation_lock_and_record`] that runs
/// *after* lock acquisition. Split out so instrumented commands can do
/// pre-mutation validation (e.g. immutability checks) without polluting
/// the op log with rejected operations: acquire the lock first, validate,
/// then call this helper immediately before mutating.
///
/// The returned guard MUST be `finalize`d on the success path — dropping
/// it leaves the record `Pending` on disk for the sweep to promote to
/// `Interrupted`.
pub fn begin_recorded_op(
    repo: &Repository,
    config: &crate::config::Config,
    kind: crate::operations::OperationKind,
    args: Vec<String>,
    stack_name: Option<String>,
    scope: crate::operations::SnapshotScope<'_>,
) -> Result<crate::operations::OperationGuard> {
    use crate::operations::{
        self, new_id, now_ms, OperationGuard, OperationRecord, OperationStatus, OperationStore,
        SCHEMA_VERSION,
    };

    // 1. Sweep stale Pending records. Swallows all errors.
    let gg_dir_path = gg_dir(repo);
    let store = OperationStore::new(&gg_dir_path);
    store.sweep_pending(now_ms());

    // 2. Capture refs_before. Snapshot errors propagate — if we can't read
    //    refs we can't safely record anything.
    let refs_before = operations::snapshot_refs(repo, config, scope)?;

    // 3. Write the Pending record.
    let record = OperationRecord {
        id: new_id(),
        schema_version: SCHEMA_VERSION,
        kind,
        status: OperationStatus::Pending,
        created_at_ms: now_ms(),
        args,
        stack_name,
        refs_before,
        refs_after: vec![],
        remote_effects: vec![],
        touched_remote: false,
        undoes: None,
        pending_plan: None,
    };
    store.save(&record)?;

    Ok(OperationGuard {
        record,
        store,
        finalized: false,
    })
}

/// Find the base branch (main, master, or trunk)
pub fn find_base_branch(repo: &Repository) -> Result<String> {
    for branch_name in &["main", "master", "trunk"] {
        if repo.find_branch(branch_name, BranchType::Local).is_ok() {
            return Ok(branch_name.to_string());
        }
        // Also check remote branches
        let remote_name = format!("origin/{}", branch_name);
        if repo
            .find_reference(&format!("refs/remotes/{}", remote_name))
            .is_ok()
        {
            return Ok(branch_name.to_string());
        }
    }
    Err(GgError::NoBaseBranch)
}

/// Get the current branch name, if on a branch
pub fn current_branch_name(repo: &Repository) -> Option<String> {
    let head = repo.head().ok()?;
    if head.is_branch() {
        head.shorthand().map(|s| s.to_string())
    } else {
        None
    }
}

/// Parse a stack branch name into (username, stack_name)
/// Note: Entry branches (username/stack--entry_id) should NOT be parsed as stack branches
pub fn parse_stack_branch(branch_name: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = branch_name.split('/').collect();
    if parts.len() == 2 {
        // Exclude entry branches which have "--" in the stack name portion
        if parts[1].contains("--") {
            return None;
        }
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

/// Parse an entry branch name into (username, stack_name, entry_id)
/// Format: username/stack_name--entry_id
pub fn parse_entry_branch(branch_name: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = branch_name.split('/').collect();
    if parts.len() == 2 && parts[1].contains("--") {
        let stack_parts: Vec<&str> = parts[1].split("--").collect();
        if stack_parts.len() == 2 {
            Some((
                parts[0].to_string(),
                stack_parts[0].to_string(),
                stack_parts[1].to_string(),
            ))
        } else {
            None
        }
    } else {
        None
    }
}

/// Format a stack branch name
pub fn format_stack_branch(username: &str, stack_name: &str) -> String {
    format!("{}/{}", username, stack_name)
}

/// Format a remote branch name for a specific entry
pub fn format_entry_branch(username: &str, stack_name: &str, entry_id: &str) -> String {
    format!("{}/{}--{}", username, stack_name, entry_id)
}

/// Find the first entry branch for a stack (username/stack_name--*)
pub fn find_entry_branch_for_stack(
    repo: &Repository,
    username: &str,
    stack_name: &str,
) -> Option<String> {
    let branches = repo.branches(Some(BranchType::Local)).ok()?;
    for (branch, _) in branches.flatten() {
        if let Ok(Some(name)) = branch.name() {
            if let Some((branch_user, branch_stack, _entry_id)) = parse_entry_branch(name) {
                if branch_user == username && branch_stack == stack_name {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Check if a branch is the HEAD of any worktree
/// Returns the worktree name if the branch is checked out, None otherwise
pub fn is_branch_checked_out_in_worktree(repo: &Repository, branch_name: &str) -> Option<String> {
    if let Ok(worktrees) = repo.worktrees() {
        for name in worktrees.iter().flatten() {
            if let Ok(wt) = repo.find_worktree(name) {
                // Open repo from worktree to check its HEAD
                if let Ok(wt_repo) = Repository::open_from_worktree(&wt) {
                    if let Some(head_name) = current_branch_name(&wt_repo) {
                        if head_name == branch_name {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Try to prune a worktree if it's stale (directory no longer exists)
/// Returns true if the worktree was pruned successfully
pub fn try_prune_worktree(repo: &Repository, wt_name: &str) -> bool {
    if let Ok(wt) = repo.find_worktree(wt_name) {
        if wt.validate().is_err() {
            // Worktree is stale, try to prune it
            if wt
                .prune(Some(
                    git2::WorktreePruneOptions::new()
                        .working_tree(true)
                        .valid(false)
                        .locked(false),
                ))
                .is_ok()
            {
                return true;
            }
        }
    }
    false
}

/// Remove a worktree using git CLI
/// This handles both stale and valid worktrees
pub fn remove_worktree(wt_name: &str) -> Result<()> {
    run_git_command(&["worktree", "remove", wt_name, "--force"])?;
    Ok(())
}

/// Check if the working directory is clean
/// Only checks for actual changes (modified, staged, deleted, etc.)
/// Ignores untracked files and submodules to match `git status` behavior
pub fn is_working_directory_clean(repo: &Repository) -> Result<bool> {
    use git2::StatusOptions;

    let mut opts = StatusOptions::new();
    opts.include_untracked(false)
        .include_ignored(false)
        .include_unmodified(false)
        .exclude_submodules(true);

    let statuses = repo.statuses(Some(&mut opts))?;
    Ok(statuses.is_empty())
}

/// Require a clean working directory, returning error if dirty
pub fn require_clean_working_directory(repo: &Repository) -> Result<()> {
    if !is_working_directory_clean(repo)? {
        return Err(GgError::DirtyWorkingDirectory);
    }
    Ok(())
}

/// Get all commits between base and stack tip (in order from base to tip)
/// Returns commit OIDs rather than Commit objects to avoid lifetime issues
/// If `stack_branch` is provided, use that branch instead of HEAD (for detached HEAD mode)
pub fn get_stack_commit_oids(
    repo: &Repository,
    base_branch: &str,
    stack_branch: Option<&str>,
) -> Result<Vec<Oid>> {
    // Get the tip of the stack - either from a branch or from HEAD
    let tip_oid = if let Some(branch) = stack_branch {
        let branch_ref = format!("refs/heads/{}", branch);
        repo.revparse_single(&branch_ref)?.id()
    } else {
        repo.head()?.peel_to_commit()?.id()
    };

    let base_ref = repo
        .revparse_single(base_branch)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", base_branch)))
        .map_err(|_| GgError::NoBaseBranch)?;

    let base_oid = base_ref.id();

    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
    revwalk.push(tip_oid)?;
    revwalk.hide(base_oid)?;

    let mut oids = Vec::new();
    for oid_result in revwalk {
        let oid = oid_result?;
        let commit = repo.find_commit(oid)?;

        // Check for merge commits
        if commit.parent_count() > 1 {
            return Err(GgError::MergeCommitInStack);
        }

        oids.push(oid);
    }

    Ok(oids)
}

/// Extract the GG-ID from a commit message (case-insensitive)
pub fn get_gg_id(commit: &Commit) -> Option<String> {
    let message = commit.message()?;
    let re = Regex::new(r"(?i)^GG-ID:\s*(.+)$").ok()?;

    for line in message.lines() {
        if let Some(captures) = re.captures(line.trim()) {
            let raw = captures.get(1).map(|m| m.as_str().trim())?;
            return normalize_gg_id(raw);
        }
    }
    None
}

/// Generate a new GG-ID
pub fn generate_gg_id() -> String {
    format!("c-{}", &uuid::Uuid::new_v4().to_string()[..7])
}

/// Extract the GG-Parent from a commit message (case-insensitive)
pub fn get_gg_parent(commit: &Commit) -> Option<String> {
    let message = commit.message()?;
    let re = Regex::new(r"(?i)^GG-Parent:\s*(.+)$").ok()?;

    for line in message.lines() {
        if let Some(captures) = re.captures(line.trim()) {
            let raw = captures.get(1).map(|m| m.as_str().trim())?;
            return normalize_gg_id(raw);
        }
    }
    None
}

/// Add or update GG-ID trailer in a message
pub fn set_gg_id_in_message(message: &str, gg_id: &str) -> String {
    let re = Regex::new(r"(?im)^GG-ID:\s*.+$").unwrap();

    if re.is_match(message) {
        // Replace existing GG-ID
        re.replace(message, format!("{} {}", GG_ID_PREFIX, gg_id))
            .to_string()
    } else {
        // Append GG-ID trailer
        let trimmed = message.trim_end();
        format!("{}\n\n{} {}", trimmed, GG_ID_PREFIX, gg_id)
    }
}

/// Add, update, or remove GG-Parent trailer in a message
pub fn set_gg_parent_in_message(message: &str, gg_parent: Option<&str>) -> String {
    let re = Regex::new(r"(?im)^GG-Parent:\s*.+\n?").unwrap();
    let without_parent = re.replace_all(message, "").to_string();
    let trimmed = without_parent.trim_end();

    match gg_parent {
        Some(parent) => {
            if trimmed.is_empty() {
                format!("{} {}", GG_PARENT_PREFIX, parent)
            } else {
                format!("{}\n\n{} {}", trimmed, GG_PARENT_PREFIX, parent)
            }
        }
        None => trimmed.to_string(),
    }
}

/// Strip GG-ID trailer from a message (for MR titles/descriptions)
pub fn strip_gg_id_from_message(message: &str) -> String {
    let re = Regex::new(r"(?im)^GG-ID:\s*.+\n?").unwrap();
    let result = re.replace_all(message, "");
    result.trim_end().to_string()
}

/// Strip GG-Parent trailer from a message
pub fn strip_gg_parent_from_message(message: &str) -> String {
    set_gg_parent_in_message(message, None)
}

/// Normalize GG-ID + GG-Parent trailers in one pass.
/// Returns the rewritten message and whether each trailer was changed.
pub fn normalize_gg_metadata_in_message(
    message: &str,
    expected_gg_id: &str,
    expected_parent: Option<&str>,
) -> (String, bool, bool, bool) {
    let had_gg_id = Regex::new(r"(?im)^GG-ID:\s*.+$").unwrap().is_match(message);
    let current_parent = Regex::new(r"(?im)^GG-Parent:\s*(.+)$")
        .unwrap()
        .captures_iter(message)
        .next()
        .and_then(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
        .and_then(|id| normalize_gg_id(&id));

    let with_id = set_gg_id_in_message(message, expected_gg_id);
    let normalized = set_gg_parent_in_message(&with_id, expected_parent);

    let id_added = !had_gg_id;
    let parent_removed = expected_parent.is_none() && current_parent.is_some();
    let parent_updated = expected_parent.is_some() && current_parent.as_deref() != expected_parent;

    (normalized, id_added, parent_updated, parent_removed)
}

/// Rewrite stack commit metadata to enforce GG-ID and GG-Parent invariants.
pub fn normalize_stack_metadata(
    repo: &Repository,
    stack: &crate::stack::Stack,
) -> Result<MetadataRewriteCounts> {
    if stack.entries.is_empty() {
        return Ok(MetadataRewriteCounts::default());
    }

    // In detached-HEAD mode, remember the original OID so we can remap HEAD to
    // the rewritten commit when that commit is part of this stack rewrite.
    let detached_head_oid = if repo.head_detached()? {
        Some(repo.head()?.peel_to_commit()?.id())
    } else {
        None
    };

    let mut rewritten_oids = std::collections::HashMap::<Oid, Oid>::new();
    let mut previous_gg_id: Option<String> = None;
    let mut counts = MetadataRewriteCounts::default();
    let mut tip_oid: Option<Oid> = None;

    for entry in &stack.entries {
        let original_commit = repo.find_commit(entry.oid)?;
        let original_message = original_commit.message().unwrap_or("");
        let current_gg_id = get_gg_id(&original_commit);
        let effective_gg_id = current_gg_id.clone().unwrap_or_else(generate_gg_id);

        let (new_message, id_added, parent_updated, parent_removed) =
            normalize_gg_metadata_in_message(
                original_message,
                &effective_gg_id,
                previous_gg_id.as_deref(),
            );
        if id_added {
            counts.gg_ids_added += 1;
        }
        if parent_updated {
            counts.gg_parents_updated += 1;
        }
        if parent_removed {
            counts.gg_parents_removed += 1;
        }

        let mut parent_oids: Vec<Oid> = original_commit.parent_ids().collect();
        if let Some(first_parent) = parent_oids.first_mut() {
            if let Some(rewritten_parent) = rewritten_oids.get(first_parent) {
                *first_parent = *rewritten_parent;
            }
        }

        let parent_changed = original_commit
            .parent_id(0)
            .ok()
            .zip(parent_oids.first().copied())
            .is_some_and(|(old_parent, new_parent)| old_parent != new_parent);

        let new_oid = if new_message != original_message || parent_changed {
            let parents: Result<Vec<_>> = parent_oids
                .iter()
                .map(|oid| repo.find_commit(*oid).map_err(GgError::Git))
                .collect();
            let parents = parents?;
            let parent_refs: Vec<&Commit> = parents.iter().collect();

            repo.commit(
                None,
                &original_commit.author(),
                &original_commit.committer(),
                &new_message,
                &original_commit.tree()?,
                &parent_refs,
            )?
        } else {
            original_commit.id()
        };

        rewritten_oids.insert(original_commit.id(), new_oid);
        previous_gg_id = Some(effective_gg_id);
        tip_oid = Some(new_oid);
    }

    let target_refname = {
        let stack_refname = format!("refs/heads/{}/{}", stack.username, stack.name);
        if repo.find_reference(&stack_refname).is_ok() {
            stack_refname
        } else {
            let head = repo.head()?;
            if let Some(branch_name) = head.shorthand() {
                format!("refs/heads/{}", branch_name)
            } else {
                return Err(GgError::Other(format!(
                    "Cannot normalize metadata: missing stack branch ref {} while HEAD is detached",
                    stack_refname
                )));
            }
        }
    };

    if let Some(tip_oid) = tip_oid {
        repo.reference(&target_refname, tip_oid, true, "gg: normalize GG metadata")?;
    }

    if let Some(remapped_head_oid) = detached_head_oid
        .and_then(|original_head_oid| rewritten_oids.get(&original_head_oid).copied())
    {
        repo.set_head_detached(remapped_head_oid)?;
    }

    Ok(counts)
}

fn extract_description_from_message(message: &str) -> Option<String> {
    let stripped = strip_gg_parent_from_message(&strip_gg_id_from_message(message));
    let newline_idx = stripped.find('\n')?;
    let description = stripped[newline_idx + 1..].trim();
    if description.is_empty() {
        None
    } else {
        Some(description.to_string())
    }
}

/// Get the commit message title (first line)
pub fn get_commit_title(commit: &Commit) -> String {
    commit.summary().unwrap_or("<no summary>").to_string()
}

/// Get the commit message description (body), if present
pub fn get_commit_description(commit: &Commit) -> Option<String> {
    let message = commit.message()?;
    extract_description_from_message(message)
}

/// Checkout a branch by name
pub fn checkout_branch(repo: &Repository, branch_name: &str) -> Result<()> {
    let refname = format!("refs/heads/{}", branch_name);
    let obj = repo.revparse_single(&refname)?;

    repo.checkout_tree(&obj, None)?;
    repo.set_head(&refname)?;
    Ok(())
}

/// Ensure HEAD is attached to the given branch.
///
/// In git worktrees, `git rebase ... <branch>` can leave HEAD detached even
/// though the branch ref has been updated correctly. This function detects
/// that situation and re-attaches HEAD to the branch using `git symbolic-ref`.
///
/// Safety: only re-attaches when HEAD and the branch tip point to the same
/// commit, so it won't silently move HEAD to an unexpected location.
///
/// Note: we use `git symbolic-ref` instead of `repo.set_head()` because the
/// latter performs a `git_branch_is_checked_out` check that refuses to point
/// HEAD at a branch that is checked out in another worktree — which is exactly
/// the scenario we need to fix.
pub fn ensure_branch_attached(repo: &Repository, branch_name: &str) -> Result<()> {
    if repo.head_detached()? {
        let head_oid = repo.head()?.peel_to_commit()?.id();
        let refname = format!("refs/heads/{}", branch_name);
        // Only re-attach if branch tip matches HEAD
        if let Ok(branch_ref) = repo.find_reference(&refname) {
            if let Ok(branch_commit) = branch_ref.peel_to_commit() {
                if head_oid == branch_commit.id() {
                    run_git_command(&["symbolic-ref", "HEAD", &refname])?;
                }
            }
        }
    }
    Ok(())
}

/// Move a branch to point at the current HEAD commit
pub fn move_branch_to_head(repo: &Repository, branch_name: &str) -> Result<()> {
    let head_oid = repo.head()?.peel_to_commit()?.id();
    let mut reference = repo.find_reference(&format!("refs/heads/{}", branch_name))?;
    reference.set_target(head_oid, "gg lint: update branch")?;
    Ok(())
}

/// Checkout a specific commit (detached HEAD)
pub fn checkout_commit(repo: &Repository, commit: &Commit) -> Result<()> {
    let obj = commit.as_object();
    repo.checkout_tree(obj, None)?;
    repo.set_head_detached(commit.id())?;
    Ok(())
}

/// Get the repository signature
pub fn get_signature(repo: &Repository) -> Result<Signature<'static>> {
    repo.signature().map_err(GgError::Git)
}

/// Run git command as subprocess (for operations git2 doesn't support well)
pub fn run_git_command(args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(GgError::Other(format!(
            "git {} failed: {}",
            args.join(" "),
            stderr
        )))
    }
}

/// Fetch from origin and prune stale remote-tracking refs
/// This ensures we have up-to-date remote state before operations like sync
pub fn fetch_and_prune() -> Result<()> {
    // Using subprocess because git2's fetch requires complex auth callback setup
    let _ = std::process::Command::new("git")
        .args(["fetch", "origin", "--prune"])
        .output();
    Ok(())
}

/// Get the OID of a remote branch, if it exists
/// Returns None if the remote branch doesn't exist
pub fn get_remote_branch_oid(repo: &Repository, branch_name: &str) -> Option<Oid> {
    let remote_ref = format!("refs/remotes/origin/{}", branch_name);
    repo.revparse_single(&remote_ref).ok().map(|obj| obj.id())
}

/// Count how many commits `local_ref` is behind `upstream_ref`.
///
/// Returns an error if either ref cannot be resolved.
pub fn count_commits_behind(
    repo: &Repository,
    local_ref: &str,
    upstream_ref: &str,
) -> Result<usize> {
    let local_oid = repo.revparse_single(local_ref)?.id();
    let upstream_oid = repo.revparse_single(upstream_ref)?.id();
    let (_ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid)?;
    Ok(behind)
}

/// Count how many commits on `upstream_ref` are not reachable from `local_ref`.
///
/// This uses merge-base to find the fork point, then counts commits from
/// merge-base to upstream. This is useful for determining if a branch needs
/// rebasing regardless of what local tracking branches look like.
///
/// Returns an error if either ref cannot be resolved or if no merge-base exists.
pub fn count_branch_behind_upstream(
    repo: &Repository,
    local_ref: &str,
    upstream_ref: &str,
) -> Result<usize> {
    let local_oid = repo.revparse_single(local_ref)?.id();
    let upstream_oid = repo.revparse_single(upstream_ref)?.id();

    let merge_base = repo.merge_base(local_oid, upstream_oid).map_err(|_| {
        GgError::Other("No merge-base found between branch and upstream".to_string())
    })?;

    // Count commits between merge_base and upstream
    let (_ahead, behind) = repo.graph_ahead_behind(merge_base, upstream_oid)?;
    Ok(behind)
}

/// Build the argv passed to `git push`.
///
/// Order: `push [--force | --force-with-lease] [--no-verify] origin <branch>`.
/// `hard_force` wins over `force_with_lease` when both are true, matching
/// `push_branch`'s existing contract.
fn build_push_args(
    branch_name: &str,
    force_with_lease: bool,
    hard_force: bool,
    no_verify: bool,
) -> Vec<&str> {
    let mut args: Vec<&str> = vec!["push"];
    if hard_force {
        args.push("--force");
    } else if force_with_lease {
        args.push("--force-with-lease");
    }
    if no_verify {
        args.push("--no-verify");
    }
    args.push("origin");
    args.push(branch_name);
    args
}

/// Push a branch to origin
///
/// - `force_with_lease`: Use --force-with-lease (safe force, recommended for stacked diffs)
/// - `hard_force`: Use --force (overrides force_with_lease, use only as escape hatch)
/// - `no_verify`: Forward `--no-verify` to `git push` (skips the `pre-push` hook only)
///
/// If force_with_lease fails with "stale info", retries without lease since
/// the remote branch may have been deleted (e.g., after a PR was merged).
/// The retry path honors `no_verify` the same way.
pub fn push_branch(
    branch_name: &str,
    force_with_lease: bool,
    hard_force: bool,
    no_verify: bool,
) -> Result<()> {
    let args = build_push_args(branch_name, force_with_lease, hard_force, no_verify);

    let output = Command::new("git").args(&args).output()?;

    if output.status.success() {
        return Ok(());
    }

    // Push failed - parse stderr for better error messages
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr_str = stderr.trim();

    // Check for "stale info" error (force-with-lease conflict)
    if force_with_lease && !hard_force && stderr_str.contains("stale info") {
        // Check if we're in a non-interactive environment
        if !atty::is(atty::Stream::Stdin) {
            return Err(GgError::Other(format!(
                "Remote branch '{}' has been updated since your last fetch.\n\
                 This could mean someone else has pushed changes.\n\
                 \n\
                 To proceed safely:\n\
                 1. Run 'git fetch origin'\n\
                 2. Review the changes\n\
                 3. Run 'gg sync' again\n\
                 \n\
                 If you're certain you want to overwrite remote changes, run with --force flag.",
                branch_name
            )));
        }

        // In interactive mode, prompt user for confirmation
        use dialoguer::Confirm;
        eprintln!(
            "\n{} Remote branch '{}' has been updated since your last fetch.",
            console::style("Warning:").yellow().bold(),
            branch_name
        );
        eprintln!("This could mean someone else has pushed changes to this branch.");
        eprintln!();

        let should_force = Confirm::new()
            .with_prompt(format!(
                "Do you want to force-push and overwrite remote changes on '{}'?",
                branch_name
            ))
            .default(false)
            .interact()
            .unwrap_or(false);

        if !should_force {
            return Err(GgError::Other(
                "Push cancelled. Run 'git fetch origin' to update your local state.".to_string(),
            ));
        }

        // User confirmed, proceed with force push
        eprintln!("{}", console::style("Force-pushing...").dim());
        let retry_args = build_push_args(branch_name, false, true, no_verify);
        return run_git_command(&retry_args).map(|_| ());
    }

    // Parse stderr to extract hook errors vs git errors
    let (hook_error, git_error) = parse_push_error(stderr_str);

    Err(GgError::PushFailed {
        branch: branch_name.to_string(),
        hook_error,
        git_error,
    })
}

/// Parse git push stderr to separate hook errors from git errors
fn parse_push_error(stderr: &str) -> (Option<String>, Option<String>) {
    let lines: Vec<&str> = stderr.lines().collect();

    // Look for pre-push hook output (usually appears before "error: failed to push")
    // Hook output typically doesn't start with "error:" or "remote:"
    let mut hook_lines = Vec::new();
    let mut git_lines = Vec::new();

    for line in lines {
        let trimmed = line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Git error messages typically start with "error:" or "remote:"
        if trimmed.starts_with("error:") || trimmed.starts_with("remote:") {
            // Skip the generic "failed to push some refs" message (it's redundant)
            if !trimmed.contains("failed to push some refs") {
                git_lines.push(trimmed);
            }
        } else {
            // Assume this is hook output
            hook_lines.push(trimmed);
        }
    }

    let hook_error = if hook_lines.is_empty() {
        None
    } else {
        Some(hook_lines.join("\n"))
    };

    let git_error = if git_lines.is_empty() {
        None
    } else {
        Some(git_lines.join("\n"))
    };

    (hook_error, git_error)
}

/// Delete a remote branch
pub fn delete_remote_branch(branch_name: &str) -> Result<()> {
    run_git_command(&["push", "origin", "--delete", branch_name])?;
    Ok(())
}

/// Continue a rebase
pub fn rebase_continue() -> Result<()> {
    // Set GIT_EDITOR=true to avoid "Terminal is dumb, but EDITOR unset" errors
    // This allows rebase to continue without requiring an interactive editor
    let output = Command::new("git")
        .args(["rebase", "--continue"])
        .env("GIT_EDITOR", "true")
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(GgError::Other(format!(
            "git rebase --continue failed: {}",
            stderr
        )))
    }
}

/// Abort a rebase
pub fn rebase_abort() -> Result<()> {
    run_git_command(&["rebase", "--abort"])?;
    Ok(())
}

/// Check if a rebase is in progress
pub fn is_rebase_in_progress(repo: &Repository) -> bool {
    let git_dir = repo.path();
    git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists()
}

/// Get short SHA from a commit
pub fn short_sha(commit: &Commit) -> String {
    commit.id().to_string()[..7].to_string()
}

/// Sanitize and validate a stack name
///
/// - Converts spaces to hyphens (kebab-case)
/// - Rejects names containing `/` (conflicts with branch format)
/// - Rejects names containing `--` (conflicts with entry branch format)
/// - Rejects names with other invalid git ref characters
///
/// Returns the sanitized name or an error
pub fn sanitize_stack_name(name: &str) -> Result<String> {
    // First, convert spaces to hyphens
    let sanitized = name.replace(' ', "-");

    // Check for reserved sequences that conflict with gg branch format
    if sanitized.contains('/') {
        return Err(GgError::InvalidStackName(
            "Stack name cannot contain '/' (conflicts with branch format)".to_string(),
        ));
    }

    if sanitized.contains("--") {
        return Err(GgError::InvalidStackName(
            "Stack name cannot contain '--' (conflicts with entry branch format)".to_string(),
        ));
    }

    // Check for other invalid git ref characters
    // Git ref names cannot contain: space, ~, ^, :, ?, *, [, \, control chars
    // Also cannot start or end with dot, contain "..", or end with ".lock"
    let invalid_chars = ['~', '^', ':', '?', '*', '[', '\\', '@'];
    for c in invalid_chars {
        if sanitized.contains(c) {
            return Err(GgError::InvalidStackName(format!(
                "Stack name cannot contain '{}'",
                c
            )));
        }
    }

    // Check for control characters
    if sanitized.chars().any(|c| c.is_control()) {
        return Err(GgError::InvalidStackName(
            "Stack name cannot contain control characters".to_string(),
        ));
    }

    // Check for ".." sequence
    if sanitized.contains("..") {
        return Err(GgError::InvalidStackName(
            "Stack name cannot contain '..'".to_string(),
        ));
    }

    // Check for starting/ending with dot
    if sanitized.starts_with('.') || sanitized.ends_with('.') {
        return Err(GgError::InvalidStackName(
            "Stack name cannot start or end with '.'".to_string(),
        ));
    }

    // Check for ending with .lock
    if sanitized.ends_with(".lock") {
        return Err(GgError::InvalidStackName(
            "Stack name cannot end with '.lock'".to_string(),
        ));
    }

    // Check for empty name after sanitization
    if sanitized.is_empty() || sanitized.chars().all(|c| c == '-') {
        return Err(GgError::InvalidStackName(
            "Stack name cannot be empty".to_string(),
        ));
    }

    Ok(sanitized)
}

/// Validate branch username used in branch names.
///
/// The username must not contain `/` or other invalid git ref characters.
pub fn validate_branch_username(username: &str) -> Result<()> {
    if username.is_empty() {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot be empty".to_string(),
        ));
    }

    if username.contains('/') {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot contain '/'".to_string(),
        ));
    }

    if username.chars().any(|c| c.is_whitespace()) {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot contain whitespace".to_string(),
        ));
    }

    let invalid_chars = ['~', '^', ':', '?', '*', '[', '\\', '@'];
    for c in invalid_chars {
        if username.contains(c) {
            return Err(GgError::InvalidBranchUsername(format!(
                "Branch username cannot contain '{}'",
                c
            )));
        }
    }

    if username.chars().any(|c| c.is_control()) {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot contain control characters".to_string(),
        ));
    }

    if username.contains("..") {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot contain '..'".to_string(),
        ));
    }

    if username.starts_with('.') || username.ends_with('.') {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot start or end with '.'".to_string(),
        ));
    }

    if username.ends_with(".lock") {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot end with '.lock'".to_string(),
        ));
    }

    Ok(())
}

/// Validate GG-ID format and normalize to lowercase.
pub fn normalize_gg_id(gg_id: &str) -> Option<String> {
    let re = Regex::new(r"(?i)^c-[0-9a-f]{7}$").ok()?;
    if re.is_match(gg_id) {
        Some(gg_id.to_lowercase())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    use crate::stack::{Stack, StackEntry};

    fn run_git(repo_path: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn test_sanitize_stack_name() {
        // Spaces converted to hyphens
        assert_eq!(
            sanitize_stack_name("my new feature").unwrap(),
            "my-new-feature"
        );

        // Already valid
        assert_eq!(sanitize_stack_name("my-feature").unwrap(), "my-feature");

        // Invalid: contains /
        assert!(sanitize_stack_name("my/feature").is_err());

        // Invalid: contains --
        assert!(sanitize_stack_name("my--feature").is_err());

        // Invalid: contains special chars
        assert!(sanitize_stack_name("my~feature").is_err());
        assert!(sanitize_stack_name("my:feature").is_err());
        assert!(sanitize_stack_name("my*feature").is_err());

        // Invalid: starts/ends with dot
        assert!(sanitize_stack_name(".myfeature").is_err());
        assert!(sanitize_stack_name("myfeature.").is_err());

        // Invalid: contains ..
        assert!(sanitize_stack_name("my..feature").is_err());

        // Invalid: ends with .lock
        assert!(sanitize_stack_name("feature.lock").is_err());

        // Invalid: empty
        assert!(sanitize_stack_name("").is_err());
        assert!(sanitize_stack_name("   ").is_err());
    }

    #[test]
    fn test_validate_branch_username() {
        assert!(validate_branch_username("nacho").is_ok());
        assert!(validate_branch_username("nacho-lopez").is_ok());
        assert!(validate_branch_username("nacho.lopez").is_ok());

        assert!(validate_branch_username("").is_err());
        assert!(validate_branch_username("na/cho").is_err());
        assert!(validate_branch_username("na cho").is_err());
        assert!(validate_branch_username("na..cho").is_err());
        assert!(validate_branch_username(".nacho").is_err());
        assert!(validate_branch_username("nacho.").is_err());
        assert!(validate_branch_username("nacho.lock").is_err());
        assert!(validate_branch_username("nacho@home").is_err());
    }

    #[test]
    fn test_normalize_gg_id() {
        assert_eq!(normalize_gg_id("c-abc1234"), Some("c-abc1234".to_string()));
        assert_eq!(normalize_gg_id("C-ABC1234"), Some("c-abc1234".to_string()));
        assert_eq!(normalize_gg_id("c-abcdefg"), None);
        assert_eq!(normalize_gg_id("c-123456"), None);
        assert_eq!(normalize_gg_id("invalid"), None);
    }

    #[test]
    fn test_parse_stack_branch() {
        assert_eq!(
            parse_stack_branch("nacho/my-feature"),
            Some(("nacho".to_string(), "my-feature".to_string()))
        );
        assert_eq!(parse_stack_branch("main"), None);
        assert_eq!(parse_stack_branch("nacho/my-feature/c-abc123"), None);
        // Entry branches should NOT be parsed as stack branches
        assert_eq!(parse_stack_branch("nacho/my-feature--c-abc123"), None);
        assert_eq!(parse_stack_branch("nacho/claude.md--c-7d3f2a6"), None);
    }

    #[test]
    fn test_format_entry_branch() {
        assert_eq!(
            format_entry_branch("nacho", "my-feature", "c-abc123"),
            "nacho/my-feature--c-abc123"
        );
    }

    #[test]
    fn test_parse_entry_branch() {
        // Format: username/stack_name--entry_id
        assert_eq!(
            parse_entry_branch("nacho/my-feature--c-abc123"),
            Some((
                "nacho".to_string(),
                "my-feature".to_string(),
                "c-abc123".to_string()
            ))
        );
        assert_eq!(parse_entry_branch("main"), None);
        assert_eq!(parse_entry_branch("nacho/my-feature"), None);
        // Old format with slash should not match
        assert_eq!(parse_entry_branch("nacho/my-feature/c-abc123"), None);
    }

    #[test]
    fn test_set_gg_id_in_message() {
        let msg = "Add feature\n\nSome description";
        let result = set_gg_id_in_message(msg, "c-abc123");
        assert!(result.contains("GG-ID: c-abc123"));

        // Test replacement
        let msg_with_id = "Add feature\n\nGG-ID: c-old123";
        let result = set_gg_id_in_message(msg_with_id, "c-new456");
        assert!(result.contains("GG-ID: c-new456"));
        assert!(!result.contains("c-old123"));
    }

    #[test]
    fn test_set_gg_parent_in_message() {
        let msg = "Add feature\n\nSome description\n\nGG-ID: c-abc123";
        let result = set_gg_parent_in_message(msg, Some("c-parent1"));
        assert!(result.contains("GG-Parent: c-parent1"));

        let replaced = set_gg_parent_in_message(&result, Some("c-parent2"));
        assert!(replaced.contains("GG-Parent: c-parent2"));
        assert!(!replaced.contains("c-parent1"));
    }

    #[test]
    fn test_strip_gg_parent_from_message() {
        let msg = "Title\n\nBody\n\nGG-ID: c-abc123\nGG-Parent: c-def456";
        let stripped = strip_gg_parent_from_message(msg);
        assert!(!stripped.contains("GG-Parent:"));
        assert!(stripped.contains("GG-ID: c-abc123"));
    }

    #[test]
    fn test_normalize_gg_metadata_in_message() {
        let msg = "Title\n\nBody";
        let (normalized, id_added, parent_updated, parent_removed) =
            normalize_gg_metadata_in_message(msg, "c-abc1234", Some("c-parent1"));
        assert!(normalized.contains("GG-ID: c-abc1234"));
        assert!(normalized.contains("GG-Parent: c-parent1"));
        assert!(id_added);
        assert!(parent_updated);
        assert!(!parent_removed);
    }

    #[test]
    fn test_strip_gg_id_from_message() {
        let msg = "Add feature\n\nSome description\n\nGG-ID: c-abc123";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.contains("GG-ID"));
        assert!(result.contains("Add feature"));
        assert!(result.contains("Some description"));
    }

    #[test]
    fn test_normalize_stack_metadata_preserves_patch_content_when_base_advanced() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_path = temp_dir.path();

        run_git(repo_path, &["init", "--initial-branch=main"]);
        run_git(repo_path, &["config", "user.email", "test@example.com"]);
        run_git(repo_path, &["config", "user.name", "Test User"]);

        std::fs::write(repo_path.join("shared.txt"), "base-1\n").unwrap();
        run_git(repo_path, &["add", "."]);
        run_git(repo_path, &["commit", "-m", "base 1"]);
        let base_before_oid = Oid::from_str(
            String::from_utf8_lossy(
                &Command::new("git")
                    .args(["rev-parse", "HEAD"])
                    .current_dir(repo_path)
                    .output()
                    .unwrap()
                    .stdout,
            )
            .trim(),
        )
        .unwrap();

        run_git(repo_path, &["checkout", "-b", "nacho/stack"]);

        std::fs::write(repo_path.join("stack.txt"), "stack-1\n").unwrap();
        run_git(repo_path, &["add", "."]);
        run_git(
            repo_path,
            &["commit", "-m", "stack 1\n\nBody\n\nGG-ID: c-1111111"],
        );
        let stack1_oid = Oid::from_str(
            String::from_utf8_lossy(
                &Command::new("git")
                    .args(["rev-parse", "HEAD"])
                    .current_dir(repo_path)
                    .output()
                    .unwrap()
                    .stdout,
            )
            .trim(),
        )
        .unwrap();

        std::fs::write(repo_path.join("stack.txt"), "stack-2\n").unwrap();
        run_git(repo_path, &["add", "."]);
        run_git(
            repo_path,
            &[
                "commit",
                "-m",
                "stack 2\n\nBody\n\nGG-ID: c-2222222\nGG-Parent: c-deadbee",
            ],
        );
        let stack2_oid = Oid::from_str(
            String::from_utf8_lossy(
                &Command::new("git")
                    .args(["rev-parse", "HEAD"])
                    .current_dir(repo_path)
                    .output()
                    .unwrap()
                    .stdout,
            )
            .trim(),
        )
        .unwrap();

        run_git(repo_path, &["checkout", "main"]);
        std::fs::write(repo_path.join("shared.txt"), "base-2\n").unwrap();
        run_git(repo_path, &["add", "."]);
        run_git(repo_path, &["commit", "-m", "base 2"]);

        run_git(repo_path, &["checkout", "nacho/stack"]);

        let repo = Repository::open(repo_path).unwrap();
        let original_stack1 = repo.find_commit(stack1_oid).unwrap();
        let original_stack2 = repo.find_commit(stack2_oid).unwrap();

        let stack = Stack {
            name: "stack".to_string(),
            username: "nacho".to_string(),
            base: "main".to_string(),
            entries: vec![
                StackEntry::from_commit(&original_stack1, 1),
                StackEntry::from_commit(&original_stack2, 2),
            ],
            current_position: Some(1),
        };

        let counts = normalize_stack_metadata(&repo, &stack).unwrap();
        assert_eq!(
            counts,
            MetadataRewriteCounts {
                gg_ids_added: 0,
                gg_parents_updated: 1,
                gg_parents_removed: 0,
            }
        );

        let new_head = repo.head().unwrap().peel_to_commit().unwrap();
        let rewritten_stack1 = new_head.parent(0).unwrap();

        assert_eq!(rewritten_stack1.parent_id(0).unwrap(), base_before_oid);
        assert_eq!(rewritten_stack1.tree_id(), original_stack1.tree_id());
        assert_eq!(new_head.tree_id(), original_stack2.tree_id());

        let new_head_message = new_head.message().unwrap();
        assert!(new_head_message.contains("GG-ID: c-2222222"));
        assert!(new_head_message.contains("GG-Parent: c-1111111"));
        assert!(!new_head_message.contains("c-deadbee"));
    }

    #[test]
    fn test_normalize_stack_metadata_uses_stack_branch_and_remaps_detached_head() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_path = temp_dir.path();

        run_git(repo_path, &["init", "--initial-branch=main"]);
        run_git(repo_path, &["config", "user.email", "test@example.com"]);
        run_git(repo_path, &["config", "user.name", "Test User"]);

        std::fs::write(repo_path.join("base.txt"), "base\n").unwrap();
        run_git(repo_path, &["add", "."]);
        run_git(repo_path, &["commit", "-m", "base"]);

        run_git(repo_path, &["checkout", "-b", "nacho/stack"]);

        std::fs::write(repo_path.join("stack.txt"), "stack-1\n").unwrap();
        run_git(repo_path, &["add", "."]);
        run_git(
            repo_path,
            &["commit", "-m", "stack 1\n\nBody\n\nGG-ID: c-1111111"],
        );

        std::fs::write(repo_path.join("stack.txt"), "stack-2\n").unwrap();
        run_git(repo_path, &["add", "."]);
        run_git(
            repo_path,
            &[
                "commit",
                "-m",
                "stack 2\n\nBody\n\nGG-ID: c-2222222\nGG-Parent: c-deadbee",
            ],
        );

        let stack_tip_before = Oid::from_str(
            String::from_utf8_lossy(
                &Command::new("git")
                    .args(["rev-parse", "nacho/stack"])
                    .current_dir(repo_path)
                    .output()
                    .unwrap()
                    .stdout,
            )
            .trim(),
        )
        .unwrap();

        run_git(repo_path, &["checkout", "--detach", "HEAD"]);

        let repo = Repository::open(repo_path).unwrap();
        assert!(repo.head_detached().unwrap());

        let stack_tip_commit = repo.find_commit(stack_tip_before).unwrap();
        let stack_parent_commit = stack_tip_commit.parent(0).unwrap();

        let stack = Stack {
            name: "stack".to_string(),
            username: "nacho".to_string(),
            base: "main".to_string(),
            entries: vec![
                StackEntry::from_commit(&stack_parent_commit, 1),
                StackEntry::from_commit(&stack_tip_commit, 2),
            ],
            current_position: Some(1),
        };

        let counts = normalize_stack_metadata(&repo, &stack).unwrap();
        assert_eq!(
            counts,
            MetadataRewriteCounts {
                gg_ids_added: 0,
                gg_parents_updated: 1,
                gg_parents_removed: 0,
            }
        );

        let detached_head_after = repo.head().unwrap().peel_to_commit().unwrap();

        let updated_stack_tip = repo
            .find_reference("refs/heads/nacho/stack")
            .unwrap()
            .peel_to_commit()
            .unwrap();

        assert_eq!(detached_head_after.id(), updated_stack_tip.id());
        assert_ne!(detached_head_after.id(), stack_tip_before);

        let updated_message = updated_stack_tip.message().unwrap();
        assert!(updated_message.contains("GG-ID: c-2222222"));
        assert!(updated_message.contains("GG-Parent: c-1111111"));
        assert!(!updated_message.contains("c-deadbee"));
        assert_ne!(updated_stack_tip.id(), stack_tip_before);
    }

    #[test]
    fn test_strip_gg_id_edge_cases() {
        // Case insensitive
        let msg = "Title\n\nBody\n\ngg-id: c-abc123";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.to_lowercase().contains("gg-id"));
        assert_eq!(result, "Title\n\nBody");

        // Mixed case
        let msg = "Title\n\nBody\n\nGg-Id: c-abc123";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.to_lowercase().contains("gg-id"));

        // Multiple GG-IDs (shouldn't happen but should handle)
        let msg = "Title\n\nBody\n\nGG-ID: c-abc123\nGG-ID: c-def456";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.contains("GG-ID"));
        assert_eq!(result, "Title\n\nBody");

        // GG-ID with extra spaces after colon
        let msg = "Title\n\nBody\n\nGG-ID:   c-abc123";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.contains("GG-ID"));

        // GG-ID only (no body)
        let msg = "Title\n\nGG-ID: c-abc123";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.contains("GG-ID"));
        assert_eq!(result, "Title");

        // GG-ID in middle of body (rare but possible)
        let msg = "Title\n\nFirst paragraph\n\nGG-ID: c-abc123\n\nSecond paragraph";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.contains("GG-ID"));
        assert!(result.contains("First paragraph"));
        assert!(result.contains("Second paragraph"));
    }

    #[test]
    fn test_extract_description_from_message() {
        let msg = "Add feature\n\nSome description\n\nGG-ID: c-abc123";
        let result = extract_description_from_message(msg);
        assert_eq!(result.as_deref(), Some("Some description"));

        let msg_no_body = "Add feature\n\nGG-ID: c-abc123";
        let result = extract_description_from_message(msg_no_body);
        assert!(result.is_none());

        let msg_multi = "Add feature\n\nLine one\n\nLine two";
        let result = extract_description_from_message(msg_multi);
        assert_eq!(result.as_deref(), Some("Line one\n\nLine two"));
    }

    #[test]
    fn test_extract_description_filters_gg_id() {
        // Ensure GG-ID is never present in extracted description
        // This is critical for PR/MR descriptions

        // GG-ID at end of body
        let msg = "Title\n\nThis is the body.\n\nMore details here.\n\nGG-ID: c-abc123";
        let result = extract_description_from_message(msg);
        assert!(result.is_some());
        let desc = result.unwrap();
        assert!(!desc.contains("GG-ID"));
        assert!(desc.contains("This is the body."));
        assert!(desc.contains("More details here."));

        // GG-ID with lowercase
        let msg = "Title\n\nBody text\n\ngg-id: c-abc123";
        let result = extract_description_from_message(msg);
        assert!(result.is_some());
        assert!(!result.unwrap().to_lowercase().contains("gg-id"));

        // Multiple paragraphs with GG-ID at end
        let msg = "feat: add new feature\n\nFirst paragraph explaining the change.\n\nSecond paragraph with more details.\n\n- Bullet point 1\n- Bullet point 2\n\nGG-ID: c-1234567";
        let result = extract_description_from_message(msg);
        assert!(result.is_some());
        let desc = result.unwrap();
        assert!(!desc.contains("GG-ID"));
        assert!(desc.contains("First paragraph"));
        assert!(desc.contains("Bullet point"));

        // GG-Parent should also be stripped from PR/MR descriptions
        let msg = "Title\n\nBody text\n\nGG-ID: c-1234567\nGG-Parent: c-7654321";
        let result = extract_description_from_message(msg);
        assert_eq!(result.as_deref(), Some("Body text"));
    }

    #[test]
    fn test_count_commits_behind() {
        use std::process::Command;

        let temp_dir = tempfile::tempdir().unwrap();
        let repo_path = temp_dir.path();

        Command::new("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        std::fs::write(repo_path.join("file.txt"), "v1").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Create a synthetic origin/main ref 2 commits ahead of main.
        std::fs::write(repo_path.join("file.txt"), "v2").unwrap();
        Command::new("git")
            .args(["commit", "-am", "second"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        std::fs::write(repo_path.join("file.txt"), "v3").unwrap();
        Command::new("git")
            .args(["commit", "-am", "third"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        let head = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        let head_oid = String::from_utf8_lossy(&head.stdout).trim().to_string();

        // Move local main back by two commits and keep origin/main at HEAD.
        Command::new("git")
            .args(["reset", "--hard", "HEAD~2"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["update-ref", "refs/remotes/origin/main", &head_oid])
            .current_dir(repo_path)
            .output()
            .unwrap();

        let repo = Repository::open(repo_path).unwrap();
        let behind = count_commits_behind(&repo, "main", "origin/main").unwrap();
        assert_eq!(behind, 2);
    }

    #[test]
    fn test_count_commits_behind_missing_ref() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(temp_dir.path()).unwrap();

        let err = count_commits_behind(&repo, "main", "origin/main");
        assert!(err.is_err());
    }

    #[test]
    fn test_count_branch_behind_upstream() {
        use std::process::Command;

        let temp_dir = tempfile::tempdir().unwrap();
        let repo_path = temp_dir.path();

        // Initialize repo
        Command::new("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Create initial commit on main
        std::fs::write(repo_path.join("file.txt"), "v1").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Create a feature branch from this point
        Command::new("git")
            .args(["checkout", "-b", "feature"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Make a commit on feature branch
        std::fs::write(repo_path.join("feature.txt"), "feature work").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "feature work"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Go back to main and add commits (simulating upstream progress)
        Command::new("git")
            .args(["checkout", "main"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        std::fs::write(repo_path.join("file.txt"), "v2").unwrap();
        Command::new("git")
            .args(["commit", "-am", "main progress 1"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        std::fs::write(repo_path.join("file.txt"), "v3").unwrap();
        Command::new("git")
            .args(["commit", "-am", "main progress 2"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Create origin/main ref at current main position
        let head = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        let origin_main_oid = String::from_utf8_lossy(&head.stdout).trim().to_string();
        Command::new("git")
            .args(["update-ref", "refs/remotes/origin/main", &origin_main_oid])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Now update local main to match origin/main (simulating `git pull`)
        // Local main is now up-to-date with origin/main

        // Switch to feature branch
        Command::new("git")
            .args(["checkout", "feature"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        let repo = Repository::open(repo_path).unwrap();

        // The OLD approach: compare main vs origin/main
        // This would return 0 because local main == origin/main
        let old_behind = count_commits_behind(&repo, "main", "origin/main").unwrap();
        assert_eq!(old_behind, 0, "local main is up-to-date with origin/main");

        // The NEW approach: use merge-base between HEAD (feature) and origin/main
        // This should return 2 because feature was forked from old main
        let new_behind = count_branch_behind_upstream(&repo, "HEAD", "origin/main").unwrap();
        assert_eq!(
            new_behind, 2,
            "feature branch is 2 commits behind origin/main"
        );
    }

    #[test]
    fn test_count_branch_behind_upstream_already_rebased() {
        use std::process::Command;

        let temp_dir = tempfile::tempdir().unwrap();
        let repo_path = temp_dir.path();

        // Initialize repo
        Command::new("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Create initial commit on main
        std::fs::write(repo_path.join("file.txt"), "v1").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Add more commits to main
        std::fs::write(repo_path.join("file.txt"), "v2").unwrap();
        Command::new("git")
            .args(["commit", "-am", "main progress"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Create origin/main ref at current main position
        let head = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        let origin_main_oid = String::from_utf8_lossy(&head.stdout).trim().to_string();
        Command::new("git")
            .args(["update-ref", "refs/remotes/origin/main", &origin_main_oid])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Create a feature branch FROM the latest main (already rebased)
        Command::new("git")
            .args(["checkout", "-b", "feature"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Make a commit on feature branch
        std::fs::write(repo_path.join("feature.txt"), "feature work").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "feature work"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        let repo = Repository::open(repo_path).unwrap();

        // Branch is already based on latest origin/main, should be 0 behind
        let behind = count_branch_behind_upstream(&repo, "HEAD", "origin/main").unwrap();
        assert_eq!(
            behind, 0,
            "feature branch is already rebased on origin/main"
        );
    }

    #[test]
    fn test_generate_gg_id() {
        let id = generate_gg_id();
        assert!(id.starts_with("c-"));
        assert_eq!(id.len(), 9); // "c-" + 7 chars
    }

    #[test]
    fn test_parse_push_error_with_hook() {
        let stderr = "😢 ktlint found style violations in Kotlin code.\nerror: failed to push some refs to 'gitlab.example.com:user/repo.git'";
        let (hook_error, git_error) = parse_push_error(stderr);

        assert!(hook_error.is_some());
        assert_eq!(
            hook_error.unwrap(),
            "😢 ktlint found style violations in Kotlin code."
        );
        assert!(git_error.is_none()); // Generic "failed to push" is filtered out
    }

    #[test]
    fn test_parse_push_error_hook_only() {
        let stderr = "Pre-push hook failed\nSome detailed error message";
        let (hook_error, git_error) = parse_push_error(stderr);

        assert!(hook_error.is_some());
        assert!(hook_error.unwrap().contains("Pre-push hook failed"));
        assert!(git_error.is_none());
    }

    #[test]
    fn test_parse_push_error_git_only() {
        let stderr = "error: failed to push some refs to 'origin'\nremote: Permission denied";
        let (hook_error, git_error) = parse_push_error(stderr);

        assert!(hook_error.is_none());
        assert!(git_error.is_some());
        assert_eq!(git_error.unwrap(), "remote: Permission denied");
    }

    #[test]
    fn test_parse_push_error_both() {
        let stderr =
            "Hook output line 1\nHook output line 2\nerror: some git error\nremote: remote error";
        let (hook_error, git_error) = parse_push_error(stderr);

        assert!(hook_error.is_some());
        assert!(hook_error.unwrap().contains("Hook output line 1"));
        assert!(git_error.is_some());
        assert!(git_error.unwrap().contains("some git error"));
    }

    #[test]
    fn test_parse_push_error_empty() {
        let stderr = "";
        let (hook_error, git_error) = parse_push_error(stderr);

        assert!(hook_error.is_none());
        assert!(git_error.is_none());
    }

    #[test]
    fn test_ensure_branch_attached_noop_when_on_branch() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();

        // Configure git user for commits
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "test@test.com").unwrap();

        // Create an initial commit
        let sig = repo.signature().unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        // HEAD is on a branch, not detached
        assert!(!repo.head_detached().unwrap());

        let branch_name = repo.head().unwrap().shorthand().unwrap().to_string();

        // Should be a no-op when HEAD is already attached
        ensure_branch_attached(&repo, &branch_name).unwrap();
        assert!(!repo.head_detached().unwrap());
    }

    #[test]
    fn test_ensure_branch_attached_reattaches_detached_head() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();

        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "test@test.com").unwrap();

        let sig = repo.signature().unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let oid = repo
            .commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        let branch_name = repo.head().unwrap().shorthand().unwrap().to_string();

        // Detach HEAD at the same commit as the branch
        repo.set_head_detached(oid).unwrap();
        assert!(repo.head_detached().unwrap());

        // run_git_command uses CWD, so we need to be in the repo dir
        let prev_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        // Should re-attach because HEAD and branch tip match
        let result = ensure_branch_attached(&repo, &branch_name);

        std::env::set_current_dir(&prev_dir).unwrap();

        result.unwrap();
        assert!(!repo.head_detached().unwrap());
    }

    #[test]
    fn test_ensure_branch_attached_skips_when_oids_differ() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();

        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "test@test.com").unwrap();

        let sig = repo.signature().unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let oid1 = repo
            .commit(Some("HEAD"), &sig, &sig, "first", &tree, &[])
            .unwrap();

        let branch_name = repo.head().unwrap().shorthand().unwrap().to_string();

        // Create a second commit, advancing the branch
        let parent = repo.find_commit(oid1).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&parent])
            .unwrap();

        // Detach HEAD at the first commit (behind the branch tip)
        repo.set_head_detached(oid1).unwrap();
        assert!(repo.head_detached().unwrap());

        // Should NOT re-attach because HEAD and branch tip differ
        ensure_branch_attached(&repo, &branch_name).unwrap();
        assert!(repo.head_detached().unwrap()); // still detached
    }

    #[test]
    fn test_operation_lock_succeeds_when_no_index_lock() {
        use std::process::Command;

        let temp_dir = tempfile::tempdir().unwrap();
        let repo_path = temp_dir.path();

        Command::new("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // Create initial commit
        std::fs::write(repo_path.join("file.txt"), "content").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        let repo = Repository::open(repo_path).unwrap();
        let index_lock = repo_path.join(".git/index.lock");

        assert!(
            !index_lock.exists(),
            "index.lock should not exist before lock"
        );

        {
            let lock = acquire_operation_lock(&repo, "test");
            assert!(lock.is_ok(), "Should succeed when no index.lock exists");
        }

        // Lock dropped - we only check for index.lock, we don't create it
        // so there's nothing to verify about index.lock after drop
    }

    #[test]
    fn test_operation_lock_blocks_when_index_lock_exists() {
        use std::process::Command;

        let temp_dir = tempfile::tempdir().unwrap();
        let repo_path = temp_dir.path();

        Command::new("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        std::fs::write(repo_path.join("file.txt"), "content").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        let repo = Repository::open(repo_path).unwrap();
        let index_lock = repo_path.join(".git/index.lock");

        // Create an index.lock file to simulate git holding the lock
        // (git's real index.lock is a binary copy of the index, not text)
        std::fs::write(&index_lock, "binary index data").unwrap();

        // acquire_operation_lock should timeout and fail
        // Use short timeout (1 second) for test speed
        let result = acquire_operation_lock_with_timeout(&repo, "test", 1);
        assert!(result.is_err(), "Should fail when index.lock exists");

        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("git operation is currently in progress") || err.contains("index.lock"),
            "Error should mention git operation in progress. Got: {}",
            err
        );

        // Clean up
        std::fs::remove_file(&index_lock).ok();
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn test_build_push_args_matrix() {
        let cases: &[((bool, bool, bool), &[&str])] = &[
            (
                (true, false, false),
                &["push", "--force-with-lease", "origin", "feat/x"],
            ),
            (
                (true, false, true),
                &[
                    "push",
                    "--force-with-lease",
                    "--no-verify",
                    "origin",
                    "feat/x",
                ],
            ),
            (
                (false, true, false),
                &["push", "--force", "origin", "feat/x"],
            ),
            (
                (false, true, true),
                &["push", "--force", "--no-verify", "origin", "feat/x"],
            ),
            ((false, false, false), &["push", "origin", "feat/x"]),
            (
                (false, false, true),
                &["push", "--no-verify", "origin", "feat/x"],
            ),
        ];

        for ((fwl, hard, no_verify), expected) in cases {
            let got = build_push_args("feat/x", *fwl, *hard, *no_verify);
            assert_eq!(
                got.as_slice(),
                *expected,
                "mismatch for (fwl={}, hard={}, no_verify={})",
                fwl,
                hard,
                no_verify
            );
        }
    }
}

/// Remote provider type
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum RemoteProvider {
    GitHub,
    GitLab,
}

/// Detect the remote provider based on the remote URL
#[allow(dead_code)]
pub fn detect_remote_provider(repo: &Repository) -> Result<RemoteProvider> {
    let remote = repo
        .find_remote("origin")
        .map_err(|_| GgError::Other("No origin remote found".to_string()))?;

    let url = remote
        .url()
        .ok_or_else(|| GgError::Other("Origin remote has no URL".to_string()))?;

    if url.contains("github.com") {
        Ok(RemoteProvider::GitHub)
    } else if url.contains("gitlab.com") {
        Ok(RemoteProvider::GitLab)
    } else {
        // Default to GitLab for self-hosted instances
        Err(GgError::Other(format!(
            "Could not detect remote provider from URL: {}. Supported: github.com, gitlab.com",
            url
        )))
    }
}
