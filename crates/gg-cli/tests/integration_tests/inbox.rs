use crate::helpers::{create_test_repo, run_gg, run_gg_with_env, run_git};

use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

fn parse_jsonl(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("every JSONL line must parse"))
        .collect()
}

#[test]
fn test_gg_inbox_help_mentions_jsonl() {
    let (_temp_dir, repo_path) = create_test_repo();

    let (success, stdout, stderr) = run_gg(&repo_path, &["inbox", "--help"]);

    assert!(success, "gg inbox --help failed: {stderr}");
    assert!(stdout.contains("--jsonl"), "help should mention --jsonl");
}

#[test]
fn test_gg_inbox_rejects_json_and_jsonl_together() {
    let (_temp_dir, repo_path) = create_test_repo();

    let (success, _stdout, stderr) = run_gg(&repo_path, &["inbox", "--json", "--jsonl"]);

    assert!(!success, "--json and --jsonl should conflict");
    assert!(
        stderr.contains("cannot be used with"),
        "Clap should report an explicit flag conflict: {stderr}"
    );
}

#[test]
fn test_gg_inbox_jsonl_fatal_error_uses_inbox_envelope() {
    let temp_dir = TempDir::new().expect("create non-repository directory");

    let (success, stdout, stderr) = run_gg(temp_dir.path(), &["inbox", "--jsonl"]);

    assert!(!success, "fatal inbox error should exit nonzero");
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSONL mode: {stderr}"
    );
    let events = parse_jsonl(&stdout);
    assert_eq!(events.len(), 1, "fatal error should emit one event");
    assert_eq!(events[0]["version"], 1);
    assert_eq!(events[0]["command"], "inbox");
    assert_eq!(events[0]["status"], "error");
    assert_eq!(events[0]["event"], "error");
    assert!(events[0]["message"].is_string());
}

#[test]
fn test_gg_inbox_jsonl_empty_inbox_emits_start_then_summary() {
    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"},"stacks":{}}"#,
    )
    .expect("Failed to write config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["inbox", "--jsonl"]);

    assert!(success, "gg inbox --jsonl failed: {stderr}");
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSONL mode: {stderr}"
    );
    let events = parse_jsonl(&stdout);
    assert_eq!(
        events.len(),
        2,
        "empty inbox should emit exactly two events"
    );
    assert_eq!(events[0]["version"], 1);
    assert_eq!(events[0]["command"], "inbox");
    assert_eq!(events[0]["event"], "start");
    assert_eq!(events[0]["total_candidates"], 0);
    assert_eq!(events[1]["event"], "summary");
    assert_eq!(events[1]["total_items"], 0);
}

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
        "refresh_failed",
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

fn write_fake_gh(temp_dir: &TempDir, fails_pr_view: bool) -> (PathBuf, PathBuf) {
    let fake_bin = temp_dir.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("create fake gh directory");
    let log_path = temp_dir.path().join("gh-requests.log");
    let pr_view_response = if fails_pr_view {
        "echo 'simulated provider failure' >&2\n  exit 1"
    } else {
        r#"printf '%s\n' '{"number":42,"title":"Inbox item","state":"OPEN","url":"https://github.com/acme/app/pull/42","headRefName":"testuser/inbox-copy/c-abc1234","isDraft":false,"mergeable":"MERGEABLE","reviewDecision":"APPROVED","statusCheckRollup":[{"status":"COMPLETED","conclusion":"SUCCESS"}]}'
  exit 0"#
    };
    fs::write(
        fake_bin.join("gh"),
        format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "gh version test"
  exit 0
fi
printf '%s\n' "$*" >> "$GG_FAKE_LOG"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  {pr_view_response}
fi
exit 1
"#
        ),
    )
    .expect("write fake gh");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(fake_bin.join("gh"), fs::Permissions::from_mode(0o755))
            .expect("make fake gh executable");
    }

    (fake_bin, log_path)
}

#[test]
fn test_gg_inbox_uses_one_snapshot_request_per_github_candidate() {
    let (temp_dir, repo_path) = create_repo_with_inbox_item("github", 42);
    let (fake_bin, log_path) = write_fake_gh(&temp_dir, false);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["inbox", "--json"],
        &[
            ("PATH", fake_bin.as_os_str()),
            ("GG_FAKE_LOG", log_path.as_os_str()),
        ],
    );
    assert!(success, "gg inbox --json failed: {stderr}");

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(
        parsed["buckets"]["ready_to_land"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        fs::read_to_string(log_path)
            .expect("read fake gh log")
            .lines()
            .filter(|line| line.starts_with("pr view "))
            .count(),
        1,
        "one candidate should issue exactly one snapshot request"
    );
}

