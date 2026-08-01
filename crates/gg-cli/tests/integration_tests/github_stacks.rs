use crate::helpers::{create_test_repo_with_remote, run_gg, run_gg_with_env, run_git};

use serde_json::Value;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

struct GithubStackFixture {
    _temp_dir: tempfile::TempDir,
    repo_path: PathBuf,
    fake_log: PathBuf,
    path_env: OsString,
}

impl GithubStackFixture {
    fn env(&self) -> Vec<(&str, &OsStr)> {
        vec![
            ("PATH", self.path_env.as_os_str()),
            ("GG_FAKE_GH_LOG", self.fake_log.as_os_str()),
        ]
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.fake_log).unwrap_or_default()
    }

    fn link_lines(&self) -> Vec<String> {
        self.log()
            .lines()
            .filter(|line| line.starts_with("stack link"))
            .map(ToString::to_string)
            .collect()
    }

    fn set_stack_response(&self, name: &str, json: &str) -> PathBuf {
        let path = self.repo_path.join(name);
        fs::write(&path, json).expect("failed to write fake stack response");
        path
    }

    fn set_link_failure(&self, message: &str) -> PathBuf {
        let path = self.repo_path.join("fake-link-failure");
        fs::write(&path, message).expect("failed to write fake link failure");
        path
    }
}

fn setup_fixture(mode: &str) -> GithubStackFixture {
    let (temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("failed to create gg dir");

    fs::write(
        gg_dir.join("config.json"),
        format!(
            r#"{{
  "defaults": {{
    "branch_username": "testuser",
    "provider": "github",
    "base": "main",
    "sync_behind_threshold": 0,
    "github": {{ "stacks_integration": "{mode}" }}
  }}
}}"#
        ),
    )
    .expect("failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "native-stack"]);
    assert!(success, "failed to create stack: {stderr}");

    fs::write(repo_path.join("one.txt"), "one\n").expect("failed to write one");
    run_git(&repo_path, &["add", "one.txt"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Entry one\n\nGG-ID: c-1111111"],
    );

    fs::write(repo_path.join("two.txt"), "two\n").expect("failed to write two");
    run_git(&repo_path, &["add", "two.txt"]);
    run_git(
        &repo_path,
        &[
            "commit",
            "-m",
            "Entry two\n\nGG-ID: c-2222222\nGG-Parent: c-1111111",
        ],
    );

    fs::write(
        gg_dir.join("config.json"),
        format!(
            r#"{{
  "defaults": {{
    "branch_username": "testuser",
    "provider": "github",
    "base": "main",
    "sync_behind_threshold": 0,
    "github": {{ "stacks_integration": "{mode}" }}
  }},
  "stacks": {{
    "native-stack": {{
      "base": "main",
      "mrs": {{
        "c-1111111": 41,
        "c-2222222": 42
      }}
    }}
  }}
}}"#
        ),
    )
    .expect("failed to write mapped config");

    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("failed to create fake-bin");
    let fake_log = repo_path.join("fake-gh.log");
    fs::write(&fake_log, "").expect("failed to create fake log");
    write_fake_gh(&fake_bin.join("gh"));

    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut path_env = OsString::from(fake_bin.as_os_str());
    path_env.push(":");
    path_env.push(old_path);

    GithubStackFixture {
        _temp_dir: temp_dir,
        repo_path,
        fake_log,
        path_env,
    }
}

fn write_fake_gh(path: &Path) {
    fs::write(
        path,
        r#"#!/bin/sh
set -eu
echo "$@" >> "$GG_FAKE_GH_LOG"

if [ "$1" = "--version" ]; then
  echo "gh version 2.97.0"
  exit 0
fi

if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  if [ "$3" = "42" ] && [ -n "${GG_FAKE_PR_VIEW_42_FAIL:-}" ]; then
    echo "failed to view PR #42" >&2
    exit 1
  fi
  case "$3" in
    41)
      echo '{"number":41,"title":"Entry one","state":"OPEN","url":"https://github.com/test/repo/pull/41","headRefName":"testuser/native-stack--c-1111111","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
      exit 0
      ;;
    42)
      if [ -n "${GG_FAKE_PR_VIEW_42_OLD_HEAD:-}" ]; then
        echo '{"number":42,"title":"Entry two","state":"OPEN","url":"https://github.com/test/repo/pull/42","headRefName":"testuser/old-stack--c-2222222","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
        exit 0
      fi
      echo '{"number":42,"title":"Entry two","state":"OPEN","url":"https://github.com/test/repo/pull/42","headRefName":"testuser/native-stack--c-2222222","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
      exit 0
      ;;
  esac
