//! End-to-end tests for `gg undo`, the operation log, and the
//! mutating-command instrumentation added in task #5.
//!
//! Covers the acceptance criteria called out in the design/review:
//!   - per-kind undo for drop and reorder (a newly-locked surface)
//!   - `--list` ordering and JSON shape
//!   - redo-via-double-undo (D5)
//!   - worktree round-trip: op log lives under `commondir()`, not the
//!     worktree's `.git` file, so undo works from either side (design §6)

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers (minimal copies of those in integration_tests.rs; the test harness
// compiles each file as a separate binary so we can't share).
// ---------------------------------------------------------------------------

fn create_test_repo() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("tempdir");
    let repo_path = temp_dir.path().to_path_buf();

    run(&repo_path, "git", &["init", "--initial-branch=main"]);
    run(&repo_path, "git", &["config", "user.email", "test@test.com"]);
    run(&repo_path, "git", &["config", "user.name", "Test User"]);

    std::fs::write(repo_path.join("README.md"), "# Test\n").unwrap();
    run(&repo_path, "git", &["add", "."]);
    run(&repo_path, "git", &["commit", "-m", "Initial commit"]);

    // Seed the gg config so stacks can resolve `branch_username`.
    let gg_dir = repo_path.join(".git/gg");
    std::fs::create_dir_all(&gg_dir).unwrap();
    std::fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    (temp_dir, repo_path)
}

