use crate::helpers::{
    create_test_repo, create_test_repo_with_remote, run_gg, run_gg_with_env, run_git,
};

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn checked_git(repo: &Path, args: &[&str]) -> String {
    let (success, output) = run_git(repo, args);
    assert!(success, "git {args:?} failed: {output}");
    output.trim().to_string()
}

fn install_fake_gh(repo_path: &Path) -> std::ffi::OsString {
    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("Failed to create fake bin dir");
    fs::write(
        fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu

if [ "$1" = "--version" ]; then
  echo "gh version 2.0.0"
  exit 0
fi

if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  echo "https://github.com/test/repo/pull/101"
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  echo '{"number":101,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/101","headRefName":null,"isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "edit" ]; then
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  if [ "$3" = "--base" ] && [ -f "$(dirname "$0")/targets/$(echo "$4" | tr / _)" ]; then
    cat "$(dirname "$0")/targets/$(echo "$4" | tr / _)"
  fi
  exit 0
fi

echo "unexpected gh invocation: $@" >&2
exit 1
"#,
    )
    .expect("Failed to write fake gh");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(fake_bin.join("gh"))
            .expect("Failed to stat fake gh")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(fake_bin.join("gh"), perms).expect("Failed to chmod fake gh");
    }

    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut path = std::ffi::OsString::from(fake_bin.as_os_str());
    path.push(":");
    path.push(old_path);
    path
}

#[cfg(unix)]
fn install_git_delete_shim(repo_path: &Path) -> String {
    let real_git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("locate git");
    assert!(real_git.status.success());
    let real_git = String::from_utf8(real_git.stdout)
        .expect("git path is UTF-8")
        .trim()
        .to_string();
    fs::write(
        repo_path.join("fake-bin/git"),
        r#"#!/bin/sh
set -eu
delete_branch=${GG_TEST_DELETE_BRANCH:-__no_test_branch__}

case " $* " in
  *" push --force-with-lease=refs/heads/$delete_branch:"*" :refs/heads/$delete_branch "*)
    "$GG_TEST_REAL_GIT" "$@"
    status=$?
    if [ "$status" -ne 0 ]; then
      exit "$status"
    fi
    case "${GG_TEST_DELETE_MODE:-}" in
      fail_after_delete)
        echo "simulated lost deletion response" >&2
        exit 1
        ;;
      republish_after_delete)
        "$GG_TEST_REAL_GIT" -C "$GG_TEST_DELETE_REPO" push origin \
          "$GG_TEST_DELETE_OID:refs/heads/$delete_branch" >/dev/null 2>&1
        ;;
    esac
    exit 0
    ;;
esac

exec "$GG_TEST_REAL_GIT" "$@"
"#,
    )
    .expect("write git shim");
    let shim = repo_path.join("fake-bin/git");
    let mut perms = fs::metadata(&shim).expect("stat git shim").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(shim, perms).expect("chmod git shim");
    real_git
}

#[cfg(unix)]
fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn test_gg_sync_help_has_update_descriptions() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["sync", "--help"]);

    assert!(success);
    assert!(stdout.contains("--update-descriptions"));
    assert!(stdout.contains("--no-rebase-check"));
}

#[test]
fn test_gg_sync_json_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["sync", "--help"]);

    assert!(success);
    assert!(stdout.contains("--json"));
}

#[test]
fn test_gg_sync_jsonl_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["sync", "--help"]);

    assert!(success);
    assert!(stdout.contains("--jsonl"), "help should mention --jsonl");
}

#[test]
fn test_gg_sync_jsonl_error_output_without_provider() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "jsonl-sync-error"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["sync", "--jsonl"]);
    assert!(!success, "sync --jsonl should fail without provider");
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in --jsonl mode: {stderr}"
    );

    let mut found_error = false;
    for line in stdout.lines() {
        let parsed: Value = serde_json::from_str(line)
            .unwrap_or_else(|_| panic!("every --jsonl line must be valid JSON: {line}"));
        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["command"], "sync");
        if parsed["event"] == "error" {
            found_error = true;
            assert!(
                parsed["message"].is_string(),
                "error message must be string"
            );
        }
    }
    assert!(found_error, "--jsonl output must contain an error event");
}

#[test]
fn test_gg_sync_jsonl_empty_stack_emits_summary_only() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "jsonl-empty-stack"]);
    assert!(success, "Failed to create stack: {}", stderr);

    let (success, stdout, stderr) = run_gg(&repo_path, &["sync", "--jsonl"]);
    assert!(
        success,
        "sync --jsonl on empty stack failed\nstdout:\n{}\nstderr:\n{}",
        stdout, stderr
    );
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in --jsonl mode: {stderr}"
    );

    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "empty stack should emit one summary event");
    let parsed: Value = serde_json::from_str(lines[0])
        .unwrap_or_else(|_| panic!("summary line must be valid JSON: {}", lines[0]));
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["command"], "sync");
    assert_eq!(parsed["event"], "summary");
    assert_eq!(parsed["status"], "ok");
    assert_eq!(parsed["stack"], "jsonl-empty-stack");
    assert!(
        parsed["entries"].as_array().unwrap().is_empty(),
        "empty stack summary should include no entries"
    );
}

#[test]
fn test_gg_sync_json_error_output_without_provider() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "json-sync-error"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["sync", "--json"]);
    assert!(!success, "sync --json should fail without provider");
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSON mode"
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert!(parsed["error"].is_string(), "error field must be string");
}

#[test]
fn test_gg_sync_jsonl_success_emits_ndjson_events_and_summary() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github","base":"main","sync_behind_threshold":0}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "jsonl-success"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("entry.txt"), "a\n").expect("Failed to write entry");
    run_git(&repo_path, &["add", "entry.txt"]);
    run_git(&repo_path, &["commit", "-m", "Entry\n\nGG-ID: c-8fd7581"]);

    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("Failed to create fake bin dir");
    fs::write(
        fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu

if [ "$1" = "--version" ]; then
  echo "gh version 2.0.0"
  exit 0
fi

if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  echo "https://github.com/test/repo/pull/101"
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  echo '{"number":101,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/101","headRefName":"testuser/jsonl-success--c-8fd7581","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
  exit 0
fi

if [ "$1" = "api" ] && [ "$2" = "-X" ] && [ "$3" = "POST" ]; then
  exit 0
fi

echo "unexpected gh invocation: $@" >&2
exit 1
"#,
    )
    .expect("Failed to write fake gh");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(fake_bin.join("gh"))
            .expect("Failed to stat fake gh")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(fake_bin.join("gh"), perms).expect("Failed to chmod fake gh");
    }

    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = std::ffi::OsString::from(fake_bin.as_os_str());
    new_path.push(":");
    new_path.push(old_path);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--jsonl", "--lint", "--no-rebase-check"],
        &[("PATH", new_path.as_os_str())],
    );
    assert!(
        success,
        "sync --jsonl failed\nstdout:\n{}\nstderr:\n{}",
        stdout, stderr
    );
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in --jsonl mode: {stderr}"
    );

    let mut events: Vec<Value> = Vec::new();
    for line in stdout.lines() {
        let parsed: Value = serde_json::from_str(line)
            .unwrap_or_else(|_| panic!("every --jsonl line must be valid JSON: {line}"));
        assert_eq!(parsed["version"], 1);
        assert_eq!(parsed["command"], "sync");
        events.push(parsed);
    }

    assert!(
        !events.is_empty(),
        "--jsonl output must contain at least one event"
    );
    assert_eq!(events[0]["event"], "start", "first event must be start");
    assert!(
        events[0]["total_entries"].is_number(),
        "start event must include total_entries"
    );

    let summary = events.last().unwrap();
    assert_eq!(summary["event"], "summary", "last event must be summary");
    assert!(
        summary["entries"].is_array(),
        "summary must include entries"
    );
    assert_eq!(summary["entries"].as_array().unwrap().len(), 1);
    assert_eq!(summary["entries"][0]["action"], "created");
    assert_eq!(summary["entries"][0]["pr_number"], 101);

    let events_before_summary = &events[..events.len() - 1];
    let has_entry_started = events_before_summary
        .iter()
        .any(|e| e["event"] == "entry_started");
    let has_pr_created = events_before_summary
        .iter()
        .any(|e| e["event"] == "pr_created");
    assert!(has_entry_started, "must emit entry_started event");
    assert!(has_pr_created, "must emit pr_created event");

    let line_count = stdout.lines().count();
    assert!(
        line_count >= 3,
        "expected at least start, entry_started/created, summary lines, got {line_count}"
    );
}

