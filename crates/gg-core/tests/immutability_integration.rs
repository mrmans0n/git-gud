//! Integration tests for the immutability policy that require a real git
//! repository on disk (for ancestry / ref lookups).

use std::path::Path;
use std::process::Command;

use git2::Repository;

use gg_core::immutability::{
    guard, refresh_mr_state_for_guard, ImmutabilityPolicy, ImmutableReason,
};
use gg_core::provider::PrState;
use gg_core::stack::{Stack, StackEntry};

fn run_git(repo_path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .expect("git command failed to execute");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn head_oid(repo_path: &Path) -> git2::Oid {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()
        .unwrap();
    git2::Oid::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap()
}

/// Build a linear stack on top of `main` and return (repo, stack).
/// The repo has: base commit on main, then `n` stack commits committed on a
/// feature branch. `origin/main` is set to advance up to `base_advance_to`
/// of the stack commits (0 = still at base, 1 = covers first stack commit,
/// etc.).
fn make_linear_stack(
    temp: &tempfile::TempDir,
    n: usize,
    base_advance_to: usize,
) -> (Repository, Vec<git2::Oid>, git2::Oid) {
    let repo_path = temp.path();

    run_git(repo_path, &["init", "--initial-branch=main"]);
    run_git(repo_path, &["config", "user.email", "test@example.com"]);
    run_git(repo_path, &["config", "user.name", "Test User"]);

    std::fs::write(repo_path.join("base.txt"), "base\n").unwrap();
    run_git(repo_path, &["add", "."]);
    run_git(repo_path, &["commit", "-m", "base"]);
    let base_main_oid = head_oid(repo_path);

    run_git(repo_path, &["checkout", "-b", "u/stack"]);

    let mut stack_oids: Vec<git2::Oid> = Vec::new();
    for i in 0..n {
        let file = format!("stack_{}.txt", i + 1);
        std::fs::write(repo_path.join(&file), format!("content-{}\n", i + 1)).unwrap();
        run_git(repo_path, &["add", "."]);
        let msg = format!("stack commit {}\n\nGG-ID: c-{:07}", i + 1, i + 1);
        run_git(repo_path, &["commit", "-m", &msg]);
        stack_oids.push(head_oid(repo_path));
    }

    // Create refs/remotes/origin/main pointing at `base_advance_to` commits
    // up the stack (0 = at base, otherwise stack_oids[base_advance_to - 1]).
    let target_oid = if base_advance_to == 0 {
        base_main_oid
    } else {
        stack_oids[base_advance_to - 1]
    };
    run_git(
        repo_path,
        &[
            "update-ref",
            "refs/remotes/origin/main",
            &target_oid.to_string(),
        ],
    );

    let repo = Repository::open(repo_path).unwrap();
    (repo, stack_oids, base_main_oid)
}

fn build_stack(oids: &[git2::Oid], states: &[Option<PrState>]) -> Stack {
    assert_eq!(oids.len(), states.len());
    let entries: Vec<StackEntry> = oids
        .iter()
        .zip(states.iter())
        .enumerate()
        .map(|(i, (oid, state))| StackEntry {
            oid: *oid,
            short_sha: oid.to_string()[..7].to_string(),
            title: format!("stack commit {}", i + 1),
            gg_id: Some(format!("c-{:07}", i + 1)),
            gg_parent: None,
            mr_number: state.as_ref().map(|_| (i as u64) + 100),
            mr_state: state.clone(),
            approved: false,
            changes_requested: false,
            mergeable: false,
            ci_status: None,
            position: i + 1,
            in_merge_train: false,
            merge_train_position: None,
        })
        .collect();

    Stack {
        name: "stack".to_string(),
        username: "u".to_string(),
        base: "main".to_string(),
        entries,
        current_position: Some(oids.len().saturating_sub(1)),
    }
}

#[test]
fn base_ancestor_rule_fires_when_origin_base_covers_stack_commit() {
    let temp = tempfile::tempdir().unwrap();
    let (repo, oids, _) = make_linear_stack(&temp, 3, 2);
    // origin/main now points at stack_oids[1] (the 2nd stack commit).

    let stack = build_stack(&oids, &[None, None, None]);
    let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();

    // Entry #1 and #2 are ancestors of origin/main, so they should be immutable.
    let report = policy.check_all(&stack);
    let positions: Vec<usize> = report.entries.iter().map(|e| e.position).collect();
    assert_eq!(positions, vec![1, 2]);

    // Reasons should include BaseAncestor pointing at origin/main.
    for entry in &report.entries {
        assert!(
            entry
                .reasons
                .iter()
                .any(|r| matches!(r, ImmutableReason::BaseAncestor { base_ref } if base_ref == "origin/main")),
            "expected base-ancestor reason, got {:?}",
            entry.reasons
        );
    }
}

#[test]
fn base_ancestor_rule_is_silent_when_origin_base_is_behind_stack() {
    let temp = tempfile::tempdir().unwrap();
    let (repo, oids, _) = make_linear_stack(&temp, 3, 0);
    // origin/main at base → no stack commit is an ancestor of origin/main.

    let stack = build_stack(&oids, &[None, None, None]);
    let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();

    let report = policy.check_all(&stack);
    assert!(report.is_clear(), "expected clear report, got {:?}", report);
}

#[test]
fn merged_pr_and_base_ancestor_stack_together() {
    let temp = tempfile::tempdir().unwrap();
    let (repo, oids, _) = make_linear_stack(&temp, 3, 1);
    // origin/main covers stack commit #1 only.

    // Also mark commit #1 as Merged. Both reasons should show up.
    let stack = build_stack(&oids, &[Some(PrState::Merged), Some(PrState::Open), None]);

    let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();
    let report = policy.check_all(&stack);

    assert_eq!(report.entries.len(), 1);
    let entry = &report.entries[0];
    assert_eq!(entry.position, 1);
    assert_eq!(entry.reasons.len(), 2);
    assert!(entry
        .reasons
        .iter()
        .any(|r| matches!(r, ImmutableReason::MergedPr { number: Some(100) })));
    assert!(entry
        .reasons
        .iter()
        .any(|r| matches!(r, ImmutableReason::BaseAncestor { .. })));
}

#[test]
fn guard_error_includes_both_sha_and_reason() {
    let temp = tempfile::tempdir().unwrap();
    let (repo, oids, _) = make_linear_stack(&temp, 2, 1);
    let stack = build_stack(&oids, &[Some(PrState::Merged), None]);

    let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();
    let report = policy.check_positions(&stack, &[1]);

    let err = guard(report, false).expect_err("should fail");
    let msg = format!("{}", err);
    assert!(msg.contains("merged as !100"));
    assert!(msg.contains("already in origin/main"));
}

#[test]
fn squash_merged_commit_fires_guard_via_mr_state_only() {
    // Reproduces the scenario the merged-PR rule exists for: a PR was
    // squash-merged upstream, so its merge commit on origin/<base> has a new
    // SHA that doesn't share ancestry with the local commit. The
    // base-ancestor rule misses it; only mr_state == Merged catches it.
    let temp = tempfile::tempdir().unwrap();
    let (repo, oids, _) = make_linear_stack(&temp, 3, 0);
    // origin/main is at base — no stack commit is reachable from it.

    // Commit #2 was squash-merged: its PR is Merged but the local SHA isn't
    // anywhere on origin/main. Without the merged-PR rule, the guard would
    // happily rewrite it.
    let stack = build_stack(&oids, &[None, Some(PrState::Merged), None]);
    let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();
    let report = policy.check_all(&stack);

    assert_eq!(
        report.entries.len(),
        1,
        "expected exactly one immutable entry"
    );
    let entry = &report.entries[0];
    assert_eq!(entry.position, 2);
    assert_eq!(
        entry.reasons.len(),
        1,
        "only the merged-PR reason should fire (base-ancestor misses squash-merge): {:?}",
        entry.reasons
    );
    assert!(matches!(
        entry.reasons[0],
        ImmutableReason::MergedPr { number: Some(101) }
    ));

    // And the guard rejects it without --force.
    let err = guard(report, false).expect_err("guard should reject squash-merged commit");
    let msg = format!("{}", err);
    assert!(msg.contains("merged as !101"), "got: {}", msg);
}

#[test]
fn refresh_mr_state_for_guard_is_noop_without_provider() {
    // No remote configured → Provider::detect fails → helper must be a quiet
    // no-op (offline / no-auth users see no behaviour change). We rely on the
    // helper not panicking and not modifying mr_state.
    let temp = tempfile::tempdir().unwrap();
    let (repo, oids, _) = make_linear_stack(&temp, 2, 0);

    let mut stack = build_stack(&oids, &[Some(PrState::Open), None]);
    let original_state = stack.entries[0].mr_state.clone();

    refresh_mr_state_for_guard(&repo, &mut stack);

    // mr_state must be unchanged because no provider could be reached.
    assert_eq!(
        stack.entries[0].mr_state, original_state,
        "helper must not mutate state when no provider is configured"
    );
    // And entries without an mr_number must remain untouched too.
    assert_eq!(stack.entries[1].mr_state, None);
}

#[test]
fn rebase_guard_passes_when_only_immutable_is_base_ancestor() {
    // Repro for #293: 2-commit stack where bottom is already on origin/main.
    // After without_base_ancestors(), the guard should pass without --force.
    let temp = tempfile::tempdir().unwrap();
    let (repo, oids, _) = make_linear_stack(&temp, 2, 1);
    // origin/main covers stack commit #1.

    let stack = build_stack(&oids, &[None, None]);
    let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();

    let report = policy.check_all(&stack);
    // Pre-filter: commit #1 is immutable (BaseAncestor).
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].position, 1);

    // After filtering, the report should be clear.
    let filtered = report.without_base_ancestors();
    assert!(
        filtered.is_clear(),
        "expected clear report after filtering base ancestors, got {:?}",
        filtered
    );

    // Guard passes without --force.
    assert!(guard(filtered, false).is_ok());
}