fn run(cwd: &std::path::Path, bin: &str, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(bin)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {} {:?}: {}", bin, args, e));
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

fn gg(cwd: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let bin = env!("CARGO_BIN_EXE_gg");
    let output = Command::new(bin)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("failed to run gg {:?}: {}", args, e));
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// Add a file with `content` and commit with `message`.
fn commit_file(repo: &std::path::Path, name: &str, content: &str, message: &str) {
    std::fs::write(repo.join(name), content).unwrap();
    run(repo, "git", &["add", "."]);
    run(repo, "git", &["commit", "-m", message]);
}

/// Build a stack named `stack` with N commits "Commit 1"..."Commit N".
fn setup_stack_with_commits(repo: &std::path::Path, stack: &str, n: usize) {
    let (ok, _, err) = gg(repo, &["co", stack]);
    assert!(ok, "co failed: {err}");
    for i in 1..=n {
        commit_file(
            repo,
            &format!("file{i}.txt"),
            &format!("content {i}\n"),
            &format!("Commit {i}"),
        );
    }
}

fn head_sha(repo: &std::path::Path) -> String {
    let (_ok, out, _err) = run(repo, "git", &["rev-parse", "HEAD"]);
    out.trim().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn undo_drop_restores_commit_and_head() {
    let (_tmp, repo) = create_test_repo();
    setup_stack_with_commits(&repo, "undo-drop", 3);
    let sha_before = head_sha(&repo);

    // Drop the middle commit.
    let (ok, stdout, stderr) = gg(&repo, &["drop", "2", "--force"]);
    assert!(ok, "drop failed: {stdout}{stderr}");
    let (_ok, ls_out, _) = gg(&repo, &["ls"]);
    assert!(!ls_out.contains("Commit 2"));

    // Undo.
    let (ok, stdout, stderr) = gg(&repo, &["undo"]);
    assert!(ok, "undo failed: {stdout}{stderr}");
    let (_ok, ls_out, _) = gg(&repo, &["ls"]);
    assert!(
        ls_out.contains("Commit 1") && ls_out.contains("Commit 2") && ls_out.contains("Commit 3"),
        "all three commits should reappear after undo. Got:\n{ls_out}"
    );
    assert_eq!(head_sha(&repo), sha_before, "HEAD must be restored");
}

#[test]
fn undo_reorder_restores_original_order() {
    // `reorder` is one of the surfaces that newly acquires the operation
    // lock in task #5; a dedicated per-kind test verifies the instrumentation
    // round-trips.
    let (_tmp, repo) = create_test_repo();
    setup_stack_with_commits(&repo, "undo-reorder", 3);
    let sha_before = head_sha(&repo);

    // Move bottom to top: 3,1,2 (new order top→bottom is 2,1,3 in ls output).
    let (ok, stdout, stderr) = gg(&repo, &["reorder", "--order", "3,1,2"]);
    assert!(ok, "reorder failed: {stdout}{stderr}");

    // Undo restores the original stack order + HEAD.
    let (ok, stdout, stderr) = gg(&repo, &["undo"]);
    assert!(ok, "undo failed: {stdout}{stderr}");
    assert_eq!(head_sha(&repo), sha_before);
}

#[test]
fn undo_list_shows_ops_newest_first_and_has_expected_fields() {
    let (_tmp, repo) = create_test_repo();
    setup_stack_with_commits(&repo, "undo-list", 2);

    // Make two distinct mutating ops so the list has ≥2 entries.
    let (ok, _, err) = gg(&repo, &["drop", "1", "--force"]);
    assert!(ok, "drop1 failed: {err}");
    commit_file(&repo, "extra.txt", "extra\n", "Commit extra");
    let (ok, _, err) = gg(&repo, &["drop", "1", "--force"]);
    assert!(ok, "drop2 failed: {err}");

    let (ok, stdout, err) = gg(&repo, &["undo", "--list", "--json"]);
    assert!(ok, "list failed: {err}");
    let v: Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(v["version"], 1);
    let ops = v["operations"].as_array().unwrap();
    assert!(ops.len() >= 2, "expected ≥2 ops, got {}", ops.len());

    // Newest-first ordering.
    let ts0 = ops[0]["created_at_ms"].as_u64().unwrap();
    let ts1 = ops[1]["created_at_ms"].as_u64().unwrap();
    assert!(
        ts0 >= ts1,
        "list must be newest-first: ts0={ts0}, ts1={ts1}"
    );

    // Expected fields present.
    for op in ops.iter().take(2) {
        for field in &[
            "id",
            "kind",
            "status",
            "created_at_ms",
            "args",
            "touched_remote",
            "is_undoable",
        ] {
            assert!(op.get(field).is_some(), "op missing `{field}`");
        }
    }
}

#[test]
fn undo_undo_redoes_the_original_op() {
    // Design §3.5 / D5: redo is modeled as undoing the previous `Undo` op.
    let (_tmp, repo) = create_test_repo();
    setup_stack_with_commits(&repo, "undo-redo", 3);

    let (ok, _, err) = gg(&repo, &["drop", "2", "--force"]);
    assert!(ok, "drop failed: {err}");
    let after_drop = head_sha(&repo);

    // Undo — restores commit 2.
    let (ok, _, err) = gg(&repo, &["undo"]);
    assert!(ok, "first undo failed: {err}");
    let (_ok, ls_out, _) = gg(&repo, &["ls"]);
    assert!(ls_out.contains("Commit 2"), "undo should restore Commit 2");

    // Undo again — redoes the drop, putting us back at after_drop.
    let (ok, _, err) = gg(&repo, &["undo"]);
    assert!(ok, "second undo failed: {err}");
    assert_eq!(
        head_sha(&repo),
        after_drop,
        "double-undo must return to the post-drop SHA"
    );
    let (_ok, ls_out, _) = gg(&repo, &["ls"]);
    assert!(
        !ls_out.contains("Commit 2"),
        "commit 2 should be dropped again after redo"
    );
}

#[test]
fn undo_list_json_from_linked_worktree_shows_main_worktree_ops() {
    // Design §6 / AC#6: the op log lives under the git commondir, not the
    // worktree's `.git` pointer file. An op recorded from the main worktree
    // must therefore be visible from a linked worktree.
    let (_tmp, repo) = create_test_repo();
    setup_stack_with_commits(&repo, "worktree-undo", 2);

    // Record an op from the main worktree.
    let (ok, _, err) = gg(&repo, &["drop", "1", "--force"]);
    assert!(ok, "drop in main worktree failed: {err}");

    // Create a linked worktree in its own unique tempdir so parallel test
    // runs don't collide on a shared path.
    let wt_tmp = TempDir::new().expect("worktree tempdir");
    let wt_path = wt_tmp.path().join("wt");
    let (ok, _, err) = run(
        &repo,
        "git",
        &[
            "worktree",
            "add",
            "-b",
            "linked",
            wt_path.to_str().unwrap(),
            "main",
        ],
    );
    assert!(ok, "worktree add failed: {err}");

    // List from the linked worktree — must see the op recorded from the main.
    let (ok, stdout, err) = gg(&wt_path, &["undo", "--list", "--json"]);
    assert!(ok, "undo --list from linked worktree failed: {err}");
    let v: Value = serde_json::from_str(&stdout).expect("valid JSON");
    let ops = v["operations"].as_array().unwrap();
    assert!(
        ops.iter().any(|op| op["kind"] == "drop"),
        "linked worktree should see the main worktree's drop op. Got:\n{stdout}"
    );
}