#[test]
fn test_gg_inbox_reports_snapshot_refresh_failures_without_failing() {
    let (temp_dir, repo_path) = create_repo_with_inbox_item("github", 42);
    let (fake_bin, log_path) = write_fake_gh(&temp_dir, true);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["inbox", "--json"],
        &[
            ("PATH", fake_bin.as_os_str()),
            ("GG_FAKE_LOG", log_path.as_os_str()),
        ],
    );
    assert!(success, "refresh failures should not fail inbox: {stderr}");

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    let refresh_failed = parsed["buckets"]["refresh_failed"]
        .as_array()
        .expect("refresh_failed must be an array");
    assert_eq!(refresh_failed.len(), 1);
    assert!(refresh_failed[0].get("refresh_error").is_some());
    assert!(parsed["buckets"]["awaiting_review"]
        .as_array()
        .expect("awaiting_review must be an array")
        .is_empty());
}

#[test]
fn test_gg_inbox_jsonl_success_emits_entry_and_atomic_summary() {
    let (temp_dir, repo_path) = create_repo_with_inbox_item("github", 42);
    let (fake_bin, log_path) = write_fake_gh(&temp_dir, false);
    let env = [
        ("PATH", fake_bin.as_os_str()),
        ("GG_FAKE_LOG", log_path.as_os_str()),
    ];

    let (atomic_success, atomic_stdout, atomic_stderr) =
        run_gg_with_env(&repo_path, &["inbox", "--json"], &env);
    assert!(atomic_success, "gg inbox --json failed: {atomic_stderr}");
    let atomic: Value =
        serde_json::from_str(&atomic_stdout).expect("atomic stdout must be valid JSON");

    let (success, stdout, stderr) = run_gg_with_env(&repo_path, &["inbox", "--jsonl"], &env);

    assert!(success, "gg inbox --jsonl failed: {stderr}");
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSONL mode: {stderr}"
    );
    let events = parse_jsonl(&stdout);
    assert_eq!(
        events
            .iter()
            .map(|event| event["event"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["start", "entry", "summary"]
    );
    assert_eq!(events[1]["completed"], 1);
    assert_eq!(events[1]["total_candidates"], 1);
    assert_eq!(events[1]["included"], true);
    assert_eq!(events[1]["bucket"], "ready_to_land");
    assert_eq!(events[1]["remote_state"], "open");
    assert_eq!(events[1]["entry"]["pr_number"], 42);

    let summary = events.last().expect("summary event");
    for field in ["total_items", "buckets", "stack_errors"] {
        assert_eq!(
            summary[field], atomic[field],
            "streaming summary field {field} should match atomic JSON"
        );
    }
}

#[test]
fn test_gg_inbox_jsonl_refresh_failure_emits_entry_error_and_summary() {
    let (temp_dir, repo_path) = create_repo_with_inbox_item("github", 42);
    let (fake_bin, log_path) = write_fake_gh(&temp_dir, true);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["inbox", "--jsonl"],
        &[
            ("PATH", fake_bin.as_os_str()),
            ("GG_FAKE_LOG", log_path.as_os_str()),
        ],
    );

    assert!(
        success,
        "refresh failures should not fail inbox JSONL: {stderr}"
    );
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSONL mode: {stderr}"
    );
    let events = parse_jsonl(&stdout);
    assert_eq!(
        events
            .iter()
            .map(|event| event["event"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["start", "entry_error", "summary"]
    );
    assert_eq!(events[1]["completed"], 1);
    assert_eq!(events[1]["total_candidates"], 1);
    assert_eq!(events[1]["included"], true);
    assert_eq!(events[1]["bucket"], "refresh_failed");
    assert_eq!(events[1]["entry"]["pr_number"], 42);
    assert!(events[1]["error"]
        .as_str()
        .expect("entry error message")
        .contains("simulated provider failure"));
    assert_eq!(events[2]["total_items"], 1);
    assert_eq!(
        events[2]["buckets"]["refresh_failed"]
            .as_array()
            .expect("refresh_failed bucket")
            .len(),
        1
    );
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