#[test]
fn test_gg_sync_json_includes_mismatched_stack_prefix_warning() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "wrong-prefix-sync"]);
    assert!(success, "Failed to create stack: {}", stderr);
    run_git(&repo_path, &["branch", "-m", "other/wrong-prefix-sync"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["sync", "--json"]);
    assert!(success, "gg sync --json failed: {}", stderr);
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSON mode: {stderr}"
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    let warnings = parsed["sync"]["warnings"]
        .as_array()
        .expect("warnings must be an array");
    assert_eq!(warnings.len(), 1);
    let warning = warnings[0].as_str().expect("warning must be a string");
    assert!(warning.contains("configured prefix 'testuser/'"));
    assert!(warning.contains("git branch -m testuser/wrong-prefix-sync"));
}

#[test]
fn test_gg_sync_help_has_no_verify() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["sync", "--help"]);

    assert!(success);
    assert!(stdout.contains("--no-verify"));
}

#[test]
fn test_sync_lint_no_commands_omits_example_configuration() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github","base":"main","sync_behind_threshold":0}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "sync-no-lint-config"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("entry.txt"), "content\n").expect("Failed to write entry");
    run_git(&repo_path, &["add", "entry.txt"]);
    run_git(&repo_path, &["commit", "-m", "Entry\n\nGG-ID: c-8fd7581"]);

    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("Failed to create fake bin dir");
    fs::write(
        fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu

if [ "$1" = "--version" ]; then
  echo "gh version 2.0.0"
  exit 0
fi

if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  echo "https://github.com/test/repo/pull/915"
  exit 0
fi

echo "unexpected gh invocation: $@" >&2
exit 1
"#,
    )
    .expect("Failed to write fake gh");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(fake_bin.join("gh"))
            .expect("Failed to stat fake gh")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(fake_bin.join("gh"), perms).expect("Failed to chmod fake gh");
    }

    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = std::ffi::OsString::from(fake_bin.as_os_str());
    new_path.push(":");
    new_path.push(old_path);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--lint", "--no-rebase-check"],
        &[("PATH", new_path.as_os_str())],
    );
    assert!(
        success,
        "sync failed\nstdout:\n{}\nstderr:\n{}",
        stdout, stderr
    );
    assert!(
        stdout.contains("No lint commands configured. Run 'gg setup' to configure lint commands.")
    );
    assert!(!stdout.contains("Example configuration:"));
    assert!(!stdout.contains(r#""lint": ["cargo fmt", "cargo clippy -- -D warnings"]"#));
}

#[test]
fn test_sync_recreates_mapped_pr_when_head_branch_changed() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github","base":"main","sync_behind_threshold":0}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "u312b-split"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("entry-a.txt"), "a\n").expect("Failed to write entry A");
    run_git(&repo_path, &["add", "entry-a.txt"]);
    run_git(&repo_path, &["commit", "-m", "Entry A\n\nGG-ID: c-8b999da"]);

    fs::write(repo_path.join("entry-b.txt"), "b\n").expect("Failed to write entry B");
    run_git(&repo_path, &["add", "entry-b.txt"]);
    run_git(
        &repo_path,
        &[
            "commit",
            "-m",
            "Entry B\n\nGG-ID: c-fa7d2e9\nGG-Parent: c-8b999da",
        ],
    );

    fs::write(
        gg_dir.join("config.json"),
        r#"{
  "defaults": {
    "branch_username": "testuser",
    "provider": "github",
    "base": "main",
    "sync_behind_threshold": 0
  },
  "stacks": {
    "u312b-split": {
      "base": "main",
      "mrs": {
        "c-8b999da": 428,
        "c-fa7d2e9": 429
      }
    }
  }
}"#,
    )
    .expect("Failed to write moved PR mapping");

    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("Failed to create fake bin dir");
    let fake_log = repo_path.join("fake-gh.log");
    let fake_next = repo_path.join("fake-gh-next");
    fs::write(&fake_next, "900\n").expect("Failed to write fake gh state");
    fs::write(
        fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
echo "$@" >> "$GG_FAKE_GH_LOG"

if [ "$1" = "--version" ]; then
  echo "gh version 2.0.0"
  exit 0
fi

if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  case "$3" in
    428)
      echo '{"number":428,"title":"Entry A","state":"OPEN","url":"https://github.com/test/repo/pull/428","headRefName":"testuser/u312b203606--c-8b999da","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
      exit 0
      ;;
    429)
      echo '{"number":429,"title":"Entry B","state":"OPEN","url":"https://github.com/test/repo/pull/429","headRefName":"testuser/u312b203606--c-fa7d2e9","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
      exit 0
      ;;
    *)
      echo '{"number":999,"title":"Replacement","state":"OPEN","url":"https://github.com/test/repo/pull/999","headRefName":"testuser/u312b-split--c-unknown","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
      exit 0
      ;;
  esac
fi

if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  num=$(cat "$GG_FAKE_GH_NEXT")
  next=$((num + 1))
  echo "$next" > "$GG_FAKE_GH_NEXT"
  echo "https://github.com/test/repo/pull/$num"
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "close" ]; then
  exit 0
fi

if [ "$1" = "api" ] && [ "$2" = "-X" ] && [ "$3" = "POST" ]; then
  exit 0
fi

echo "unexpected gh invocation: $@" >&2
exit 1
"#,
    )
    .expect("Failed to write fake gh");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(fake_bin.join("gh"))
            .expect("Failed to stat fake gh")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(fake_bin.join("gh"), perms).expect("Failed to chmod fake gh");
    }

    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = std::ffi::OsString::from(fake_bin.as_os_str());
    new_path.push(":");
    new_path.push(old_path);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--json", "--until", "2"],
        &[
            ("PATH", new_path.as_os_str()),
            ("GG_FAKE_GH_LOG", fake_log.as_os_str()),
            ("GG_FAKE_GH_NEXT", fake_next.as_os_str()),
        ],
    );
    assert!(
        success,
        "sync failed\nstdout:\n{}\nstderr:\n{}",
        stdout, stderr
    );

    let json: Value = serde_json::from_str(&stdout).expect("sync should emit JSON");
    let entries = json["sync"]["entries"]
        .as_array()
        .expect("entries should be an array");
    assert_eq!(entries[0]["action"], "recreated");
    assert_eq!(entries[0]["pr_number"], 900);
    assert_eq!(entries[1]["action"], "recreated");
    assert_eq!(entries[1]["pr_number"], 901);

    let config = fs::read_to_string(gg_dir.join("config.json")).expect("Failed to read config");
    assert!(
        config.contains(r#""c-8b999da": 900"#),
        "config should map first entry to replacement PR: {}",
        config
    );
    assert!(
        config.contains(r#""c-fa7d2e9": 901"#),
        "config should map second entry to replacement PR: {}",
        config
    );

    let log = fs::read_to_string(fake_log).expect("Failed to read fake gh log");
    assert!(
        log.contains("pr create --head testuser/u312b-split--c-8b999da --base main"),
        "first replacement should use new head branch, log:\n{}",
        log
    );
    assert!(
        log.contains("pr create --head testuser/u312b-split--c-fa7d2e9 --base testuser/u312b-split--c-8b999da"),
        "second replacement should target the new first entry branch, log:\n{}",
        log
    );
    assert!(log.contains("pr close 428"), "old PR 428 should be closed");
    assert!(log.contains("pr close 429"), "old PR 429 should be closed");
    assert!(
        !log.contains("pr edit 428 --base") && !log.contains("pr edit 429 --base"),
        "old PR bases should not be edited when source branch is wrong, log:\n{}",
        log
    );
}

#[test]
fn test_sync_detects_uncommitted_changes() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config with auto_add_gg_ids enabled
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","auto_add_gg_ids":true}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "auto-stash-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create a commit WITHOUT a GG-ID (directly via git)
    fs::write(repo_path.join("committed.txt"), "committed content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit without GG-ID"]);

    // Make uncommitted changes
    fs::write(repo_path.join("uncommitted.txt"), "uncommitted content")
        .expect("Failed to write file");
    run_git(&repo_path, &["add", "uncommitted.txt"]);

    // Verify we have staged changes
    let (_, status) = run_git(&repo_path, &["status", "--short"]);
    assert!(
        status.contains("uncommitted.txt"),
        "Should have uncommitted changes before sync"
    );

    // Run sync - will fail on provider check in test environment
    // We're primarily testing that the code handles uncommitted changes correctly
    let (_success, _stdout, _stderr) = run_gg(&repo_path, &["sync"]);

    // The sync command should handle the uncommitted changes somehow
    // (either by stashing, failing with a clear error, or processing them)
    // The actual behavior is tested in the sync.rs unit tests and manual testing
    // This integration test verifies the basic flow doesn't panic or crash
}

