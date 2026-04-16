//! Per-repo operation log for `gg undo`.
//!
//! See `docs/plans/2026-04-17-task-5-gg-undo-op-log-design.md` for the
//! architectural contract (decisions D1–D11).
//!
//! This module owns the record/replay seam for `gg undo`:
//! - [`OperationRecord`] + [`OperationKind`] + [`OperationStatus`]: the durable
//!   schema persisted at `<commondir>/gg/operations/<id>.json`.
//! - [`OperationStore`]: atomic save/load/list/prune.
//! - [`OperationGuard`]: RAII-ish guard returned alongside the operation lock.
//! - [`snapshot_refs`] + [`SnapshotScope`]: ref capture for `refs_before`
//!   / `refs_after`.
//! - [`run_undo`] + [`UndoOptions`] + [`UndoOutcome`]: undo replay.
//!
//! The on-disk schema is versioned by [`SCHEMA_VERSION`]. Optional fields
//! are always `#[serde(default, skip_serializing_if = "…")]` so a future
//! schema version can deserialize under the current version (forward-compat).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{BranchType, Repository};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::stack::Stack;

/// Durable schema version for operation records. Bumping this is a
/// backwards-incompatible change and must be accompanied by a migration.
pub const SCHEMA_VERSION: u32 = 1;

/// Ring buffer cap. Committed/Interrupted records beyond this count are
/// pruned oldest-first. `Pending` records are never pruned — they may
/// belong to a live process.
pub const OPERATION_LOG_CAP: usize = 100;

/// How long a `Pending` record may live before the sweep promotes it to
/// `Interrupted`. 30s is 3× the existing operation-lock timeout.
pub(crate) const PENDING_STALENESS_MS: u64 = 30_000;

/// Reserved top-level branch names that never belong to a user namespace,
/// excluded from `SnapshotScope::AllUserBranches`.
const TRUNK_EXCLUSIONS: &[&str] = &["main", "master", "trunk"];

// ---------------------------------------------------------------------------
// Record schema
// ---------------------------------------------------------------------------

/// The kind of operation a record represents. Serialized as snake_case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Drop,
    Squash,
    Split,
    Rebase,
    Reorder,
    Absorb,
    Checkout,
    Nav,
    Sync,
    Land,
    Clean,
    Reconcile,
    Run,
    Undo,
}

/// Lifecycle status of an operation record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    /// Mutation started but `finalize` not yet called. May be an
    /// in-flight process or a crashed one.
    Pending,
    /// Mutation finished successfully; `finalize` ran.
    Committed,
    /// Mutation did not finish; the sweep promoted a stale `Pending`.
    Interrupted,
}

/// Snapshot of a single ref's target at a point in time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefSnapshot {
    /// Fully-qualified reference name (e.g. `refs/heads/nacho/feat/1`) or
    /// the literal `"HEAD"`.
    pub name: String,
    /// Target OID as a hex string, or `None` if the ref did not exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// True if this snapshot represents `HEAD`.
    #[serde(default)]
    pub is_head: bool,
    /// For `HEAD` snapshots: the symbolic ref HEAD pointed at, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_symbolic: Option<String>,
}

/// A remote-side effect produced by the operation. `gg undo` does not reverse
/// these — it surfaces them as provider-specific revert hints instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteEffect {
    /// A branch was pushed to a remote.
    Pushed {
        remote: String,
        branch: String,
        force: bool,
    },
    /// A pull/merge request was created.
    PrCreated { number: u64, url: String },
    /// A pull/merge request was merged.
    PrMerged { number: u64, url: String },
    /// A pull/merge request was closed (without merging).
    PrClosed { number: u64, url: String },
    /// A pull/merge request was queued for auto-merge. URL may be empty if
    /// the provider didn't surface one at queue time.
    PrQueued {
        number: u64,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        url: String,
    },
}

/// Durable operation record. One JSON file per record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRecord {
    /// `op_<13-digit-ms>_<32-char-uuid>`. Sortable lexically ↔ chronologically.
    pub id: String,
    /// Schema version at write time.
    pub schema_version: u32,
    pub kind: OperationKind,
    pub status: OperationStatus,
    pub created_at_ms: u64,
    /// Raw CLI args `std::env::args().skip(1)` so operators can audit what ran.
    pub args: Vec<String>,
    /// Stack name if the operation was stack-scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_name: Option<String>,
    /// Ref snapshot captured before the mutation.
    pub refs_before: Vec<RefSnapshot>,
    /// Ref snapshot captured after the mutation (empty until `finalize`).
    #[serde(default)]
    pub refs_after: Vec<RefSnapshot>,
    /// Remote side effects captured by the command (empty until `finalize`).
    #[serde(default)]
    pub remote_effects: Vec<RemoteEffect>,
    /// True iff any remote-visible mutation happened. When true, `gg undo`
    /// refuses with a provider-specific hint.
    #[serde(default)]
    pub touched_remote: bool,
    /// For `OperationKind::Undo` records: the id of the operation this undo
    /// reversed (so `--list` can show "undo of op_…").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undoes: Option<String>,
    /// Open-ended, schema-free per-command annotation. Reserved for future
    /// use (e.g. partial-rebase state).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_plan: Option<serde_json::Value>,
}