fi

if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  if [ -n "${GG_FAKE_PR_CREATE_FAIL:-}" ]; then
    echo "failed to create replacement PR" >&2
    exit 1
  fi
  echo "https://github.com/test/repo/pull/99"
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "edit" ]; then
  if [ -n "${GG_FAKE_PR_EDIT_FAIL:-}" ]; then
    echo "failed to update PR base" >&2
    exit 1
  fi
  exit 0
fi

if [ "$1" = "api" ] && printf '%s' "$2" | grep -q '/issues/'; then
  echo '[]'
  exit 0
fi

if [ "$1" = "api" ] && printf '%s' "$2" | grep -q '/stacks?pull_request='; then
  if [ -n "${GG_FAKE_STACK_404:-}" ]; then
    echo 'gh: HTTP 404: Not Found' >&2
    exit 1
  fi
  if [ "$2" = "repos/{owner}/{repo}/stacks?pull_request=41" ] && [ -n "${GG_FAKE_STACK_AFTER_41:-}" ] && [ -f "${GG_FAKE_STACK_AFTER_41}" ] && [ -n "${GG_FAKE_LINKED_FILE:-}" ] && [ -f "${GG_FAKE_LINKED_FILE}" ]; then
    cat "${GG_FAKE_STACK_AFTER_41}"
    exit 0
  fi
  if [ -n "${GG_FAKE_STACK_RESPONSE:-}" ] && [ -f "${GG_FAKE_STACK_RESPONSE}" ]; then
    cat "${GG_FAKE_STACK_RESPONSE}"
    exit 0
  fi
  echo '[]'
  exit 0
fi

if [ "$1" = "stack" ] && [ "$2" = "--version" ]; then
  if [ -n "${GG_FAKE_STACK_VERSION_EXIT:-}" ]; then
    exit "$GG_FAKE_STACK_VERSION_EXIT"
  fi
  echo "gh stack version ${GG_FAKE_STACK_VERSION:-0.1.0}"
  exit 0
fi

if [ "$1" = "stack" ] && [ "$2" = "link" ]; then
  if [ -n "${GG_FAKE_LINK_FAILURE:-}" ]; then
    cat "${GG_FAKE_LINK_FAILURE}" >&2
    exit 1
  fi
  if [ -n "${GG_FAKE_LINKED_FILE:-}" ]; then
    echo linked > "${GG_FAKE_LINKED_FILE}"
  fi
  exit 0
fi

echo "unexpected gh invocation: $@" >&2
exit 1
"#,
    )
    .expect("failed to write fake gh");

    #[cfg(unix)]
    {
        let mut perms = fs::metadata(path)
            .expect("failed to stat fake gh")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("failed to chmod fake gh");
    }
}

fn stack_created_json() -> &'static str {
    r#"[{"number":7,"pull_requests":[
{"number":41,"state":"open","draft":false,"merged_at":null},
{"number":42,"state":"open","draft":false,"merged_at":null}
]}]"#
}

fn stack_prefix_json() -> &'static str {
    r#"[{"number":7,"pull_requests":[
{"number":40,"state":"closed","draft":false,"merged_at":"2026-07-01T00:00:00Z"},
{"number":41,"state":"open","draft":false,"merged_at":null}
]}]"#
}

fn stack_diverged_json() -> &'static str {
    r#"[{"number":7,"pull_requests":[
{"number":42,"state":"open","draft":false,"merged_at":null},
{"number":41,"state":"open","draft":false,"merged_at":null}
]}]"#
}

fn run_sync_json(
    fixture: &GithubStackFixture,
    extra_env: &[(&str, &OsStr)],
) -> (bool, String, String) {
    let mut env = fixture.env();
    env.extend_from_slice(extra_env);
    run_gg_with_env(
        &fixture.repo_path,
        &["sync", "--json", "--no-rebase-check"],
        &env,
    )
}