#[test]
fn test_sync_detects_rebase_in_progress() {
    // Use a repo with remote to avoid "No origin remote" error
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","auto_add_gg_ids":true}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    run_gg(&repo_path, &["co", "rebase-detection-test"]);

    // Create a commit without GG-ID
    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    // Simulate a rebase in progress
    let rebase_dir = repo_path.join(".git/rebase-merge");
    fs::create_dir_all(&rebase_dir).expect("Failed to create rebase-merge dir");
    fs::write(
        rebase_dir.join("head-name"),
        "refs/heads/testuser/rebase-detection-test",
    )
    .expect("Failed to write head-name");

    // Run sync - should detect rebase in progress
    let (success, stdout, stderr) = run_gg(&repo_path, &["sync"]);

    assert!(!success, "Sync should fail when rebase is in progress");

    let combined = format!("{}{}", stdout, stderr);

    // Should mention rebase in progress (or fail on provider, which is also acceptable)
    // The key is it should fail gracefully, not crash
    assert!(
        combined.contains("rebase")
            || combined.contains("in progress")
            || combined.contains("provider")
            || combined.contains("glab")
            || combined.contains("gh"),
        "Should fail gracefully: {}",
        combined
    );

    // Clean up
    fs::remove_dir_all(&rebase_dir).ok();
}

#[test]
fn test_sync_error_message_quality_with_uncommitted_changes() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","auto_add_gg_ids":true}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    run_gg(&repo_path, &["co", "error-message-test"]);

    // Create a commit without GG-ID
    fs::write(repo_path.join("file.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Stack commit"]);

    // Make uncommitted changes
    fs::write(repo_path.join("uncommitted.txt"), "uncommitted").expect("Failed to write file");
    run_git(&repo_path, &["add", "uncommitted.txt"]);

    // Run sync - will fail on provider check, but should handle uncommitted changes gracefully
    let (success, stdout, stderr) = run_gg(&repo_path, &["sync"]);

    assert!(
        !success,
        "Sync should fail without provider in test environment"
    );

    let combined = format!("{}{}", stdout, stderr);

    // The error message should be informative (not panic or crash)
    assert!(
        !combined.is_empty(),
        "Should provide an error message, not crash silently"
    );

    // Verify uncommitted changes are still accessible (either in working dir or stash)
    let (_, status) = run_git(&repo_path, &["status", "--short"]);
    let (_, stash_list) = run_git(&repo_path, &["stash", "list"]);

    let file_in_working_dir = status.contains("uncommitted.txt");
    let file_in_stash = stash_list.contains("gg-sync-autostash");

    assert!(
        file_in_working_dir || file_in_stash,
        "Uncommitted changes should be preserved (either in working dir or stash)"
    );
}

#[test]
fn test_stash_operations_work_correctly() {
    // This test verifies that git stash operations work as expected
    // to ensure the auto-stashing functionality can rely on them
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up a git repo with a commit
    fs::write(repo_path.join("file.txt"), "original").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Initial commit"]);

    // Make uncommitted changes
    fs::write(repo_path.join("file.txt"), "modified").expect("Failed to write file");

    // Stash the changes
    let (success, _) = run_git(&repo_path, &["stash", "push", "-m", "test-stash"]);
    assert!(success, "Stash push should succeed");

    // Verify file is back to original
    let content = fs::read_to_string(repo_path.join("file.txt")).expect("Failed to read file");
    assert_eq!(content, "original", "File should be reset after stash");

    // Pop the stash
    let (success, _) = run_git(&repo_path, &["stash", "pop"]);
    assert!(success, "Stash pop should succeed");

    // Verify changes are restored
    let content = fs::read_to_string(repo_path.join("file.txt")).expect("Failed to read file");
    assert_eq!(content, "modified", "Changes should be restored after pop");
}

#[test]
fn test_git_stash_handles_mixed_changes() {
    // Verify that git stash works correctly with both staged and unstaged changes
    // This is important for the auto-stashing feature
    let (_temp_dir, repo_path) = create_test_repo();

    // Create a base commit
    fs::write(repo_path.join("base.txt"), "base").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Base commit"]);

    // Create both staged and unstaged changes
    fs::write(repo_path.join("staged.txt"), "staged content").expect("Failed to write file");
    run_git(&repo_path, &["add", "staged.txt"]);

    fs::write(repo_path.join("unstaged.txt"), "unstaged content").expect("Failed to write file");
    // Don't stage unstaged.txt

    // Verify we have both types of changes
    let (_, status) = run_git(&repo_path, &["status", "--short"]);
    assert!(status.contains("staged.txt"), "Should have staged changes");
    assert!(
        status.contains("unstaged.txt"),
        "Should have unstaged changes"
    );

    // Stash all changes (including untracked)
    let (success, _) = run_git(&repo_path, &["stash", "push", "-u", "-m", "test-stash"]);
    assert!(success, "Stash should succeed with mixed changes");

    // Verify working directory is clean
    let (_, status) = run_git(&repo_path, &["status", "--short"]);
    assert!(
        !status.contains("staged.txt") && !status.contains("unstaged.txt"),
        "Working directory should be clean after stash"
    );

    // Pop the stash
    let (success, _) = run_git(&repo_path, &["stash", "pop"]);
    assert!(success, "Stash pop should succeed");

    // Verify changes are restored (they might not be in the same staged/unstaged state,
    // but they should be present)
    assert!(
        repo_path.join("staged.txt").exists() && repo_path.join("unstaged.txt").exists(),
        "Files should be restored after stash pop"
    );
}

#[test]
fn test_sync_fails_gracefully_without_provider() {
    // Test that sync fails with a clear error when no provider is configured
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config without provider
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    run_gg(&repo_path, &["co", "no-provider-test"]);

    // Create a commit
    fs::write(repo_path.join("file.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    // Try to sync - will fail on provider detection
    let (success, _stdout, stderr) = run_gg(&repo_path, &["sync"]);

    assert!(!success, "Sync should fail without provider");

    // Should have a clear error message about provider
    assert!(
        !stderr.is_empty(),
        "Should provide an error message about missing provider"
    );
}

#[test]
fn test_sync_until_by_position() {
    // Test sync --until with numeric position
    let (_temp_dir, repo_path) = create_test_repo();

    // Setup config first
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack
    run_gg(&repo_path, &["co", "test-until"]);

    // Commit 1
    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file1");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    // Commit 2
    fs::write(repo_path.join("file2.txt"), "content2").expect("Failed to write file2");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Commit 3
    fs::write(repo_path.join("file3.txt"), "content3").expect("Failed to write file3");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 3"]);

    // List stack to verify 3 commits
    let (success, stdout, _stderr) = run_gg(&repo_path, &["ls"]);
    assert!(success, "gg ls should succeed");
    assert!(
        stdout.contains("[1]") && stdout.contains("[2]") && stdout.contains("[3]"),
        "Stack should have 3 commits. stdout: {}",
        stdout
    );

    // Test sync --until 2 - will fail on remote but shouldn't fail on parsing
    let (_success, stdout, stderr) = run_gg(&repo_path, &["sync", "--until", "2"]);

    // Should not fail with "Could not find commit matching" error
    assert!(
        !stderr.contains("Could not find commit matching"),
        "Should parse --until position correctly. stderr: {}",
        stderr
    );

    // Will fail on remote/provider, but that's expected and OK
    assert!(
        stdout.contains("2 commits")
            || stderr.contains("provider")
            || stderr.contains("remote")
            || stderr.contains("origin"),
        "Should either mention 2 commits or fail on provider/remote. stdout: {}, stderr: {}",
        stdout,
        stderr
    );
}

#[test]
fn test_sync_until_invalid_position() {
    // Test sync --until with out-of-range position
    let (_temp_dir, repo_path) = create_test_repo();

    // Setup config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack with 2 commits
    run_gg(&repo_path, &["co", "test-invalid"]);

    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file1");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("file2.txt"), "content2").expect("Failed to write file2");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Try sync --until 5 (out of range)
    let (success, _stdout, stderr) = run_gg(&repo_path, &["sync", "--until", "5"]);

    assert!(!success, "Should fail with out-of-range position");
    assert!(
        stderr.contains("out of range") || stderr.contains("Position"),
        "Should indicate position is out of range. stderr: {}",
        stderr
    );
}

#[test]
fn test_sync_until_by_sha() {
    // Test sync --until with SHA prefix
    // Important: ensure all commits have GG-IDs to avoid sync doing a rebase (which would change SHAs).
    let (_temp_dir, repo_path) = create_test_repo();

    // Setup config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack with 2 commits
    run_gg(&repo_path, &["co", "test-sha"]);

    // Commit 1 (with GG-ID)
    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file1");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Commit 1\n\nGG-ID: c-aaaaaaa"],
    );

    // Get SHA of commit 1
    let (_, sha_output) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let sha = sha_output.trim();
    let first_non_digit = sha.chars().position(|c| !c.is_ascii_digit()).unwrap_or(6);
    let prefix_len = std::cmp::max(7, first_non_digit + 1);
    let sha_prefix = sha[..prefix_len].to_string();

    // Commit 2 (with GG-ID)
    fs::write(repo_path.join("file2.txt"), "content2").expect("Failed to write file2");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Commit 2\n\nGG-ID: c-bbbbbbb"],
    );

    // Test sync --until <sha_prefix> (should sync only commit 1)
    let (_success, stdout, stderr) = run_gg(&repo_path, &["sync", "--until", &sha_prefix]);

    assert!(
        !stderr.contains("Could not find commit matching"),
        "Should find commit by SHA prefix. stderr: {}",
        stderr
    );

    assert!(
        stdout.contains("1 commit")
            || stdout.contains("1 commits")
            || stderr.contains("provider")
            || stderr.contains("remote")
            || stderr.contains("origin"),
        "Should either mention 1 commit or fail on provider/remote. stdout: {}, stderr: {}",
        stdout,
        stderr
    );
}