#[test]
fn rebase_guard_passes_squash_merged_not_on_base() {
    // A squash-merged PR whose SHA is NOT on origin/main should pass during
    // rebase: git rebase drops it via patch-id matching. The
    // without_bottom_merged_prs() filter removes it.
    let temp = tempfile::tempdir().unwrap();
    let (repo, oids, _) = make_linear_stack(&temp, 2, 0);
    // origin/main at base commit — no stack commits are ancestors.

    // Mark commit #1 as squash-merged (MergedPr only, not BaseAncestor).
    let stack = build_stack(&oids, &[Some(PrState::Merged), None]);
    let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();

    let report = policy.check_all(&stack);
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].position, 1);
    assert!(matches!(
        report.entries[0].reasons[0],
        ImmutableReason::MergedPr { .. }
    ));

    // without_base_ancestors does NOT remove this entry (no BaseAncestor reason).
    let filtered = report.without_base_ancestors();
    assert_eq!(
        filtered.entries.len(),
        1,
        "squash-merged entry must survive without_base_ancestors filtering"
    );

    // But without_bottom_merged_prs DOES remove it (position 1, contiguous prefix).
    let filtered = filtered.without_bottom_merged_prs();
    assert!(
        filtered.is_clear(),
        "squash-merged entry at bottom should be removed by without_bottom_merged_prs"
    );

    // Guard passes without --force.
    assert!(guard(filtered, false).is_ok());
}