#[test]
fn sync_json_creates_native_stack_with_exact_pr_order() {
    let fixture = setup_fixture("auto");
    let after = fixture.set_stack_response("stack-after.json", stack_created_json());
    let linked = fixture.repo_path.join("fake-linked");
    let (success, stdout, stderr) = run_sync_json(
        &fixture,
        &[
            ("GG_FAKE_STACK_AFTER_41", after.as_os_str()),
            ("GG_FAKE_LINKED_FILE", linked.as_os_str()),
        ],
    );
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    assert!(stderr.trim().is_empty(), "stderr should be empty: {stderr}");

    let value: Value = serde_json::from_str(&stdout).expect("sync should emit JSON");
    let stack = &value["sync"]["github_stack"];
    assert_eq!(stack["action"], "created");
    assert_eq!(stack["stack_number"], 7);
    assert_eq!(stack["pr_numbers"], serde_json::json!([41, 42]));
    assert_eq!(fixture.link_lines(), vec!["stack link --base main 41 42"]);
}

#[test]
fn sync_jsonl_appends_after_merged_prefix_and_repeats_result_in_summary() {
    let fixture = setup_fixture("auto");
    let prefix = fixture.set_stack_response("stack-prefix.json", stack_prefix_json());
    let mut env = fixture.env();
    env.push(("GG_FAKE_STACK_RESPONSE", prefix.as_os_str()));
    let (success, stdout, stderr) = run_gg_with_env(
        &fixture.repo_path,
        &["sync", "--jsonl", "--no-rebase-check"],
        &env,
    );
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    assert!(stderr.trim().is_empty(), "stderr should be empty: {stderr}");

    let events: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("every JSONL line must parse"))
        .collect();
    let github_event_index = events
        .iter()
        .position(|event| event["event"] == "github_stack")
        .expect("github_stack event should be emitted");
    let summary_index = events
        .iter()
        .position(|event| event["event"] == "summary")
        .expect("summary should be emitted");
    assert!(github_event_index < summary_index);
    assert_eq!(events[github_event_index]["action"], "appended");
    assert_eq!(events[github_event_index]["stack_number"], 7);
    assert_eq!(
        events[summary_index]["github_stack"]["action"],
        events[github_event_index]["action"]
    );
    assert_eq!(
        events[summary_index]["github_stack"]["stack_number"],
        events[github_event_index]["stack_number"]
    );
    assert_eq!(
        events[summary_index]["github_stack"]["pr_numbers"],
        events[github_event_index]["pr_numbers"]
    );
    assert_eq!(fixture.link_lines(), vec!["stack link 7 42"]);
}

#[test]
fn sync_until_reports_partial_skip_without_stack_command() {
    let fixture = setup_fixture("auto");
    let env = fixture.env();
    let (success, stdout, stderr) = run_gg_with_env(
        &fixture.repo_path,
        &["sync", "--json", "--until", "1", "--no-rebase-check"],
        &env,
    );
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("sync should emit JSON");
    assert_eq!(value["sync"]["github_stack"]["reason"], "partial_sync");
    let log = fixture.log();
    assert!(!log.contains("stack --version"));
    assert!(!log.contains("/stacks"));
}

#[test]
fn sync_off_reports_disabled_without_stack_command() {
    let fixture = setup_fixture("off");
    let (success, stdout, stderr) = run_sync_json(&fixture, &[]);
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("sync should emit JSON");
    assert_eq!(value["sync"]["github_stack"]["reason"], "disabled");
    assert!(!fixture.log().contains("stack --version"));
}

#[test]
fn sync_unresolved_pr_state_skips_without_stack_command() {
    let fixture = setup_fixture("auto");
    let (success, stdout, stderr) =
        run_sync_json(&fixture, &[("GG_FAKE_PR_VIEW_42_FAIL", OsStr::new("1"))]);
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("sync should emit JSON");
    assert_eq!(value["sync"]["github_stack"]["action"], "skipped");
    assert_eq!(value["sync"]["github_stack"]["reason"], "unresolved_prs");
    let log = fixture.log();
    assert!(!log.contains("stack --version"));
    assert!(!log.contains("/stacks"));
}