impl OperationRecord {
    /// True iff `gg undo` can locally reverse this record.
    pub fn is_undoable_locally(&self) -> bool {
        matches!(self.status, OperationStatus::Committed)
            && !self.touched_remote
            && self.schema_version <= SCHEMA_VERSION
    }
}

// ---------------------------------------------------------------------------
// ID + timestamp helpers
// ---------------------------------------------------------------------------

/// Epoch milliseconds from the wall clock. Test-visible via a public helper
/// so fixtures and the sweep use the same source of truth.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Generate `op_{13-digit-ms}_{32-char-uuidv4-no-dashes}`. The zero-padded
/// timestamp prefix keeps lexical sort == chronological sort.
pub fn new_id() -> String {
    let ms = now_ms();
    let uuid = uuid::Uuid::new_v4().simple().to_string();
    format!("op_{ms:013}_{uuid}")
}

// ---------------------------------------------------------------------------
// OperationStore (atomic save/load/list/prune)
// ---------------------------------------------------------------------------

/// On-disk store for operation records. One JSON file per record under
/// `<gg_dir>/operations/<id>.json`. Writes are crash-safe via
/// write-tempfile-then-rename.
#[derive(Debug, Clone)]
pub struct OperationStore {
    operations_dir: PathBuf,
}

impl OperationStore {
    /// `gg_dir` is `<repo.commondir()>/gg` (see design §2.1). Resolve it
    /// via `crate::git::gg_dir(repo)` — this type does not look at repos.
    pub fn new(gg_dir: &Path) -> Self {
        Self {
            operations_dir: gg_dir.join("operations"),
        }
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.operations_dir.join(format!("{id}.json"))
    }

    /// Persist a record atomically. Prunes Committed/Interrupted records
    /// beyond `OPERATION_LOG_CAP` afterwards, oldest-first.
    pub fn save(&self, record: &OperationRecord) -> Result<()> {
        fs::create_dir_all(&self.operations_dir)?;
        let path = self.path_for(&record.id);
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(record)?;
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)?;
        self.prune_to_cap();
        Ok(())
    }

    /// Load a record by id. Returns `OperationRecordNotFound` if absent.
    pub fn load(&self, id: &str) -> Result<OperationRecord> {
        let path = self.path_for(id);
        let bytes = fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => GgError::OperationRecordNotFound(id.to_string()),
            _ => GgError::Io(e),
        })?;
        let rec = serde_json::from_slice(&bytes)?;
        Ok(rec)
    }

    /// Newest-first ids (lexical sort == chronological because of the
    /// zero-padded timestamp prefix). Cheap: no JSON parse.
    pub fn list_ids(&self, limit: usize) -> Result<Vec<String>> {
        if !self.operations_dir.exists() {
            return Ok(vec![]);
        }
        let mut ids: Vec<String> = fs::read_dir(&self.operations_dir)?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| name.ends_with(".json") && !name.ends_with(".json.tmp"))
            .map(|name| name.trim_end_matches(".json").to_string())
            .collect();
        ids.sort_by(|a, b| b.cmp(a));
        ids.truncate(limit);
        Ok(ids)
    }

    /// Newest-first records, up to `limit`. Records that fail to parse are
    /// skipped (a poisoned record must not block the rest of the log).
    pub fn list(&self, limit: usize) -> Result<Vec<OperationRecord>> {
        let mut records: Vec<OperationRecord> = self
            .list_ids(usize::MAX)?
            .into_iter()
            .filter_map(|id| self.load(&id).ok())
            .collect();
        records.sort_by_key(|r| std::cmp::Reverse(r.created_at_ms));
        records.truncate(limit);
        Ok(records)
    }

    /// Prune oldest Committed/Interrupted records when over cap. Never
    /// removes Pending — the writing process may still be alive. Errors are
    /// swallowed to avoid cascading a save failure into the caller.
    fn prune_to_cap(&self) {
        let Ok(mut records) = self.list(usize::MAX) else {
            return;
        };
        if records.len() <= OPERATION_LOG_CAP {
            return;
        }
        // Sort oldest-first for pruning.
        records.sort_by_key(|r| r.created_at_ms);
        let excess = records.len() - OPERATION_LOG_CAP;
        let mut pruned = 0;
        for rec in records {
            if pruned >= excess {
                break;
            }
            if rec.status == OperationStatus::Pending {
                continue;
            }
            let _ = fs::remove_file(self.path_for(&rec.id));
            pruned += 1;
        }
    }

    /// Promote Pending records older than [`PENDING_STALENESS_MS`] to
    /// `Interrupted`. Swallows all errors: a poisoned record must not block
    /// mutations.
    pub fn sweep_pending(&self, now_ms_value: u64) {
        let Ok(ids) = self.list_ids(usize::MAX) else {
            return;
        };
        for id in ids {
            let Ok(mut rec) = self.load(&id) else {
                continue;
            };
            if rec.status != OperationStatus::Pending {
                continue;
            }
            if rec.created_at_ms.saturating_add(PENDING_STALENESS_MS) >= now_ms_value {
                continue;
            }
            rec.status = OperationStatus::Interrupted;
            let _ = self.save(&rec);
        }
    }
}

