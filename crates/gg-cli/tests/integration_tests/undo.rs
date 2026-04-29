use crate::helpers::{create_test_repo, run_gg, run_git};

use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

fn setup_undo_test_repo(stack_name: &str) -> (TempDir, PathBuf) {
    let (temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", stack_name]);
    assert!(success, "Failed to create stack {stack_name}: {stderr}");
    (temp_dir, repo_path)
}

/// Return the current HEAD sha as a String.
fn head_sha(repo_path: &std::path::Path) -> String {
    let (success, stdout) = run_git(repo_path, &["rev-parse", "HEAD"]);
    assert!(success, "git rev-parse HEAD failed");
    stdout.trim().to_string()
}

#[test]
fn test_undo_list_empty_when_no_ops() {
    let (_temp_dir, repo_path) = setup_undo_test_repo("undo-empty");

    // No mutating ops yet → list should succeed with empty operations array.
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "--list", "--json"]);
    assert!(success, "undo --list failed: stderr={stderr}");

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    let ops = parsed["operations"]
        .as_array()
        .expect("operations must be array");
    // `gg co` above actually records a checkout operation, so this is non-empty.
    // Assert the invariant we care about: only `checkout`-kind records exist.
    for op in ops {
        assert_eq!(
            op["kind"], "checkout",
            "unexpected op kind before any mutations: {op:?}"
        );
    }
}

#[test]
fn test_undo_reverses_drop() {
    let (_temp_dir, repo_path) = setup_undo_test_repo("undo-drop");

    // Build a 3-commit stack.
    for i in 1..=3 {
        fs::write(repo_path.join(format!("f{i}.txt")), format!("content {i}")).unwrap();
        run_git(&repo_path, &["add", "."]);
        run_git(&repo_path, &["commit", "-m", &format!("Commit {i}")]);
    }
    let head_before_drop = head_sha(&repo_path);

    // Drop position 2.
    let (success, _, stderr) = run_gg(&repo_path, &["drop", "2", "--force"]);
    assert!(success, "drop failed: {stderr}");
    let (_, stdout, _) = run_gg(&repo_path, &["ls"]);
    assert!(!stdout.contains("Commit 2"), "drop did not remove Commit 2");

    // Undo — HEAD should return to the pre-drop sha.
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "--json"]);
    assert!(success, "undo failed: stdout={stdout} stderr={stderr}");

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["status"], "succeeded");
    assert_eq!(parsed["undone"]["kind"], "drop");
    assert_eq!(parsed["undone"]["is_undoable"], true);
    assert_eq!(parsed["undone"]["is_undo"], false);
    assert_eq!(parsed["undone"]["touched_remote"], false);

    assert_eq!(
        head_sha(&repo_path),
        head_before_drop,
        "undo should restore pre-drop HEAD"
    );

    let (_, stdout, _) = run_gg(&repo_path, &["ls"]);
    assert!(
        stdout.contains("Commit 2"),
        "undo should restore Commit 2: {stdout}"
    );
}

#[test]
fn test_undo_reverses_sc_amend() {
    let (_temp_dir, repo_path) = setup_undo_test_repo("undo-sc");

    fs::write(repo_path.join("a.txt"), "v1").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Base commit"]);
    let head_before_sc = head_sha(&repo_path);

    // Stage a change and amend via `gg sc -a`.
    fs::write(repo_path.join("a.txt"), "v2").unwrap();
    let (success, _, stderr) = run_gg(&repo_path, &["sc", "-a"]);
    assert!(success, "gg sc -a failed: {stderr}");
    assert_ne!(head_sha(&repo_path), head_before_sc, "sc should move HEAD");

    // Undo the amend.
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "--json"]);
    assert!(success, "undo failed: stdout={stdout} stderr={stderr}");

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["status"], "succeeded");
    assert_eq!(parsed["undone"]["kind"], "squash");

    assert_eq!(
        head_sha(&repo_path),
        head_before_sc,
        "undo should restore pre-sc HEAD"
    );
}

#[test]
fn test_undo_list_newest_first_and_limit() {
    let (_temp_dir, repo_path) = setup_undo_test_repo("undo-list-order");

    // Three commits — three separate mutating ops on top of the checkout.
    for i in 1..=3 {
        fs::write(repo_path.join(format!("f{i}.txt")), format!("content {i}")).unwrap();
        run_git(&repo_path, &["add", "."]);
        run_git(&repo_path, &["commit", "-m", &format!("Commit {i}")]);
    }
    // Two gg mutations to generate distinct log entries.
    run_gg(&repo_path, &["prev"]);
    run_gg(&repo_path, &["next"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "--list", "--json"]);
    assert!(success, "undo --list failed: {stderr}");

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    let ops = parsed["operations"]
        .as_array()
        .expect("operations must be array");
    assert!(ops.len() >= 2, "expected at least 2 ops, got {}", ops.len());

    // Newest-first: created_at_ms is monotonically non-increasing.
    let mut prev: u64 = u64::MAX;
    for op in ops {
        let ts = op["created_at_ms"]
            .as_u64()
            .expect("created_at_ms must be u64");
        assert!(
            ts <= prev,
            "operations must be newest-first; prev={prev} ts={ts}"
        );
        prev = ts;
    }

    // --limit caps the response.
    let (success, stdout, _) = run_gg(&repo_path, &["undo", "--list", "--limit", "1", "--json"]);
    assert!(success);
    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    let ops = parsed["operations"].as_array().unwrap();
    assert_eq!(ops.len(), 1, "--limit 1 must return exactly 1 op");
}