#[test]
fn sync_base_update_failure_skips_without_stack_command() {
    let fixture = setup_fixture("auto");
    let (success, stdout, stderr) =
        run_sync_json(&fixture, &[("GG_FAKE_PR_EDIT_FAIL", OsStr::new("1"))]);
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("sync should emit JSON");
    assert_eq!(value["sync"]["github_stack"]["action"], "skipped");
    assert_eq!(value["sync"]["github_stack"]["reason"], "unresolved_prs");
    let log = fixture.log();
    assert!(log.contains("pr edit 41 --base main"));
    assert!(!log.contains("stack --version"));
    assert!(!log.contains("/stacks"));
}

#[test]
fn sync_replacement_create_failure_skips_without_stack_command() {
    let fixture = setup_fixture("auto");
    let (success, stdout, stderr) = run_sync_json(
        &fixture,
        &[
            ("GG_FAKE_PR_VIEW_42_OLD_HEAD", OsStr::new("1")),
            ("GG_FAKE_PR_CREATE_FAIL", OsStr::new("1")),
        ],
    );
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("sync should emit JSON");
    assert_eq!(value["sync"]["github_stack"]["action"], "skipped");
    assert_eq!(value["sync"]["github_stack"]["reason"], "unresolved_prs");
    let log = fixture.log();
    assert!(log.contains("pr create --head testuser/native-stack--c-2222222"));
    assert!(!log.contains("stack --version"));
    assert!(!log.contains("/stacks"));
}

#[test]
fn sync_auto_missing_extension_is_silent_and_non_fatal() {
    let fixture = setup_fixture("auto");
    let (success, stdout, stderr) =
        run_sync_json(&fixture, &[("GG_FAKE_STACK_VERSION_EXIT", OsStr::new("1"))]);
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    assert!(stderr.trim().is_empty(), "stderr should be empty: {stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("sync should emit JSON");
    assert_eq!(value["sync"]["github_stack"]["action"], "skipped");
    assert_eq!(value["sync"]["github_stack"]["reason"], "missing_extension");
    assert_eq!(value["sync"]["warnings"], serde_json::json!([]));
}

#[test]
fn sync_force_missing_extension_is_warning_and_non_fatal() {
    let fixture = setup_fixture("force");
    let (success, stdout, stderr) =
        run_sync_json(&fixture, &[("GG_FAKE_STACK_VERSION_EXIT", OsStr::new("1"))]);
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("sync should emit JSON");
    assert_eq!(value["sync"]["github_stack"]["action"], "warning");
    assert_eq!(value["sync"]["github_stack"]["reason"], "missing_extension");
    assert!(value["sync"]["warnings"].as_array().unwrap().len() == 1);
}

#[test]
fn sync_divergence_warns_without_link() {
    let fixture = setup_fixture("auto");
    let diverged = fixture.set_stack_response("stack-diverged.json", stack_diverged_json());
    let (success, stdout, stderr) = run_sync_json(
        &fixture,
        &[("GG_FAKE_STACK_RESPONSE", diverged.as_os_str())],
    );
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("sync should emit JSON");
    assert_eq!(value["sync"]["github_stack"]["action"], "warning");
    assert_eq!(value["sync"]["github_stack"]["reason"], "diverged");
    assert!(!fixture.log().contains("stack link"));
}

#[test]
fn sync_link_failure_warns_without_corrupting_json() {
    let fixture = setup_fixture("auto");
    let link_failure = fixture.set_link_failure("link failed");
    let (success, stdout, stderr) = run_sync_json(
        &fixture,
        &[("GG_FAKE_LINK_FAILURE", link_failure.as_os_str())],
    );
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("sync should remain one JSON document");
    assert_eq!(value["sync"]["github_stack"]["action"], "warning");
    assert_eq!(value["sync"]["github_stack"]["reason"], "backend_failed");
    assert_eq!(value["sync"]["github_stack"]["message"], "link failed");

    let (success, undo_stdout, undo_stderr) =
        run_gg(&fixture.repo_path, &["undo", "--list", "--json"]);
    assert!(
        success,
        "undo list failed\nstdout:{undo_stdout}\nstderr:{undo_stderr}"
    );
    let listed: Value = serde_json::from_str(&undo_stdout).expect("undo list should emit JSON");
    let sync_record = listed["operations"]
        .as_array()
        .expect("operations should be an array")
        .iter()
        .find(|record| record["kind"] == "sync")
        .expect("sync operation should be recorded");
    assert_eq!(sync_record["touched_remote"], true);
    assert_eq!(sync_record["is_undoable"], false);
}

#[test]
fn sync_gitlab_never_invokes_or_serializes_github_stacks() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"gitlab","base":"main","sync_behind_threshold":0}}"#,
    )
    .expect("failed to write config");
    let (success, _, stderr) = run_gg(&repo_path, &["co", "gitlab-stack"]);
    assert!(success, "failed to create stack: {stderr}");
    fs::write(repo_path.join("one.txt"), "one\n").expect("failed to write one");
    run_git(&repo_path, &["add", "one.txt"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Entry one\n\nGG-ID: c-1111111"],
    );

    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("failed to create fake-bin");
    fs::write(
        fake_bin.join("glab"),
        r#"#!/bin/sh
set -eu
if [ "$1" = "--version" ]; then echo "glab version 1.0.0"; exit 0; fi
if [ "$1" = "auth" ] && [ "$2" = "status" ]; then exit 0; fi
if [ "$1" = "mr" ] && [ "$2" = "create" ]; then echo "https://gitlab.com/test/repo/-/merge_requests/51"; exit 0; fi
echo "unexpected glab invocation: $@" >&2
exit 1
"#,
    )
    .expect("failed to write fake glab");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(fake_bin.join("glab"))
            .expect("failed to stat fake glab")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(fake_bin.join("glab"), perms).expect("failed to chmod fake glab");
    }
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = OsString::from(fake_bin.as_os_str());
    new_path.push(":");
    new_path.push(old_path);
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--json", "--no-rebase-check"],
        &[("PATH", new_path.as_os_str())],
    );
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("sync should emit JSON");
    assert!(value["sync"]["github_stack"].is_null());
}