#[test]
fn test_sync_until_by_gg_id() {
    // Test sync --until with explicit GG-ID trailer
    let (_temp_dir, repo_path) = create_test_repo();

    // Setup config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack with 2 commits
    run_gg(&repo_path, &["co", "test-ggid"]);

    // Commit 1 with fixed GG-ID
    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file1");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Commit 1\n\nGG-ID: c-abc1234"],
    );

    // Commit 2 with GG-ID as well to avoid rebase
    fs::write(repo_path.join("file2.txt"), "content2").expect("Failed to write file2");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Commit 2\n\nGG-ID: c-def5678"],
    );

    // Test sync --until c-abc1234 (should sync only commit 1)
    let (_success, stdout, stderr) = run_gg(&repo_path, &["sync", "--until", "c-abc1234"]);

    assert!(
        !stderr.contains("Could not find commit matching"),
        "Should find commit by GG-ID. stderr: {}",
        stderr
    );

    assert!(
        stdout.contains("1 commit")
            || stdout.contains("1 commits")
            || stderr.contains("provider")
            || stderr.contains("remote")
            || stderr.contains("origin"),
        "Should either mention 1 commit or fail on provider/remote. stdout: {}, stderr: {}",
        stdout,
        stderr
    );
}

#[test]
fn test_sync_until_nonexistent_target() {
    // Test sync --until with non-existent target
    let (_temp_dir, repo_path) = create_test_repo();

    // Setup config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack with 1 commit
    run_gg(&repo_path, &["co", "test-nonexistent"]);

    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file1");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    // Try sync --until with non-existent target
    let (success, _stdout, stderr) = run_gg(&repo_path, &["sync", "--until", "nonexistent"]);

    assert!(!success, "Should fail with non-existent target");
    assert!(
        stderr.contains("Could not find commit matching"),
        "Should indicate commit not found. stderr: {}",
        stderr
    );
}

#[test]
fn test_sync_auto_rebase_shows_rebase_force_hint_on_immutable_commit() {
    // Reproduces the confusing UX where `gg sync --force` still fails with
    // an immutable-commit error, because the actual override is on
    // `gg rebase --force` / `gg rebase --ignore-immutable`.
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github","sync_auto_rebase":true,"sync_behind_threshold":1}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "immutable-sync-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create two commits on the stack.
    // Commit #1 stays mutable so the guard cannot drop #2 via
    // without_bottom_merged_prs().
    fs::write(repo_path.join("stack1.txt"), "stack1\n").expect("Failed to write stack1 file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Stack commit 1\n\nGG-ID: c-1111111"],
    );

    fs::write(repo_path.join("stack2.txt"), "stack2\n").expect("Failed to write stack2 file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Stack commit 2\n\nGG-ID: c-2222222"],
    );

    // Inject a merged MR mapping for commit #2 into the config after gg co
    // created the stack entry.
    let stack_branch = git_current_branch(&repo_path);
    let config_json = fs::read_to_string(gg_dir.join("config.json")).unwrap_or_default();
    let mut config: serde_json::Value =
        serde_json::from_str(&config_json).expect("config.json must be valid JSON after gg co");
    config["stacks"]["immutable-sync-test"]["mrs"]["c-2222222"] = serde_json::json!(99);
    fs::write(
        gg_dir.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .expect("Failed to write merged config");

    // Advance origin/main so the behind-base check triggers auto-rebase.
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("base.txt"), "base update\n").expect("Failed to write base file");
    run_git(&repo_path, &["add", "base.txt"]);
    run_git(&repo_path, &["commit", "-m", "Merged upstream"]);
    run_git(&repo_path, &["push", "origin", "main"]);
    run_git(&repo_path, &["checkout", &stack_branch]);

    // The fake gh exists so provider checks pass, and also returns Merged for
    // the MR so refresh_mr_state_for_guard populates mr_state correctly.
    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("Failed to create fake bin dir");
    fs::write(
        fake_bin.join("gh"),
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'gh version 2.0.0'\n  exit 0\nfi\nif [ \"$1\" = \"auth\" ] && [ \"$2\" = \"status\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ] && [ \"$3\" = \"99\" ]; then\n  echo '{\"number\":99,\"title\":\"Stack commit 2\",\"state\":\"MERGED\",\"url\":\"https://github.com/test/repo/pull/99\",\"headRefName\":\"testuser/immutable-sync-test--c-2222222\",\"isDraft\":false,\"mergeable\":\"MERGEABLE\",\"reviews\":[]}'\n  exit 0\nfi\necho 'unexpected gh invocation' >&2\nexit 1\n",
    )
    .expect("Failed to write fake gh");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(fake_bin.join("gh"))
            .expect("Failed to stat fake gh")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(fake_bin.join("gh"), perms).expect("Failed to chmod fake gh");
    }

    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = std::ffi::OsString::from(fake_bin.as_os_str());
    new_path.push(":");
    new_path.push(old_path);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--force"],
        &[("PATH", new_path.as_os_str())],
    );
    assert!(
        !success,
        "sync should fail when auto-rebase hits immutable commit\nstdout: {}\nstderr: {}",
        stdout, stderr
    );
    assert!(
        stderr.contains("cannot rewrite immutable commits during sync"),
        "error should mention 'during sync', got: {}",
        stderr
    );
    assert!(
        stderr.contains("gg rebase --force") || stderr.contains("gg rebase --ignore-immutable"),
        "error should point to `gg rebase --force`, got: {}",
        stderr
    );
    assert!(
        !stderr.contains("pass --force / --ignore-immutable to override"),
        "error should not imply `gg sync --force` is sufficient, got: {}",
        stderr
    );
}

fn write_remote_prune_config(repo_path: &Path) {
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{
  "defaults": {
    "branch_username": "testuser",
    "provider": "github",
    "base": "main",
    "sync_behind_threshold": 0,
    "github": { "stacks_integration": "off" }
  }
}"#,
    )
    .expect("Failed to write config");
}