// ---------------------------------------------------------------------------
// snapshot_refs + SnapshotScope
// ---------------------------------------------------------------------------

/// Which refs to snapshot. `ActiveStack` is the cheap common case;
/// `AllUserBranches` is for cross-stack commands (`clean`, `reconcile`,
/// `run` amending multiple stacks).
pub enum SnapshotScope<'a> {
    ActiveStack(&'a Stack),
    AllUserBranches,
}

/// Capture a ref snapshot per the given scope.
///
/// Always appends a `HEAD` snapshot (symbolic if HEAD points at a branch).
///
/// D2 (conservative filtering): only local heads prefixed by the configured
/// `branch_username`; trunk names (`main` / `master` / `trunk`) are always
/// excluded even if they collide with the prefix filter.
pub fn snapshot_refs(
    repo: &Repository,
    config: &Config,
    scope: SnapshotScope<'_>,
) -> Result<Vec<RefSnapshot>> {
    let username = config.defaults.branch_username.as_deref();

    let mut snapshots: Vec<RefSnapshot> = Vec::new();
    match (scope, username) {
        (SnapshotScope::ActiveStack(stack), _) => {
            // Stack branch itself
            let stack_branch = stack.branch_name();
            let fq = format!("refs/heads/{stack_branch}");
            if let Some(snap) = snapshot_one(repo, &fq)? {
                snapshots.push(snap);
            }
            // Entry branches (gg-owned per-commit refs)
            for entry in &stack.entries {
                if let Some(name) = stack.entry_branch_name(entry) {
                    let fq = format!("refs/heads/{name}");
                    if let Some(snap) = snapshot_one(repo, &fq)? {
                        snapshots.push(snap);
                    }
                }
            }
        }
        (SnapshotScope::AllUserBranches, Some(u)) => {
            let prefix = format!("{u}/");
            for branch in repo.branches(Some(BranchType::Local))? {
                let (branch, _) = branch?;
                let name = match branch.name()? {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                if TRUNK_EXCLUSIONS.contains(&name.as_str()) {
                    continue;
                }
                if !name.starts_with(&prefix) {
                    continue;
                }
                let fq = format!("refs/heads/{name}");
                if let Some(snap) = snapshot_one(repo, &fq)? {
                    snapshots.push(snap);
                }
            }
        }
        (SnapshotScope::AllUserBranches, None) => {
            // Without a username we cannot tell which branches are gg-owned.
            // Record HEAD only; caller still gets a valid snapshot.
        }
    }

    snapshots.push(capture_head(repo)?);
    Ok(snapshots)
}

fn snapshot_one(repo: &Repository, fq_name: &str) -> Result<Option<RefSnapshot>> {
    match repo.find_reference(fq_name) {
        Ok(r) => {
            let target = r.target().map(|o| o.to_string());
            Ok(Some(RefSnapshot {
                name: fq_name.to_string(),
                target,
                is_head: false,
                head_symbolic: None,
            }))
        }
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(GgError::Git(e)),
    }
}

fn capture_head(repo: &Repository) -> Result<RefSnapshot> {
    match repo.head() {
        Ok(head) => {
            let symbolic = if head.is_branch() {
                Some(head.name().unwrap_or("").to_string())
            } else {
                None
            };
            let target = head.target().map(|o| o.to_string());
            Ok(RefSnapshot {
                name: "HEAD".into(),
                target,
                is_head: true,
                head_symbolic: symbolic,
            })
        }
        // Unborn HEAD / empty repo: record HEAD with no target.
        Err(e)
            if e.code() == git2::ErrorCode::UnbornBranch
                || e.code() == git2::ErrorCode::NotFound =>
        {
            Ok(RefSnapshot {
                name: "HEAD".into(),
                target: None,
                is_head: true,
                head_symbolic: None,
            })
        }
        Err(e) => Err(GgError::Git(e)),
    }
}

// ---------------------------------------------------------------------------
// OperationGuard
// ---------------------------------------------------------------------------

/// RAII-ish guard returned alongside the operation lock. On success paths
/// callers MUST invoke `finalize(...)`. Dropping without finalize is a
/// deliberate no-op — the record stays `Pending` on disk and the sweep
/// promotes it to `Interrupted` on the next lock acquisition. See design
/// §2.2 for the rationale (we cannot know mid-error if the mutation
/// partially succeeded).
#[derive(Debug)]
pub struct OperationGuard {
    pub(crate) record: OperationRecord,
    pub(crate) store: OperationStore,
    pub(crate) finalized: bool,
}

impl OperationGuard {
    pub fn id(&self) -> &str {
        &self.record.id
    }

    /// Mark the operation as Committed with the given post-mutation ref
    /// snapshot and remote effects. Consumes the guard.
    pub fn finalize(
        mut self,
        refs_after: Vec<RefSnapshot>,
        remote_effects: Vec<RemoteEffect>,
        touched_remote: bool,
    ) -> Result<()> {
        self.record.status = OperationStatus::Committed;
        self.record.refs_after = refs_after;
        self.record.remote_effects = remote_effects;
        self.record.touched_remote = touched_remote;
        self.store.save(&self.record)?;
        self.finalized = true;
        Ok(())
    }

    /// Variant for the undo handler, which also needs to populate `undoes`.
    /// Separate method to avoid bloating the common signature.
    pub fn finalize_as_undo(
        mut self,
        refs_after: Vec<RefSnapshot>,
        target_id: String,
    ) -> Result<()> {
        self.record.status = OperationStatus::Committed;
        self.record.refs_after = refs_after;
        self.record.undoes = Some(target_id);
        self.store.save(&self.record)?;
        self.finalized = true;
        Ok(())
    }

    /// Convenience: snapshot refs with the given scope and finalize. The
    /// common happy-path one-liner for Phase D instrumented commands.
    pub fn finalize_with_scope(
        self,
        repo: &Repository,
        config: &Config,
        scope: SnapshotScope<'_>,
        remote_effects: Vec<RemoteEffect>,
        touched_remote: bool,
    ) -> Result<()> {
        let refs_after = snapshot_refs(repo, config, scope)?;
        self.finalize(refs_after, remote_effects, touched_remote)
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        // Intentional: do nothing. See doc comment above and design §2.2.
    }
}

// ---------------------------------------------------------------------------
// Undo semantics
// ---------------------------------------------------------------------------

/// Options passed to [`run_undo`]. The CLI handler owns the surrounding
/// lock/record scaffolding.
///
/// JSON vs. human-readable output selection is handled by the caller; it
/// never affects [`run_undo`] behaviour, so it's not on this struct.
#[derive(Debug, Clone, Default)]
pub struct UndoOptions {
    /// Specific record id to target. `None` → most-recent-undoable.
    pub operation_id: Option<String>,
}

/// Outcome of [`run_undo`]. The caller decides exit codes and rendering.
#[derive(Debug)]
pub enum UndoOutcome {
    Succeeded(OperationRecord),
    RefusedRemote {
        target: OperationRecord,
        hints: Vec<String>,
    },
    RefusedInterrupted(OperationRecord),
    RefusedStale {
        target: OperationRecord,
        ref_name: String,
        expected: String,
        actual: String,
    },
    RefusedUnsupportedSchema(OperationRecord),
}

/// List recent operations, newest first.
pub fn list(repo: &Repository, limit: usize) -> Result<Vec<OperationRecord>> {
    let gg_dir = crate::git::gg_dir(repo);
    OperationStore::new(&gg_dir).list(limit)
}

/// Replay an operation's `refs_before` atop the repo. Consumes nothing from
/// the log itself; the caller wraps this in a fresh `Undo` record.
pub fn run_undo(
    repo: &Repository,
    _config: &Config,
    opts: UndoOptions,
) -> Result<UndoOutcome> {
    let gg_dir = crate::git::gg_dir(repo);
    let store = OperationStore::new(&gg_dir);

    // 1. Resolve target. When no id is provided, pick the most recent
    // undoable record. Per D5, undo-of-undo is fine: Undo records are
    // themselves undoable (they have refs_before/after and do not touch
    // the remote), so `gg undo; gg undo` naturally redoes the first op.
    let target = match opts.operation_id.clone() {
        Some(id) => store.load(&id)?,
        None => store
            .list(usize::MAX)?
            .into_iter()
            .find(|r| r.is_undoable_locally())
            .ok_or_else(|| GgError::OperationNotUndoable {
                id: "<none>".into(),
                reason: "no undoable operation in the log".into(),
            })?,
    };

    // 2. Gate checks in order.
    if target.schema_version > SCHEMA_VERSION {
        return Ok(UndoOutcome::RefusedUnsupportedSchema(target));
    }
    if target.status == OperationStatus::Interrupted {
        return Ok(UndoOutcome::RefusedInterrupted(target));
    }
    if target.touched_remote {
        let hints = build_remote_hints(&target);
        return Ok(UndoOutcome::RefusedRemote { target, hints });
    }
    // 2d. Staleness: every non-HEAD ref in refs_after must still match
    // current state. HEAD movement is allowed (user may have checked out
    // something else since).
    for snap in &target.refs_after {
        if snap.name == "HEAD" {
            continue;
        }
        let current = match repo.find_reference(&snap.name) {
            Ok(r) => r.target().map(|o| o.to_string()),
            Err(e) if e.code() == git2::ErrorCode::NotFound => None,
            Err(e) => return Err(GgError::Git(e)),
        };
        if current != snap.target {
            return Ok(UndoOutcome::RefusedStale {
                ref_name: snap.name.clone(),
                expected: snap.target.clone().unwrap_or_else(|| "<absent>".into()),
                actual: current.unwrap_or_else(|| "<absent>".into()),
                target,
            });
        }
    }

    // 3. Apply refs_before.
    for snap in &target.refs_before {
        if snap.name == "HEAD" {
            apply_head_snapshot(repo, snap)?;
            continue;
        }
        match &snap.target {
            Some(oid_str) => {
                let oid = git2::Oid::from_str(oid_str)?;
                repo.reference(&snap.name, oid, true, "gg undo")?;
            }
            None => {
                if let Ok(mut r) = repo.find_reference(&snap.name) {
                    r.delete()?;
                }
            }
        }
    }

    Ok(UndoOutcome::Succeeded(target))
}

fn apply_head_snapshot(repo: &Repository, snap: &RefSnapshot) -> Result<()> {
    if let Some(sym) = &snap.head_symbolic {
        repo.set_head(sym)?;
    } else if let Some(oid_str) = &snap.target {
        let oid = git2::Oid::from_str(oid_str)?;
        repo.set_head_detached(oid)?;
    }
    Ok(())
}

fn build_remote_hints(record: &OperationRecord) -> Vec<String> {
    record
        .remote_effects
        .iter()
        .map(|eff| match eff {
            RemoteEffect::Pushed { remote, branch, .. } => format!(
                "This operation pushed `{branch}` to `{remote}`. To revert a pushed \
                 branch manually, run: `git push --force-with-lease {remote} <prior_sha>:refs/heads/{branch}` \
                 where <prior_sha> is the pre-op SHA (see refs_before via `gg undo --json {id}`).",
                id = record.id
            ),
            RemoteEffect::PrCreated { number, url } => format!(
                "This operation opened {url}. Close it manually: `gh pr close {number}` / `glab mr close {number}`."
            ),
            RemoteEffect::PrMerged { number, url } => format!(
                "This operation merged {url}. Reversing a merged PR requires a manual revert on the base branch and optional reopen: `gh pr reopen {number}` / `glab mr reopen {number}`."
            ),
            RemoteEffect::PrClosed { number, url } => format!(
                "Reopen {url} with `gh pr reopen {number}` / `glab mr reopen {number}`."
            ),
            RemoteEffect::PrQueued { number, url } => {
                let link = if url.is_empty() { format!("PR #{number}") } else { url.clone() };
                format!(
                    "This operation queued {link} for auto-merge. To cancel before it merges: \
                     `gh pr merge --disable-auto {number}` / `glab mr update {number} --remove-auto-merge`."
                )
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // -- small fixtures ------------------------------------------------------

    pub(crate) fn make_record(kind: OperationKind, ts: u64) -> OperationRecord {
        OperationRecord {
            id: format!("op_{ts:013}_fixture00000000000000000000000000"),
            schema_version: SCHEMA_VERSION,
            kind,
            status: OperationStatus::Committed,
            created_at_ms: ts,
            args: vec!["fixture".into()],
            stack_name: None,
            refs_before: vec![],
            refs_after: vec![],
            remote_effects: vec![],
            touched_remote: false,
            undoes: None,
            pending_plan: None,
        }
    }

    pub(crate) fn tmp_gg_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let gg_dir = dir.path().join("gg");
        (dir, gg_dir)
    }

    // -- schema --------------------------------------------------------------

    #[test]
    fn operation_record_serde_round_trip_v1() {
        let record = OperationRecord {
            id: "op_0000000001700_abcd".into(),
            schema_version: SCHEMA_VERSION,
            kind: OperationKind::Drop,
            status: OperationStatus::Committed,
            created_at_ms: 1_700_000_000_000,
            args: vec!["drop".into(), "3".into()],
            stack_name: Some("feat/login".into()),
            refs_before: vec![RefSnapshot {
                name: "refs/heads/nacho/login/1".into(),
                target: Some("a".repeat(40)),
                is_head: true,
                head_symbolic: Some("refs/heads/nacho/login/1".into()),
            }],
            refs_after: vec![],
            remote_effects: vec![],
            touched_remote: false,
            undoes: None,
            pending_plan: None,
        };
        let json = serde_json::to_string(&record).unwrap();
        let back: OperationRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record.id, back.id);
        assert_eq!(record.kind, OperationKind::Drop);
    }

    #[test]
    fn operation_record_tolerates_unknown_fields_forward_compat() {
        let v2_json = r#"{
            "id": "op_0000000001700_efgh",
            "schema_version": 1,
            "kind": "squash",
            "status": "committed",
            "created_at_ms": 1700000000000,
            "args": ["squash"],
            "refs_before": [],
            "refs_after": [],
            "remote_effects": [],
            "touched_remote": false,
            "some_future_field": {"policy": "retain"}
        }"#;
        let parsed: OperationRecord = serde_json::from_str(v2_json).unwrap();
        assert_eq!(parsed.kind, OperationKind::Squash);
    }

    #[test]
    fn remote_effect_serializes_with_kind_tag() {
        let effect = RemoteEffect::Pushed {
            remote: "origin".into(),
            branch: "nacho/feat/1".into(),
            force: true,
        };
        let json = serde_json::to_value(&effect).unwrap();
        assert_eq!(json["kind"], "pushed");
        assert_eq!(json["remote"], "origin");
        assert_eq!(json["force"], true);
    }

    // -- id + timestamp ------------------------------------------------------

    #[test]
    fn new_id_is_sortable_by_time() {
        let a = new_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = new_id();
        assert!(a < b, "ids must sort chronologically: {a} !< {b}");
        assert!(a.starts_with("op_"));
        assert_eq!(a.len(), 3 + 13 + 1 + 32); // "op_" + 13-digit ms + "_" + uuid no-hyphens
    }

    #[test]
    fn now_ms_increases() {
        let a = now_ms();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = now_ms();
        assert!(b >= a + 2);
    }

    // -- store ---------------------------------------------------------------

    #[test]
    fn store_save_and_load_round_trip() {
        let (_guard, gg_dir) = tmp_gg_dir();
        let store = OperationStore::new(&gg_dir);
        let rec = make_record(OperationKind::Drop, 1_700_000_000_000);
        store.save(&rec).unwrap();
        let loaded = store.load(&rec.id).unwrap();
        assert_eq!(loaded.id, rec.id);
        assert_eq!(loaded.kind, OperationKind::Drop);
    }

    #[test]
    fn store_list_returns_newest_first() {
        let (_guard, gg_dir) = tmp_gg_dir();
        let store = OperationStore::new(&gg_dir);
        for ts in [1_000u64, 3_000, 2_000] {
            store.save(&make_record(OperationKind::Nav, ts)).unwrap();
        }
        let list = store.list(10).unwrap();
        let ts: Vec<u64> = list.iter().map(|r| r.created_at_ms).collect();
        assert_eq!(ts, vec![3_000, 2_000, 1_000]);
    }

    #[test]
    fn store_prunes_to_cap_skipping_pending() {
        let (_guard, gg_dir) = tmp_gg_dir();
        let store = OperationStore::new(&gg_dir);
        // 1 Pending at the oldest slot + (OPERATION_LOG_CAP + 5) Committed.
        let mut rec = make_record(OperationKind::Drop, 0);
        rec.status = OperationStatus::Pending;
        rec.id = format!("op_{:013}_pendingpendingpendingpendingpending", 0u64);
        store.save(&rec).unwrap();
        for i in 1..=(OPERATION_LOG_CAP as u64 + 5) {
            store.save(&make_record(OperationKind::Drop, i)).unwrap();
        }
        let all = store.list(usize::MAX).unwrap();
        assert!(all.iter().any(|r| r.status == OperationStatus::Pending));
        assert!(all.len() <= OPERATION_LOG_CAP + 1);
    }

    #[test]
    fn store_load_returns_err_for_unknown_id() {
        let (_guard, gg_dir) = tmp_gg_dir();
        let store = OperationStore::new(&gg_dir);
        assert!(store.load("op_does_not_exist").is_err());
    }

    // -- guard ---------------------------------------------------------------

    #[test]
    fn guard_finalize_flips_status_to_committed() {
        let (_g, gg_dir) = tmp_gg_dir();
        let store = OperationStore::new(&gg_dir);
        let rec = OperationRecord {
            id: new_id(),
            schema_version: SCHEMA_VERSION,
            kind: OperationKind::Drop,
            status: OperationStatus::Pending,
            created_at_ms: now_ms(),
            args: vec![],
            stack_name: None,
            refs_before: vec![],
            refs_after: vec![],
            remote_effects: vec![],
            touched_remote: false,
            undoes: None,
            pending_plan: None,
        };
        store.save(&rec).unwrap();
        let guard = OperationGuard {
            record: rec.clone(),
            store: store.clone(),
            finalized: false,
        };
        guard.finalize(vec![], vec![], false).unwrap();
        let loaded = store.load(&rec.id).unwrap();
        assert_eq!(loaded.status, OperationStatus::Committed);
    }

    #[test]
    fn guard_drop_without_finalize_leaves_record_pending() {
        let (_g, gg_dir) = tmp_gg_dir();
        let store = OperationStore::new(&gg_dir);
        let id = new_id();
        {
            let rec = OperationRecord {
                id: id.clone(),
                schema_version: SCHEMA_VERSION,
                kind: OperationKind::Nav,
                status: OperationStatus::Pending,
                created_at_ms: now_ms(),
                args: vec![],
                stack_name: None,
                refs_before: vec![],
                refs_after: vec![],
                remote_effects: vec![],
                touched_remote: false,
                undoes: None,
                pending_plan: None,
            };
            store.save(&rec).unwrap();
            let _guard = OperationGuard {
                record: rec,
                store: store.clone(),
                finalized: false,
            };
            // guard dropped without finalize
        }
        let loaded = store.load(&id).unwrap();
        assert_eq!(loaded.status, OperationStatus::Pending);
    }

    // -- sweep ---------------------------------------------------------------

    #[test]
    fn sweep_promotes_stale_pending_to_interrupted() {
        let (_g, gg_dir) = tmp_gg_dir();
        let store = OperationStore::new(&gg_dir);
        let mut stale = make_record(OperationKind::Nav, 1_000);
        stale.status = OperationStatus::Pending;
        stale.id = format!("op_{:013}_stalestalestalestalestalestalex", 1_000u64);
        let mut fresh = make_record(OperationKind::Nav, 100_000_000);
        fresh.status = OperationStatus::Pending;
        fresh.id = format!("op_{:013}_freshfreshfreshfreshfreshfreshff", 100_000_000u64);
        store.save(&stale).unwrap();
        store.save(&fresh).unwrap();

        store.sweep_pending(100_000_000 + 5_000);

        let stale_loaded = store.load(&stale.id).unwrap();
        let fresh_loaded = store.load(&fresh.id).unwrap();
        assert_eq!(stale_loaded.status, OperationStatus::Interrupted);
        assert_eq!(fresh_loaded.status, OperationStatus::Pending);
    }

    #[test]
    fn sweep_never_errors_when_dir_missing() {
        let (_g, gg_dir) = tmp_gg_dir();
        let store = OperationStore::new(&gg_dir);
        store.sweep_pending(now_ms()); // must not panic
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::config::Config;
    use git2::Repository;

    fn init_repo_with_branches(prefix: &str, branches: &[&str]) -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        // Use an explicit, non-conflicting initial branch so the test can freely
        // create branches named main/master/trunk without the HEAD-branch clash
        // (init.defaultBranch varies across systems).
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("gg-test-base");
        let repo = Repository::init_opts(dir.path(), &opts).unwrap();
        {
            let sig = git2::Signature::now("gg-test", "gg@test").unwrap();
            let tree_oid = {
                let mut idx = repo.index().unwrap();
                idx.write_tree().unwrap()
            };
            let tree = repo.find_tree(tree_oid).unwrap();
            let commit_oid = repo
                .commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
            let commit = repo.find_commit(commit_oid).unwrap();
            for name in branches {
                let fq: String = if name.contains('/') {
                    format!("{prefix}/{name}")
                } else {
                    (*name).to_string()
                };
                repo.branch(&fq, &commit, true).unwrap();
            }
        }
        (dir, repo)
    }

    fn config_with_username(username: &str) -> Config {
        let mut c = Config::default();
        c.defaults.branch_username = Some(username.to_string());
        c
    }

    #[test]
    fn all_user_branches_filters_by_username_prefix() {
        let (_g, repo) =
            init_repo_with_branches("nacho", &["main", "x/1", "x/2", "other/mine"]);
        let cfg = config_with_username("nacho");
        let snaps = snapshot_refs(&repo, &cfg, SnapshotScope::AllUserBranches).unwrap();
        let names: Vec<&str> = snaps.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"refs/heads/nacho/x/1"));
        assert!(names.contains(&"refs/heads/nacho/x/2"));
        assert!(!names.contains(&"refs/heads/main"));
        assert!(!names.contains(&"refs/heads/other/mine"));
    }

    #[test]
    fn all_user_branches_excludes_main_master_trunk() {
        // `init_repo_with_branches` creates trunk/main alongside whatever
        // the default branch is. We only verify the filter: no trunk name
        // should ever land in the snapshot.
        let (_g, repo) = init_repo_with_branches("nacho", &["trunk", "main"]);
        let cfg = config_with_username("nacho");
        let snaps = snapshot_refs(&repo, &cfg, SnapshotScope::AllUserBranches).unwrap();
        assert!(!snaps.iter().any(|s| s.name == "refs/heads/main"));
        assert!(!snaps.iter().any(|s| s.name == "refs/heads/master"));
        assert!(!snaps.iter().any(|s| s.name == "refs/heads/trunk"));
    }

    #[test]
    fn snapshot_captures_head_symbolic_ref() {
        let (_g, repo) = init_repo_with_branches("nacho", &["x/1"]);
        let cfg = config_with_username("nacho");
        let snaps = snapshot_refs(&repo, &cfg, SnapshotScope::AllUserBranches).unwrap();
        let head = snaps
            .iter()
            .find(|s| s.is_head)
            .expect("HEAD should be captured");
        assert!(head.head_symbolic.is_some(), "expected symbolic HEAD");
    }
}