#[test]
fn sync_gitlab_jsonl_omits_github_stack_field_and_event() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"gitlab","base":"main","sync_behind_threshold":0}}"#,
    )
    .expect("failed to write config");
    let (success, _, stderr) = run_gg(&repo_path, &["co", "gitlab-jsonl-stack"]);
    assert!(success, "failed to create stack: {stderr}");
    fs::write(repo_path.join("one.txt"), "one\n").expect("failed to write one");
    run_git(&repo_path, &["add", "one.txt"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Entry one\n\nGG-ID: c-1111111"],
    );

    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("failed to create fake-bin");
    fs::write(
        fake_bin.join("glab"),
        r#"#!/bin/sh
set -eu
if [ "$1" = "--version" ]; then echo "glab version 1.0.0"; exit 0; fi
if [ "$1" = "auth" ] && [ "$2" = "status" ]; then exit 0; fi
if [ "$1" = "mr" ] && [ "$2" = "create" ]; then echo "https://gitlab.com/test/repo/-/merge_requests/51"; exit 0; fi
echo "unexpected glab invocation: $@" >&2
exit 1
"#,
    )
    .expect("failed to write fake glab");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(fake_bin.join("glab"))
            .expect("failed to stat fake glab")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(fake_bin.join("glab"), perms).expect("failed to chmod fake glab");
    }
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = OsString::from(fake_bin.as_os_str());
    new_path.push(":");
    new_path.push(old_path);
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--jsonl", "--no-rebase-check"],
        &[("PATH", new_path.as_os_str())],
    );
    assert!(success, "sync failed\nstdout:{stdout}\nstderr:{stderr}");
    let events: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("every JSONL line should parse"))
        .collect();
    assert!(
        !events.iter().any(|event| event["event"] == "github_stack"),
        "GitLab JSONL must not emit github_stack events"
    );
    let summary = events
        .iter()
        .find(|event| event["event"] == "summary")
        .expect("summary should be emitted");
    assert!(
        summary.get("github_stack").is_none(),
        "GitLab JSONL summary must omit github_stack"
    );
}