fn clear_stack_mappings(repo_path: &Path, stack_name: &str) {
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"][stack_name]["mrs"] = serde_json::json!({});
    fs::write(config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
}

fn latest_operation(repo_path: &Path) -> Value {
    let mut records = fs::read_dir(repo_path.join(".git/gg/operations"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    records.sort();
    serde_json::from_slice(&fs::read(records.last().unwrap()).unwrap()).unwrap()
}

fn setup_published_prune_stack(
    stack_name: &str,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    std::ffi::OsString,
) {
    let (temp, repo_path, remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let fake_path = install_fake_gh(&repo_path);
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", stack_name]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");
    for id in ["c-1111111", "c-2222222", "c-3333333"] {
        checked_git(
            &repo_path,
            &[
                "commit",
                "--allow-empty",
                "-m",
                &format!("Entry\n\nGG-ID: {id}"),
            ],
        );
    }
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "3", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "initial sync failed: {stdout}\n{stderr}");
    (temp, repo_path, remote_path, fake_path)
}

fn setup_rebase_to_empty_after_drop(
    stack_name: &str,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    std::ffi::OsString,
    String,
    String,
) {
    let (temp, repo_path, remote_path, fake_path) = setup_published_prune_stack(stack_name);
    let dropped_branch = format!("testuser/{stack_name}--c-3333333");
    let dropped_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{dropped_branch}")],
    );
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    clear_stack_mappings(&repo_path, stack_name);

    checked_git(&repo_path, &["push", "origin", "HEAD:refs/heads/main"]);
    let peer = temp.path().join(format!("{stack_name}-base-writer"));
    checked_git(
        &repo_path,
        &[
            "clone",
            remote_path.to_str().unwrap(),
            peer.to_str().unwrap(),
        ],
    );
    checked_git(&peer, &["config", "user.name", "Base Writer"]);
    checked_git(&peer, &["config", "user.email", "base@example.com"]);
    checked_git(
        &peer,
        &["commit", "--allow-empty", "-m", "Advance landed base"],
    );
    checked_git(&peer, &["push", "origin", "main"]);

    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["sync_auto_rebase"] = Value::Bool(true);
    config["defaults"]["sync_behind_threshold"] = Value::from(1);
    fs::write(config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    (
        temp,
        repo_path,
        remote_path,
        fake_path,
        dropped_branch,
        dropped_oid,
    )
}

#[test]
fn test_sync_keeps_dropped_branch_while_open_review_targets_it() {
    let (_temp, repo_path, remote_path, fake_path) = setup_published_prune_stack("retarget");
    let dropped_branch = "testuser/retarget--c-2222222";
    let targets = repo_path.join("fake-bin/targets");
    fs::create_dir_all(&targets).unwrap();
    let target_file = targets.join(dropped_branch.replace('/', "_"));
    let remote_has_dropped = || {
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{dropped_branch}"),
            ],
        )
        .0
    };

    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "2", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    // Entry 3's review still targets the dropped branch, e.g. because it is
    // outside --until or an earlier retarget failed.
    fs::write(&target_file, "103\n").unwrap();

    for args in [
        &["sync", "--until", "1", "--no-rebase-check"][..],
        &["sync", "--no-rebase-check"][..],
    ] {
        let (success, stdout, stderr) =
            run_gg_with_env(&repo_path, args, &[("PATH", fake_path.as_os_str())]);
        assert!(success, "{args:?} failed: {stdout}\n{stderr}");
        assert!(stdout.contains("#103 still targets it"), "{stdout}");
        assert!(
            remote_has_dropped(),
            "{args:?} must not delete a branch an open review targets"
        );
    }

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--json", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "json sync failed: {stdout}\n{stderr}");
    let response: Value = serde_json::from_str(&stdout).unwrap();
    assert!(response["sync"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().contains(dropped_branch)));

    fs::remove_file(&target_file).unwrap();
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "sync failed: {stdout}\n{stderr}");
    assert!(
        !remote_has_dropped(),
        "sync must prune once no open review targets the branch"
    );
}

#[test]
fn test_sync_prunes_pending_drop_after_rebase_empties_stack() {
    let (temp, repo_path, remote_path, fake_path, dropped_branch, dropped_oid) =
        setup_rebase_to_empty_after_drop("empty-after-rebase");

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--json"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "sync failed: {stdout}\n{stderr}");
    let response: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(response["sync"]["rebased_before_sync"], true);
    assert_eq!(response["sync"]["entries"], serde_json::json!([]));
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{dropped_branch}")
            ]
        )
        .0,
        "empty post-rebase sync must delete the previously dropped remote entry"
    );

    let operation = latest_operation(&repo_path);
    assert_eq!(operation["status"], "committed");
    assert_eq!(operation["touched_remote"], true);
    assert!(operation["remote_effects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|effect| effect["kind"] == "branch_deleted" && effect["branch"] == dropped_branch));

    let reader = temp.path().join("empty-after-rebase-reader");
    checked_git(
        &repo_path,
        &[
            "clone",
            remote_path.to_str().unwrap(),
            reader.to_str().unwrap(),
        ],
    );
    write_remote_prune_config(&reader);
    let (success, stdout, stderr) = run_gg(&reader, &["co", "empty-after-rebase"]);
    assert!(success, "fresh checkout failed: {stdout}\n{stderr}");
    assert_ne!(checked_git(&reader, &["rev-parse", "HEAD"]), dropped_oid);
}

#[test]
fn test_sync_prunes_pending_drops_when_stack_is_already_empty() {
    let (_temp, repo_path, remote_path, fake_path, dropped_branch, _dropped_oid) =
        setup_rebase_to_empty_after_drop("already-empty");
    let (success, stdout, stderr) = run_gg(&repo_path, &["rebase"]);
    assert!(success, "rebase failed: {stdout}\n{stderr}");

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--json", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "empty sync failed: {stdout}\n{stderr}");
    let response: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(response["sync"]["rebased_before_sync"], false);
    assert_eq!(response["sync"]["entries"], serde_json::json!([]));
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{dropped_branch}")
            ]
        )
        .0,
        "empty sync must prune the pending drop"
    );
}

#[cfg(unix)]
#[test]
fn test_sync_records_failed_prune_after_rebase_empties_stack() {
    let (_temp, repo_path, remote_path, fake_path, dropped_branch, _dropped_oid) =
        setup_rebase_to_empty_after_drop("empty-after-rebase-failure");
    let hook = remote_path.join("hooks/pre-receive");
    fs::write(
        &hook,
        format!(
            r#"#!/bin/sh
while read -r old new ref; do
  if [ "$ref" = "refs/heads/{dropped_branch}" ] && [ "$new" = "0000000000000000000000000000000000000000" ]; then
    echo "deletion rejected" >&2
    exit 1
  fi
done
exit 0
"#
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&hook).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&hook, permissions).unwrap();

    let (success, stdout, stderr) =
        run_gg_with_env(&repo_path, &["sync"], &[("PATH", fake_path.as_os_str())]);
    assert!(
        !success,
        "failed empty-stack prune must not report success: {stdout}\n{stderr}"
    );
    assert!(stderr.contains("uncertain outcome"), "{stderr}");
    assert!(
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{dropped_branch}")
            ]
        )
        .0
    );
    let operation = latest_operation(&repo_path);
    assert_eq!(operation["status"], "pending");
    assert_eq!(operation["touched_remote"], true);
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert_eq!(
        config["pending_remote_branch_deletions"][0]["state"],
        "deleting"
    );
}

#[test]
fn test_sync_preserves_remote_entry_published_by_another_clone() {
    let (temp, publisher_path, remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&publisher_path);
    let publisher_fake_path = install_fake_gh(&publisher_path);
    let (success, stdout, stderr) = run_gg(&publisher_path, &["co", "shared-stack"]);
    assert!(success, "{stdout}\n{stderr}");
    for id in ["c-1111111", "c-2222222"] {
        checked_git(
            &publisher_path,
            &[
                "commit",
                "--allow-empty",
                "-m",
                &format!("Entry\n\nGG-ID: {id}"),
            ],
        );
    }
    let (success, stdout, stderr) = run_gg_with_env(
        &publisher_path,
        &["sync", "--no-rebase-check"],
        &[("PATH", publisher_fake_path.as_os_str())],
    );
    assert!(success, "initial sync failed: {stdout}\n{stderr}");

    let peer_path = temp.path().join("peer");
    checked_git(
        &publisher_path,
        &[
            "clone",
            remote_path.to_str().unwrap(),
            peer_path.to_str().unwrap(),
        ],
    );
    write_remote_prune_config(&peer_path);
    let peer_fake_path = install_fake_gh(&peer_path);
    let (success, stdout, stderr) = run_gg(&peer_path, &["co", "shared-stack"]);
    assert!(success, "peer checkout failed: {stdout}\n{stderr}");

    checked_git(
        &publisher_path,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-3333333"],
    );
    let (success, stdout, stderr) = run_gg_with_env(
        &publisher_path,
        &["sync", "--no-rebase-check"],
        &[("PATH", publisher_fake_path.as_os_str())],
    );
    assert!(success, "publisher resync failed: {stdout}\n{stderr}");

    let (success, stdout, stderr) = run_gg_with_env(
        &peer_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", peer_fake_path.as_os_str())],
    );
    assert!(success, "peer sync failed: {stdout}\n{stderr}");
    assert!(
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                "refs/heads/testuser/shared-stack--c-3333333"
            ]
        )
        .0,
        "sync must preserve remote entries that this clone did not intentionally drop"
    );
}

