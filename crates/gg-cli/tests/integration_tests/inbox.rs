use crate::helpers::{create_test_repo, run_gg, run_git};

use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

#[test]
fn test_gg_inbox_json_no_stacks() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"},"stacks":{}}"#,
    )
    .expect("Failed to write config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["inbox", "--json"]);
    assert!(success, "gg inbox --json failed: {}", stderr);
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSON mode"
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["total_items"], 0);

    let buckets = parsed["buckets"]
        .as_object()
        .expect("buckets must be an object");
    for key in [
        "ready_to_land",
        "changes_requested",
        "blocked_on_ci",
        "awaiting_review",
        "behind_base",
        "draft",
    ] {
        assert!(
            buckets[key]
                .as_array()
                .expect("bucket must be an array")
                .is_empty(),
            "bucket {key} should be empty"
        );
    }
}

#[test]
fn test_gg_inbox_json_reports_skipped_stacks_without_failing() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"},"stacks":{"stale":{"base":"main","mrs":{}}}}"#,
    )
    .expect("Failed to write config");

    run_git(
        &repo_path,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/test/repo.git",
        ],
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["inbox", "--json"]);
    assert!(success, "gg inbox --json failed: {}", stderr);
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSON mode"
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["total_items"], 0);

    let stack_errors = parsed["stack_errors"]
        .as_array()
        .expect("stack_errors must be an array");
    assert!(
        stack_errors.is_empty(),
        "stale config without a matching local stack branch should be ignored, got: {stack_errors:?}"
    );
}

#[test]
fn test_gg_inbox_json_finds_stack_branch_without_configured_username() {
    let (_temp_dir, repo_path) = create_test_repo();

    run_git(&repo_path, &["checkout", "-b", "alice/demo"]);
    fs::write(repo_path.join("demo.txt"), "demo").expect("Failed to write demo file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Demo commit"]);

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(gg_dir.join("config.json"), r#"{"stacks":{}}"#).expect("Failed to write config");
    run_git(
        &repo_path,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/test/repo.git",
        ],
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["inbox", "--json"]);
    assert!(success, "gg inbox --json failed: {}", stderr);

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["total_items"], 0);
    assert!(
        parsed.get("stack_errors").is_none()
            || parsed["stack_errors"]
                .as_array()
                .expect("stack_errors must be an array")
                .is_empty()
    );
}

#[test]
fn test_gg_inbox_json_handles_same_stack_name_across_usernames() {
    let (_temp_dir, repo_path) = create_test_repo();

    run_git(&repo_path, &["checkout", "-b", "stale/demo"]);
    fs::write(repo_path.join("stale.txt"), "stale").expect("Failed to write stale file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Stale commit"]);

    run_git(&repo_path, &["checkout", "main"]);
    run_git(&repo_path, &["checkout", "-b", "real/demo"]);
    fs::write(repo_path.join("real.txt"), "real").expect("Failed to write real file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Real commit"]);

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"stale"},"stacks":{"demo":{"base":"main","mrs":{}}}}"#,
    )
    .expect("Failed to write config");
    run_git(
        &repo_path,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/test/repo.git",
        ],
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["inbox", "--json"]);
    assert!(success, "gg inbox --json failed: {}", stderr);

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["total_items"], 0);

    let stack_errors = parsed
        .get("stack_errors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        stack_errors.len() <= 1,
        "expected at most one skipped stack for this setup"
    );
    if let Some(first_error) = stack_errors.first() {
        assert_eq!(first_error["stack_name"], "demo");
    }
}

fn create_repo_with_inbox_item(provider: &str, mr_number: u64) -> (TempDir, PathBuf) {
    let (temp_dir, repo_path) = create_test_repo();

    run_git(&repo_path, &["checkout", "-b", "testuser/inbox-copy"]);
    fs::write(repo_path.join("inbox.txt"), "inbox item").expect("Failed to write inbox file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Inbox item\n\nGG-ID: c-abc1234"],
    );

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        format!(
            r#"{{"defaults":{{"branch_username":"testuser","provider":"{provider}"}},"stacks":{{"inbox-copy":{{"base":"main","mrs":{{"c-abc1234":{mr_number}}}}}}}}}"#
        ),
    )
    .expect("Failed to write config");

    (temp_dir, repo_path)
}

#[test]
fn test_gg_inbox_human_uses_gitlab_mr_label() {
    let (_temp_dir, repo_path) = create_repo_with_inbox_item("gitlab", 42);

    let (success, stdout, stderr) = run_gg(&repo_path, &["inbox"]);
    assert!(success, "gg inbox failed: {}", stderr);

    assert!(
        stderr.contains("Refreshing MR status"),
        "stderr should use MR wording for GitLab, got: {stderr}"
    );
    assert!(
        stdout.contains("MR !42"),
        "stdout should use MR !number for GitLab, got: {stdout}"
    );
    assert!(
        !stdout.contains("PR #42"),
        "stdout should not use GitHub PR wording for GitLab, got: {stdout}"
    );
}

#[test]
fn test_gg_inbox_human_uses_github_pr_label() {
    let (_temp_dir, repo_path) = create_repo_with_inbox_item("github", 43);

    let (success, stdout, stderr) = run_gg(&repo_path, &["inbox"]);
    assert!(success, "gg inbox failed: {}", stderr);

    assert!(
        stderr.contains("Refreshing PR status"),
        "stderr should use PR wording for GitHub, got: {stderr}"
    );
    assert!(
        stdout.contains("PR #43"),
        "stdout should use PR #number for GitHub, got: {stdout}"
    );
}