#[cfg(test)]
mod undo_tests {
    use super::*;
    use crate::config::Config;
    use git2::Repository;

    fn setup_repo_with_two_commits() -> (tempfile::TempDir, Repository, git2::Oid, git2::Oid) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("t", "t@t").unwrap();
        let (c1, c2) = {
            let tree_oid = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(tree_oid).unwrap();
            let c1 = repo
                .commit(Some("HEAD"), &sig, &sig, "c1", &tree, &[])
                .unwrap();
            let c1_commit = repo.find_commit(c1).unwrap();
            let c2 = repo
                .commit(Some("HEAD"), &sig, &sig, "c2", &tree, &[&c1_commit])
                .unwrap();
            (c1, c2)
        };
        (dir, repo, c1, c2)
    }

    fn cfg_user(u: &str) -> Config {
        let mut c = Config::default();
        c.defaults.branch_username = Some(u.into());
        c
    }

    #[test]
    fn run_undo_restores_branch_to_prior_oid() {
        let (_dir, repo, c1, c2) = setup_repo_with_two_commits();
        let c1_commit = repo.find_commit(c1).unwrap();
        repo.branch("nacho/feat/1", &c1_commit, true).unwrap();

        let gg_dir = crate::git::gg_dir(&repo);
        let store = OperationStore::new(&gg_dir);

        let rec = OperationRecord {
            id: new_id(),
            schema_version: SCHEMA_VERSION,
            kind: OperationKind::Squash,
            status: OperationStatus::Committed,
            created_at_ms: now_ms(),
            args: vec![],
            stack_name: None,
            refs_before: vec![RefSnapshot {
                name: "refs/heads/nacho/feat/1".into(),
                target: Some(c1.to_string()),
                is_head: false,
                head_symbolic: None,
            }],
            refs_after: vec![RefSnapshot {
                name: "refs/heads/nacho/feat/1".into(),
                target: Some(c2.to_string()),
                is_head: false,
                head_symbolic: None,
            }],
            remote_effects: vec![],
            touched_remote: false,
            undoes: None,
            pending_plan: None,
        };
        store.save(&rec).unwrap();

        repo.reference("refs/heads/nacho/feat/1", c2, true, "test setup")
            .unwrap();

        let outcome = run_undo(
            &repo,
            &cfg_user("nacho"),
            UndoOptions {
                operation_id: Some(rec.id.clone()),
            },
        )
        .unwrap();

        assert!(matches!(outcome, UndoOutcome::Succeeded(_)));
        let restored = repo.find_reference("refs/heads/nacho/feat/1").unwrap();
        assert_eq!(restored.target().unwrap(), c1);
    }

    #[test]
    fn run_undo_refuses_when_ref_moved_since_operation() {
        let (_dir, repo, c1, c2) = setup_repo_with_two_commits();
        let c1_commit = repo.find_commit(c1).unwrap();
        repo.branch("nacho/feat/2", &c1_commit, true).unwrap();

        let gg_dir = crate::git::gg_dir(&repo);
        let store = OperationStore::new(&gg_dir);
        let rec = OperationRecord {
            id: new_id(),
            schema_version: SCHEMA_VERSION,
            kind: OperationKind::Squash,
            status: OperationStatus::Committed,
            created_at_ms: now_ms(),
            args: vec![],
            stack_name: None,
            refs_before: vec![RefSnapshot {
                name: "refs/heads/nacho/feat/2".into(),
                target: Some(c1.to_string()),
                is_head: false,
                head_symbolic: None,
            }],
            refs_after: vec![RefSnapshot {
                name: "refs/heads/nacho/feat/2".into(),
                target: Some(c2.to_string()),
                is_head: false,
                head_symbolic: None,
            }],
            remote_effects: vec![],
            touched_remote: false,
            undoes: None,
            pending_plan: None,
        };
        store.save(&rec).unwrap();

        // Branch is still at c1 (different from refs_after c2).
        let outcome = run_undo(
            &repo,
            &cfg_user("nacho"),
            UndoOptions {
                operation_id: Some(rec.id.clone()),
            },
        )
        .unwrap();
        assert!(matches!(outcome, UndoOutcome::RefusedStale { .. }));
    }

    #[test]
    fn run_undo_refuses_remote_touched_op() {
        let (_dir, repo, _c1, _c2) = setup_repo_with_two_commits();
        let gg_dir = crate::git::gg_dir(&repo);
        let store = OperationStore::new(&gg_dir);
        let rec = OperationRecord {
            id: new_id(),
            schema_version: SCHEMA_VERSION,
            kind: OperationKind::Sync,
            status: OperationStatus::Committed,
            created_at_ms: now_ms(),
            args: vec!["sync".into()],
            stack_name: None,
            refs_before: vec![],
            refs_after: vec![],
            remote_effects: vec![RemoteEffect::Pushed {
                remote: "origin".into(),
                branch: "nacho/x/1".into(),
                force: false,
            }],
            touched_remote: true,
            undoes: None,
            pending_plan: None,
        };
        store.save(&rec).unwrap();
        let out = run_undo(
            &repo,
            &cfg_user("nacho"),
            UndoOptions {
                operation_id: Some(rec.id.clone()),
            },
        )
        .unwrap();
        assert!(matches!(out, UndoOutcome::RefusedRemote { .. }));
    }

    #[test]
    fn run_undo_refuses_interrupted_op() {
        let (_dir, repo, _c1, _c2) = setup_repo_with_two_commits();
        let gg_dir = crate::git::gg_dir(&repo);
        let store = OperationStore::new(&gg_dir);
        let mut rec = crate::operations::tests::make_record(OperationKind::Drop, now_ms());
        rec.status = OperationStatus::Interrupted;
        store.save(&rec).unwrap();
        let out = run_undo(
            &repo,
            &cfg_user("nacho"),
            UndoOptions {
                operation_id: Some(rec.id.clone()),
            },
        )
        .unwrap();
        assert!(matches!(out, UndoOutcome::RefusedInterrupted(_)));
    }
}