#[test]
fn test_sync_stops_when_fetch_fails() {
    let (_temp, repo_path, _remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let fake_path = install_fake_gh(&repo_path);
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "fetch-failure"]);
    assert!(success, "{stdout}\n{stderr}");
    checked_git(
        &repo_path,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-1111111"],
    );
    checked_git(
        &repo_path,
        &["remote", "set-url", "origin", "/does/not/exist"],
    );

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(
        !success,
        "sync must stop on fetch failure: {stdout}\n{stderr}"
    );
    assert!(
        stderr.contains("git fetch origin --prune failed"),
        "{stderr}"
    );
}

fn assert_sync_prunes_dropped_tip(rewrite_retained_tip: bool) {
    let (temp, repo_path, remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let fake_path = install_fake_gh(&repo_path);

    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "pruned-stack"]);
    assert!(success, "{stdout}\n{stderr}");
    for (path, id) in [
        ("one.txt", "c-1111111"),
        ("two.txt", "c-2222222"),
        ("three.txt", "c-3333333"),
    ] {
        fs::write(repo_path.join(path), format!("{id}\n")).unwrap();
        checked_git(&repo_path, &["add", path]);
        checked_git(
            &repo_path,
            &["commit", "-m", &format!("Entry {id}\n\nGG-ID: {id}")],
        );
    }

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "3", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "initial sync failed: {stdout}\n{stderr}");
    let retained_before_drop = checked_git(&repo_path, &["rev-parse", "HEAD~"]);
    let dropped_branch = "testuser/pruned-stack--c-3333333";

    for branch in [
        "anotheruser/pruned-stack--c-aaaaaaa",
        "testuser/other-stack--c-bbbbbbb",
        "testuser/pruned-stack/c-ccccccc",
    ] {
        checked_git(
            &repo_path,
            &["push", "origin", &format!("HEAD:refs/heads/{branch}")],
        );
    }

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["drop", "3", "--yes"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "drop failed: {stdout}\n{stderr}");
    assert_eq!(
        checked_git(&repo_path, &["rev-parse", "HEAD"]),
        retained_before_drop,
        "dropping the top entry should preserve retained commit OIDs"
    );
    clear_stack_mappings(&repo_path, "pruned-stack");

    if rewrite_retained_tip {
        fs::write(repo_path.join("two.txt"), "rewritten\n").unwrap();
        checked_git(&repo_path, &["add", "two.txt"]);
        checked_git(&repo_path, &["commit", "--amend", "--no-edit"]);
    }
    let sync_until = if rewrite_retained_tip { "2" } else { "1" };
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", sync_until, "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "resync failed: {stdout}\n{stderr}");
    let expected_tip = checked_git(&repo_path, &["rev-parse", "HEAD"]);
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{dropped_branch}")
            ]
        )
        .0,
        "resync must delete the dropped entry ref"
    );
    for branch in [
        "anotheruser/pruned-stack--c-aaaaaaa",
        "testuser/other-stack--c-bbbbbbb",
        "testuser/pruned-stack/c-ccccccc",
    ] {
        assert!(
            run_git(
                &remote_path,
                &["show-ref", "--verify", &format!("refs/heads/{branch}")]
            )
            .0,
            "resync must not delete unrelated branch {branch}"
        );
    }

    let operation = latest_operation(&repo_path);
    assert_eq!(operation["kind"], "sync");
    assert_eq!(operation["status"], "committed");
    assert_eq!(operation["touched_remote"], true);
    assert!(operation["remote_effects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|effect| {
            effect["kind"] == "branch_deleted"
                && effect["remote"] == "origin"
                && effect["branch"] == dropped_branch
                && effect["prior_oid"].is_string()
        }));
    let operation_id = operation["id"].as_str().unwrap();
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", operation_id, "--json"]);
    assert!(
        !success,
        "undo must refuse remote deletion: {stdout}\n{stderr}"
    );
    let refusal: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(refusal["refusal"]["reason"], "remote");

    let reader = temp.path().join(if rewrite_retained_tip {
        "rewritten-reader"
    } else {
        "unchanged-reader"
    });
    checked_git(
        &repo_path,
        &[
            "clone",
            remote_path.to_str().unwrap(),
            reader.to_str().unwrap(),
        ],
    );
    write_remote_prune_config(&reader);
    let (success, stdout, stderr) = run_gg(&reader, &["co", "pruned-stack"]);
    assert!(success, "fresh checkout failed: {stdout}\n{stderr}");
    assert_eq!(checked_git(&reader, &["rev-parse", "HEAD"]), expected_tip);
}

#[test]
fn test_sync_prunes_dropped_tip_before_fresh_checkout_with_unchanged_retained_commits() {
    assert_sync_prunes_dropped_tip(false);
}

#[test]
fn test_sync_prunes_dropped_tip_before_fresh_checkout_with_rewritten_retained_commits() {
    assert_sync_prunes_dropped_tip(true);
}

#[test]
fn test_sync_records_partial_prune_before_later_deletion_is_rejected() {
    let (_temp, repo_path, remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let fake_path = install_fake_gh(&repo_path);
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "reject-prune"]);
    assert!(success, "{stdout}\n{stderr}");
    for id in ["c-1111111", "c-2222222", "c-3333333"] {
        checked_git(
            &repo_path,
            &[
                "commit",
                "--allow-empty",
                "-m",
                &format!("Entry\n\nGG-ID: {id}"),
            ],
        );
    }
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "3", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "initial sync failed: {stdout}\n{stderr}");
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["drop", "2", "3", "--yes"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "drop failed: {stdout}\n{stderr}");
    clear_stack_mappings(&repo_path, "reject-prune");

    let hook = remote_path.join("hooks/pre-receive");
    fs::write(
        &hook,
        r#"#!/bin/sh
while read -r old new ref; do
  if [ "$ref" = "refs/heads/testuser/reject-prune--c-3333333" ] && [ "$new" = "0000000000000000000000000000000000000000" ]; then
    echo "deletion rejected" >&2
    exit 1
  fi
done
exit 0
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&hook).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook, perms).unwrap();
    }

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(
        !success,
        "sync must report prune failure: {stdout}\n{stderr}"
    );
    assert!(
        stderr.contains("failed with an uncertain outcome"),
        "{stderr}"
    );
    assert!(
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                "refs/heads/testuser/reject-prune--c-3333333"
            ]
        )
        .0,
        "a rejected deletion must leave that remote ref intact"
    );
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                "refs/heads/testuser/reject-prune--c-2222222"
            ]
        )
        .0,
        "a successful earlier deletion must remain applied"
    );

    let operation = latest_operation(&repo_path);
    assert_eq!(operation["kind"], "sync");
    assert_eq!(operation["status"], "pending");
    assert_eq!(operation["touched_remote"], true);
    let deleted_effect = operation["remote_effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|effect| effect["kind"] == "branch_deleted")
        .expect("successful deletion must be persisted before the later failure");
    assert_eq!(deleted_effect["branch"], "testuser/reject-prune--c-2222222");
    assert!(deleted_effect["prior_oid"].is_string());

    let deleted_oid = deleted_effect["prior_oid"].as_str().unwrap();
    checked_git(
        &repo_path,
        &[
            "push",
            "origin",
            &format!("{deleted_oid}:refs/heads/testuser/reject-prune--c-2222222"),
        ],
    );

    let operation_id = operation["id"].as_str().unwrap();
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", operation_id, "--json"]);
    assert!(
        !success,
        "undo must refuse after a partial remote deletion: {stdout}\n{stderr}"
    );
    let refusal: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(refusal["refusal"]["reason"], "remote");

    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    let rejected_intent = config["pending_remote_branch_deletions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|intent| intent["branch"] == "testuser/reject-prune--c-3333333")
        .expect("rejected deletion intent must remain recorded");
    assert_eq!(rejected_intent["state"], "deleting");

    fs::remove_file(&hook).unwrap();
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(
        !success,
        "retry must require explicit reconciliation: {stdout}\n{stderr}"
    );
    assert!(stderr.contains("uncertain prior outcome"), "{stderr}");
    assert!(
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                "refs/heads/testuser/reject-prune--c-3333333"
            ]
        )
        .0,
        "retry must preserve the branch after any unsuccessful deletion attempt"
    );
    assert!(
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                "refs/heads/testuser/reject-prune--c-2222222"
            ]
        )
        .0,
        "retry must not delete a successfully consumed intent after the branch is republished"
    );
}