#[test]
fn test_undo_is_itself_recorded_and_double_undo_redoes() {
    let (_temp_dir, repo_path) = setup_undo_test_repo("undo-redo");

    // One commit → one drop so there is something to undo.
    fs::write(repo_path.join("a.txt"), "v1").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit A"]);
    fs::write(repo_path.join("b.txt"), "v1").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit B"]);
    let head_with_both = head_sha(&repo_path);

    // Drop commit A (position 1).
    let (success, _, stderr) = run_gg(&repo_path, &["drop", "1", "--force"]);
    assert!(success, "drop failed: {stderr}");
    let head_after_drop = head_sha(&repo_path);
    assert_ne!(head_after_drop, head_with_both);

    // First undo → back to head_with_both.
    let (success, _, stderr) = run_gg(&repo_path, &["undo"]);
    assert!(success, "first undo failed: {stderr}");
    assert_eq!(head_sha(&repo_path), head_with_both);

    // Second undo should redo (reverse the undo), landing back at head_after_drop.
    let (success, _, stderr) = run_gg(&repo_path, &["undo"]);
    assert!(success, "second undo (redo) failed: {stderr}");
    assert_eq!(
        head_sha(&repo_path),
        head_after_drop,
        "second undo should redo to post-drop HEAD"
    );

    // The log should contain at least one undo-kind record with is_undo=true
    // and an `undoes` field pointing at another op.
    let (_, stdout, _) = run_gg(&repo_path, &["undo", "--list", "--json"]);
    let parsed: Value = serde_json::from_str(&stdout).expect("valid JSON");
    let ops = parsed["operations"].as_array().unwrap();
    let has_undo_record = ops.iter().any(|op| {
        op["kind"] == "undo"
            && op["is_undo"] == true
            && op["undoes"].as_str().is_some_and(|s| s.starts_with("op_"))
    });
    assert!(
        has_undo_record,
        "expected an undo-kind record with is_undo=true and undoes='op_...': {ops:?}"
    );
}

#[test]
fn test_undo_unknown_operation_id_errors() {
    let (_temp_dir, repo_path) = setup_undo_test_repo("undo-unknown");

    // Pass a bogus operation id; command must fail cleanly.
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "op_does_not_exist_xxxxxxxx"]);
    assert!(
        !success,
        "unknown operation_id must fail; stdout={stdout} stderr={stderr}"
    );
    // We don't pin the exact error text, but it must mention the id or "not found".
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.to_lowercase().contains("not found")
            || combined.contains("op_does_not_exist_xxxxxxxx"),
        "error should reference the missing op id: {combined}"
    );
}

#[test]
fn test_undo_help_mentions_list_and_json() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["undo", "--help"]);
    assert!(success);
    assert!(stdout.contains("--list"), "help must document --list");
    assert!(stdout.contains("--json"), "help must document --json");
    assert!(stdout.contains("--limit"), "help must document --limit");
}

#[test]
fn test_undo_roundtrip_from_worktree() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let stack_name = "undo-wt-test";
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", stack_name, "--worktree"]);
    assert!(success, "Failed to create worktree stack: {stderr}");

    let worktree_path = repo_path.parent().unwrap().join(format!(
        "{}.{}",
        repo_path.file_name().unwrap().to_string_lossy(),
        stack_name
    ));
    assert!(
        worktree_path.exists(),
        "Worktree should exist at {}",
        worktree_path.display()
    );
    let wt = worktree_path.to_path_buf();

    for i in 1..=3 {
        fs::write(
            worktree_path.join(format!("f{i}.txt")),
            format!("content {i}"),
        )
        .unwrap();
        run_git(&wt, &["add", "."]);
        run_git(&wt, &["commit", "-m", &format!("Commit {i}")]);
    }
    let head_before_drop = head_sha(&wt);

    let (success, _, stderr) = run_gg(&wt, &["drop", "2", "--force"]);
    assert!(success, "drop from worktree failed: {stderr}");
    assert_ne!(
        head_sha(&wt),
        head_before_drop,
        "drop should move HEAD in the worktree"
    );

    let (success, stdout, stderr) = run_gg(&wt, &["undo", "--list", "--json"]);
    assert!(success, "undo --list from worktree failed: {stderr}");
    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    let ops = parsed["operations"]
        .as_array()
        .expect("operations must be array");
    assert!(
        ops.iter().any(|op| op["kind"] == "drop"),
        "worktree op-log should include the drop record: {ops:?}"
    );

    let (success, stdout, stderr) = run_gg(&wt, &["undo", "--json"]);
    assert!(
        success,
        "undo from worktree failed: stdout={stdout} stderr={stderr}"
    );
    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["status"], "succeeded");
    assert_eq!(parsed["undone"]["kind"], "drop");

    assert_eq!(
        head_sha(&wt),
        head_before_drop,
        "undo from worktree should restore pre-drop HEAD"
    );
}