#[test]
fn squash_merged_still_blocks_non_rebase_commands() {
    // Squash-merged entries must still block non-rebase commands (squash,
    // reorder, drop, etc.) that don't apply without_bottom_merged_prs().
    let temp = tempfile::tempdir().unwrap();
    let (repo, oids, _) = make_linear_stack(&temp, 2, 0);

    let stack = build_stack(&oids, &[Some(PrState::Merged), None]);
    let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();

    let report = policy.check_all(&stack);
    assert_eq!(report.entries.len(), 1);

    // Guard rejects without --force (simulates non-rebase command path).
    assert!(guard(report, false).is_err());
}

#[test]
fn rebase_guard_keeps_base_ancestors_for_cross_target_rebases() {
    let temp = tempfile::tempdir().unwrap();
    let (repo, oids, _) = make_linear_stack(&temp, 2, 1);

    let stack = build_stack(&oids, &[None, None]);
    let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();
    let report = policy.check_all(&stack);

    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].position, 1);

    // Rebasing onto a non-base target must keep BaseAncestor entries, because
    // the ancestry check was computed against stack.base, not the chosen target.
    assert!(guard(report, false).is_err());
}

#[test]
fn rebase_guard_keeps_base_ancestors_when_fetch_was_not_fresh() {
    let temp = tempfile::tempdir().unwrap();
    let (repo, oids, _) = make_linear_stack(&temp, 2, 1);

    let stack = build_stack(&oids, &[None, None]);
    let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();
    let report = policy.check_all(&stack);

    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].position, 1);

    // If fetch failed and origin/<base> may be stale, rebase should keep the
    // original guard behavior instead of silently dropping BaseAncestor entries.
    assert!(guard(report, false).is_err());
}

#[test]
fn falls_back_to_local_base_when_origin_ref_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = temp.path();

    run_git(repo_path, &["init", "--initial-branch=main"]);
    run_git(repo_path, &["config", "user.email", "test@example.com"]);
    run_git(repo_path, &["config", "user.name", "Test User"]);

    std::fs::write(repo_path.join("base.txt"), "base\n").unwrap();
    run_git(repo_path, &["add", "."]);
    run_git(repo_path, &["commit", "-m", "base"]);
    let base_oid = head_oid(repo_path);

    run_git(repo_path, &["checkout", "-b", "u/stack"]);
    std::fs::write(repo_path.join("x.txt"), "x\n").unwrap();
    run_git(repo_path, &["add", "."]);
    run_git(repo_path, &["commit", "-m", "s1"]);
    let s1 = head_oid(repo_path);

    // No origin/main ref exists; policy should fall back to local `main`.
    let repo = Repository::open(repo_path).unwrap();
    let stack = build_stack(&[base_oid, s1], &[None, None]);
    let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();
    assert_eq!(policy.base_ref(), "main");

    // The base commit is an ancestor of itself, so entry #1 (base_oid) is
    // immutable via the local base.
    let report = policy.check_all(&stack);
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].position, 1);
}