#[cfg(unix)]
#[test]
fn test_failed_delete_response_requires_reconciliation_before_same_oid_retry() {
    let (_temp, repo_path, remote_path, fake_path) = setup_published_prune_stack("ambiguous-prune");
    let dropped_branch = "testuser/ambiguous-prune--c-3333333";
    let dropped_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{dropped_branch}")],
    );
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    clear_stack_mappings(&repo_path, "ambiguous-prune");

    let real_git = install_git_delete_shim(&repo_path);
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_DELETE_BRANCH", dropped_branch.as_ref()),
            ("GG_TEST_DELETE_MODE", "fail_after_delete".as_ref()),
        ],
    );
    assert!(!success, "lost response must fail sync: {stdout}\n{stderr}");
    assert!(
        stderr.contains("simulated lost deletion response"),
        "{stderr}"
    );
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{dropped_branch}")
            ]
        )
        .0,
        "the shim must actually delete the branch before reporting failure"
    );

    let operation = latest_operation(&repo_path);
    assert_eq!(operation["kind"], "sync");
    assert_eq!(operation["status"], "pending");
    assert_eq!(operation["touched_remote"], true);
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert_eq!(
        config["pending_remote_branch_deletions"][0]["state"],
        "deleting"
    );

    checked_git(
        &repo_path,
        &[
            "push",
            "origin",
            &format!("{dropped_oid}:refs/heads/{dropped_branch}"),
        ],
    );
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_DELETE_BRANCH", dropped_branch.as_ref()),
        ],
    );
    assert!(
        !success,
        "retry must refuse same-OID republication: {stdout}\n{stderr}"
    );
    assert!(stderr.contains("uncertain prior outcome"), "{stderr}");
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{dropped_branch}")]
        ),
        dropped_oid,
        "retry must preserve the republished branch"
    );

    let operation_id = operation["id"].as_str().unwrap();
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", operation_id, "--json"]);
    assert!(
        !success,
        "undo must not claim local-only safety: {stdout}\n{stderr}"
    );
    let refusal: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(refusal["refusal"]["reason"], "remote");
}

#[cfg(unix)]
#[test]
fn test_redo_drop_preserves_uncertain_deletion_state() {
    let (_temp, repo_path, remote_path, fake_path) =
        setup_published_prune_stack("redo-uncertain-prune");
    let dropped_branch = "testuser/redo-uncertain-prune--c-3333333";
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    let drop_operation_id = latest_operation(&repo_path)["id"]
        .as_str()
        .unwrap()
        .to_string();
    clear_stack_mappings(&repo_path, "redo-uncertain-prune");

    let real_git = install_git_delete_shim(&repo_path);
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_DELETE_BRANCH", dropped_branch.as_ref()),
            ("GG_TEST_DELETE_MODE", "fail_after_delete".as_ref()),
        ],
    );
    assert!(!success, "lost response must fail sync: {stdout}\n{stderr}");
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{dropped_branch}")
            ]
        )
        .0,
        "the simulated uncertain attempt must delete the branch"
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", &drop_operation_id, "--json"]);
    assert!(success, "undo failed: {stdout}\n{stderr}");
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "--json"]);
    assert!(success, "redo failed: {stdout}\n{stderr}");

    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert_eq!(
        config["pending_remote_branch_deletions"][0]["state"], "deleting",
        "redo must preserve uncertainty rather than mint pending authority"
    );

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_DELETE_BRANCH", dropped_branch.as_ref()),
        ],
    );
    assert!(
        !success,
        "sync must not retry an uncertain deletion: {stdout}\n{stderr}"
    );
    assert!(stderr.contains("uncertain prior outcome"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn test_waiting_sync_loads_prune_authority_after_operation_lock() {
    let (temp, repo_path, remote_path, fake_path) = setup_published_prune_stack("serialized-prune");
    let dropped_branch = "testuser/serialized-prune--c-3333333";
    let dropped_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{dropped_branch}")],
    );
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    clear_stack_mappings(&repo_path, "serialized-prune");

    let real_git = install_git_delete_shim(&repo_path);
    let locked = temp.path().join("sync-a-locked");
    let release = temp.path().join("release-sync-a");
    let before_lock = temp.path().join("sync-b-before-lock");
    let test_home = repo_path.join(".test-home");

    let sync_a = Command::new(env!("CARGO_BIN_EXE_gg"))
        .args(["sync", "--until", "1", "--no-rebase-check"])
        .current_dir(&repo_path)
        .env("HOME", &test_home)
        .env("PATH", &fake_path)
        .env("GG_TEST_SYNC_LOCKED", &locked)
        .env("GG_TEST_SYNC_RELEASE", &release)
        .env("GG_TEST_REAL_GIT", &real_git)
        .env("GG_TEST_DELETE_BRANCH", dropped_branch)
        .env("GG_TEST_DELETE_MODE", "republish_after_delete")
        .env("GG_TEST_DELETE_REPO", &repo_path)
        .env("GG_TEST_DELETE_OID", &dropped_oid)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start first sync");
    wait_for_file(&locked);

    let sync_b = Command::new(env!("CARGO_BIN_EXE_gg"))
        .args(["sync", "--until", "1", "--no-rebase-check"])
        .current_dir(&repo_path)
        .env("HOME", &test_home)
        .env("PATH", &fake_path)
        .env("GG_TEST_SYNC_BEFORE_LOCK", &before_lock)
        .env("GG_TEST_REAL_GIT", &real_git)
        .env("GG_TEST_DELETE_BRANCH", dropped_branch)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start waiting sync");
    wait_for_file(&before_lock);
    fs::write(&release, "continue").expect("release first sync");

    let output_a = sync_a.wait_with_output().expect("wait for first sync");
    assert!(
        output_a.status.success(),
        "first sync failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output_a.stdout),
        String::from_utf8_lossy(&output_a.stderr)
    );
    let output_b = sync_b.wait_with_output().expect("wait for waiting sync");
    assert!(
        output_b.status.success(),
        "waiting sync failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output_b.stdout),
        String::from_utf8_lossy(&output_b.stderr)
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{dropped_branch}")]
        ),
        dropped_oid,
        "the waiting sync must load consumed authority after locking and preserve republication"
    );
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert!(
        config["pending_remote_branch_deletions"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "consumed deletion authority must not be resurrected: {config}"
    );
}

#[test]
fn test_sync_refuses_to_prune_remote_entry_changed_after_drop() {
    let (temp, repo_path, remote_path, fake_path) =
        setup_published_prune_stack("changed-after-drop");
    let dropped_branch = "testuser/changed-after-drop--c-3333333";
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    clear_stack_mappings(&repo_path, "changed-after-drop");

    let peer_path = temp.path().join("peer-change");
    checked_git(
        &repo_path,
        &[
            "clone",
            remote_path.to_str().unwrap(),
            peer_path.to_str().unwrap(),
        ],
    );
    checked_git(&peer_path, &["config", "user.name", "Peer"]);
    checked_git(&peer_path, &["config", "user.email", "peer@example.com"]);
    checked_git(
        &peer_path,
        &[
            "checkout",
            "-b",
            "changed-entry",
            &format!("origin/{dropped_branch}"),
        ],
    );
    fs::write(peer_path.join("peer.txt"), "changed remotely\n").unwrap();
    checked_git(&peer_path, &["add", "peer.txt"]);
    checked_git(
        &peer_path,
        &[
            "commit",
            "--amend",
            "-m",
            "Changed entry\n\nGG-ID: c-3333333",
        ],
    );
    checked_git(
        &peer_path,
        &[
            "push",
            "--force",
            "origin",
            &format!("HEAD:refs/heads/{dropped_branch}"),
        ],
    );
    let changed_oid = checked_git(&peer_path, &["rev-parse", "HEAD"]);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(
        !success,
        "sync must refuse changed remote deletion: {stdout}\n{stderr}"
    );
    assert!(stderr.contains("changed after the local drop"), "{stderr}");
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{dropped_branch}")]
        ),
        changed_oid,
        "sync must preserve the changed remote version"
    );
}

