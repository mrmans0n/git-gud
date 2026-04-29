use crate::helpers::{create_test_repo, run_gg, run_git};

use serde_json::Value;
use std::fs;

#[test]
fn test_gg_log_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["log", "--help"]);

    assert!(success, "gg log --help failed");
    assert!(stdout.contains("smartlog"), "help should mention smartlog");
    assert!(stdout.contains("--json"));
    assert!(stdout.contains("--refresh"));
}

#[test]
fn test_gg_log_shows_stack_tree() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "log-tree"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("a.txt"), "a").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add A"]);

    fs::write(repo_path.join("b.txt"), "b").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add B"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["log"]);
    assert!(success, "gg log failed: {}", stderr);

    assert!(stdout.contains("log-tree"), "stack name should appear");
    assert!(stdout.contains("[1]"), "position 1 should appear");
    assert!(stdout.contains("[2]"), "position 2 should appear");
    assert!(stdout.contains("Add A"), "commit title should appear");
    assert!(stdout.contains("Add B"), "commit title should appear");
    // Tree glyphs: non-last uses ├──, last uses └──
    assert!(
        stdout.contains("├──"),
        "tree tee glyph should appear: {stdout}"
    );
    assert!(
        stdout.contains("└──"),
        "tree corner glyph should appear: {stdout}"
    );
    // HEAD marker should appear on the currently-checked-out (latest) commit
    assert!(
        stdout.contains("HEAD"),
        "HEAD marker should appear: {stdout}"
    );
}

#[test]
fn test_gg_log_json_shape() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "log-json"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("a.txt"), "a").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add A\n\nGG-ID: c-abc1234"]);

    fs::write(repo_path.join("b.txt"), "b").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add B\n\nGG-ID: c-def5678"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["log", "--json"]);
    assert!(success, "gg log --json failed: {}", stderr);

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["log"]["stack"], "log-json");
    let base = parsed["log"]["base"]
        .as_str()
        .expect("log.base must be a string");
    assert!(
        matches!(base, "main" | "master"),
        "expected base 'main' or 'master', got '{base}'"
    );

    let entries = parsed["log"]["entries"]
        .as_array()
        .expect("entries must be an array");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["position"], 1);
    assert_eq!(entries[0]["title"], "Add A");
    assert_eq!(entries[1]["position"], 2);
    assert_eq!(entries[1]["title"], "Add B");

    // Current position should be the last entry (HEAD sits at stack head by default).
    let current = parsed["log"]["current_position"]
        .as_u64()
        .expect("current_position should be populated");
    assert_eq!(current, 2);
    assert_eq!(entries[1]["is_current"], true);
    assert_eq!(entries[0]["is_current"], false);
}

#[test]
fn test_gg_log_warns_on_mismatched_stack_prefix() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "wrong-prefix-log"]);
    assert!(success, "Failed to create stack: {}", stderr);
    run_git(&repo_path, &["branch", "-m", "other/wrong-prefix-log"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["log"]);
    assert!(success, "gg log failed: {}", stderr);
    assert!(
        stdout.contains("Warning:"),
        "warning should appear: {stdout}"
    );
    assert!(
        stdout.contains("configured prefix 'testuser/'"),
        "configured prefix should appear: {stdout}"
    );
    assert!(
        stdout.contains("git branch -m testuser/wrong-prefix-log"),
        "rename hint should appear: {stdout}"
    );
}

// ── Unstack tests ──────────────────────────────────────────────

#[test]
fn test_gg_log_empty_stack() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "log-empty"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // No commits yet — stack is empty.
    let (success, stdout, stderr) = run_gg(&repo_path, &["log"]);
    assert!(success, "gg log on empty stack should succeed: {}", stderr);
    assert!(stdout.contains("log-empty"), "stack name should appear");
    assert!(
        stdout.contains("empty stack"),
        "empty-stack hint should appear: {stdout}"
    );

    // JSON shape should still be valid with zero entries.
    let (success, stdout, stderr) = run_gg(&repo_path, &["log", "--json"]);
    assert!(success, "gg log --json on empty stack failed: {}", stderr);

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["log"]["stack"], "log-empty");
    let entries = parsed["log"]["entries"]
        .as_array()
        .expect("entries must be an array");
    assert!(entries.is_empty(), "empty stack should have zero entries");
}

#[test]
fn test_gg_log_not_on_stack() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Not on any stack branch — still on `main`.
    let (success, stdout, stderr) = run_gg(&repo_path, &["log"]);
    assert!(!success, "gg log off-stack should fail: stdout={stdout}");
    assert!(
        stderr.contains("not a stack branch"),
        "stderr should explain we are not on a stack: {stderr}"
    );

    // JSON mode surfaces the same condition as a structured error on stdout.
    let (success, stdout, _stderr) = run_gg(&repo_path, &["log", "--json"]);
    assert!(!success, "gg log --json off-stack should fail");
    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert!(parsed["error"].is_string(), "error field must be string");
}

#[test]
fn test_stack_command_on_bare_branch_suggests_configured_prefix() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");
    run_git(&repo_path, &["checkout", "-b", "bare-feature"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["log"]);
    assert!(!success, "gg log off-stack should fail: stdout={stdout}");
    assert!(
        stderr.contains("Current branch 'bare-feature' is not a stack branch"),
        "stderr should include current branch: {stderr}"
    );
    assert!(
        stderr.contains("git branch -m testuser/bare-feature"),
        "stderr should include rename hint: {stderr}"
    );
}