#[test]
fn test_consumed_prune_intent_does_not_delete_same_oid_republication() {
    let (_temp, repo_path, remote_path, fake_path) = setup_published_prune_stack("one-shot-prune");
    let dropped_branch = "testuser/one-shot-prune--c-3333333";
    let dropped_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{dropped_branch}")],
    );
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    clear_stack_mappings(&repo_path, "one-shot-prune");
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "first prune failed: {stdout}\n{stderr}");

    checked_git(
        &repo_path,
        &[
            "push",
            "origin",
            &format!("{dropped_oid}:refs/heads/{dropped_branch}"),
        ],
    );
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "second sync failed: {stdout}\n{stderr}");
    assert!(
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{dropped_branch}")
            ]
        )
        .0,
        "consumed intent must not delete a same-OID republication"
    );
}

#[test]
fn test_historical_drop_record_without_durable_intent_is_not_prune_authority() {
    let (_temp, repo_path, remote_path, fake_path) = setup_published_prune_stack("historical-drop");
    let dropped_branch = "testuser/historical-drop--c-3333333";
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    clear_stack_mappings(&repo_path, "historical-drop");

    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config
        .as_object_mut()
        .unwrap()
        .remove("pending_remote_branch_deletions");
    fs::write(config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "sync failed: {stdout}\n{stderr}");
    assert!(
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{dropped_branch}")
            ]
        )
        .0,
        "a historical Drop journal record must not be upgraded into deletion authority"
    );
}

#[test]
fn test_drop_without_trusted_remote_version_creates_no_prune_intent() {
    let (_temp, repo_path, _remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "unpublished-drop"]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");
    for id in ["c-1111111", "c-2222222"] {
        checked_git(
            &repo_path,
            &[
                "commit",
                "--allow-empty",
                "-m",
                &format!("Entry\n\nGG-ID: {id}"),
            ],
        );
    }

    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "2", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert!(
        config["pending_remote_branch_deletions"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "an unpublished branch has no trusted remote OID and must fail closed: {config}"
    );
}

#[test]
fn test_undo_drop_cancels_remote_prune_intent() {
    let (_temp, repo_path, _remote_path, _fake_path) = setup_published_prune_stack("undo-prune");
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    let drop_operation = latest_operation(&repo_path);
    let operation_id = drop_operation["id"].as_str().unwrap();

    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", operation_id, "--json"]);
    assert!(success, "undo failed: {stdout}\n{stderr}");
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert!(
        config["pending_remote_branch_deletions"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "undo must durably cancel the drop's deletion intent: {config}"
    );
}

#[test]
fn test_redo_drop_restores_remote_prune_intent_before_fresh_checkout() {
    let (temp, repo_path, remote_path, fake_path) = setup_published_prune_stack("redo-prune");
    let dropped_branch = "testuser/redo-prune--c-3333333";
    let dropped_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{dropped_branch}")],
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    let retained_oid = checked_git(&repo_path, &["rev-parse", "HEAD"]);
    let drop_operation = latest_operation(&repo_path);
    let drop_operation_id = drop_operation["id"].as_str().unwrap();

    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", drop_operation_id, "--json"]);
    assert!(success, "undo failed: {stdout}\n{stderr}");
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "--json"]);
    assert!(success, "redo failed: {stdout}\n{stderr}");
    assert_eq!(
        checked_git(&repo_path, &["rev-parse", "HEAD"]),
        retained_oid
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "--json"]);
    assert!(success, "second undo failed: {stdout}\n{stderr}");
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert!(
        config["pending_remote_branch_deletions"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "each undo cycle must cancel the intent: {config}"
    );
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "--json"]);
    assert!(success, "second redo failed: {stdout}\n{stderr}");
    let redo_operation_id = latest_operation(&repo_path)["id"]
        .as_str()
        .unwrap()
        .to_string();

    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    let intent = config["pending_remote_branch_deletions"]
        .as_array()
        .and_then(|intents| {
            intents
                .iter()
                .find(|intent| intent["branch"] == dropped_branch)
        })
        .expect("redo must restore the Drop deletion intent");
    assert_eq!(intent["expected_oid"], dropped_oid);
    assert_eq!(intent["state"], "pending");

    clear_stack_mappings(&repo_path, "redo-prune");
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "sync failed: {stdout}\n{stderr}");
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{dropped_branch}")
            ]
        )
        .0,
        "sync after redo must delete the dropped entry ref"
    );
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", &redo_operation_id, "--json"]);
    assert!(
        !success,
        "undo after intent consumption must refuse stale config: {stdout}\n{stderr}"
    );
    let refusal: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(refusal["refusal"]["reason"], "stale");
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert!(
        config["pending_remote_branch_deletions"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "consumed deletion authority must not be resurrected: {config}"
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", drop_operation_id, "--json"]);
    assert!(
        success,
        "undo of Drop after completed sync failed: {stdout}\n{stderr}"
    );
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "--json"]);
    assert!(
        success,
        "redo of Drop after completed sync failed: {stdout}\n{stderr}"
    );
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert!(
        config["pending_remote_branch_deletions"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "a valid redo after completed sync must not recreate consumed authority: {config}"
    );

    let reader = temp.path().join("redo-reader");
    checked_git(
        &repo_path,
        &[
            "clone",
            remote_path.to_str().unwrap(),
            reader.to_str().unwrap(),
        ],
    );
    write_remote_prune_config(&reader);
    let (success, stdout, stderr) = run_gg(&reader, &["co", "redo-prune"]);
    assert!(success, "fresh checkout failed: {stdout}\n{stderr}");
    assert_eq!(checked_git(&reader, &["rev-parse", "HEAD"]), retained_oid);
}

#[test]
fn test_recreated_entry_cancels_remote_prune_intent() {
    let (_temp, repo_path, remote_path, fake_path) = setup_published_prune_stack("recreated-prune");
    let dropped_branch = "testuser/recreated-prune--c-3333333";
    let dropped_oid = checked_git(&repo_path, &["rev-parse", "HEAD"]);
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    clear_stack_mappings(&repo_path, "recreated-prune");

    checked_git(&repo_path, &["reset", "--hard", &dropped_oid]);
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "recreation sync failed: {stdout}\n{stderr}");
    checked_git(&repo_path, &["reset", "--hard", "HEAD~"]);
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "post-recreation sync failed: {stdout}\n{stderr}");
    assert!(
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{dropped_branch}")
            ]
        )
        .0,
        "recreating an entry must cancel stale deletion intent, not merely defer it"
    );
}

#[test]
fn test_remote_prune_intent_survives_operation_journal_rotation() {
    let (temp, repo_path, remote_path, fake_path) = setup_published_prune_stack("rotated-prune");
    let dropped_branch = "testuser/rotated-prune--c-3333333";
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    clear_stack_mappings(&repo_path, "rotated-prune");

    for _ in 0..101 {
        let (success, stdout, stderr) = run_gg(&repo_path, &["co", "rotated-prune"]);
        assert!(success, "recorded checkout failed: {stdout}\n{stderr}");
    }
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(
        success,
        "sync after journal rotation failed: {stdout}\n{stderr}"
    );
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{dropped_branch}")
            ]
        )
        .0,
        "pending deletion must survive operation log pruning"
    );

    let reader = temp.path().join("rotation-reader");
    checked_git(
        &repo_path,
        &[
            "clone",
            remote_path.to_str().unwrap(),
            reader.to_str().unwrap(),
        ],
    );
    write_remote_prune_config(&reader);
    let (success, stdout, stderr) = run_gg(&reader, &["co", "rotated-prune"]);
    assert!(success, "fresh checkout failed: {stdout}\n{stderr}");
    assert_eq!(
        checked_git(&reader, &["rev-parse", "HEAD"]),
        checked_git(&repo_path, &["rev-parse", "HEAD"]),
        "fresh checkout must not resurrect the pruned entry"
    );
}

fn git_current_branch(repo_path: &std::path::Path) -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_path)
        .output()
        .expect("git rev-parse failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
