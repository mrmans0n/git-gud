use crate::helpers::{create_test_repo, create_test_repo_with_remote, run_gg, run_git};

use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
fn write_waiting_fake_gh(path: &Path) {
    fs::write(
        path,
        r#"#!/bin/sh
set -eu

if [ "$1" = "--version" ]; then
  echo "gh version 2.97.0"
  exit 0
fi

if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  calls=0
  if [ -f "$GG_FAKE_PR_VIEW_CALLS" ]; then calls=$(cat "$GG_FAKE_PR_VIEW_CALLS"); fi
  calls=$((calls + 1))
  echo "$calls" > "$GG_FAKE_PR_VIEW_CALLS"
  if [ -f "$GG_FAKE_CLEAN_PROVIDER_FAIL" ] && [ "$calls" -gt 1 ]; then
    echo "provider unavailable" >&2
    exit 1
  fi
  case "$*" in
    *statusCheckRollup*)
      touch "$GG_FAKE_POLLED"
      calls=0
      if [ -f "$GG_FAKE_CI_CALLS" ]; then calls=$(cat "$GG_FAKE_CI_CALLS"); fi
      calls=$((calls + 1))
      echo "$calls" > "$GG_FAKE_CI_CALLS"
      if [ -f "$GG_FAKE_REGRESS" ] && [ "$calls" -eq 3 ]; then
        echo PENDING
      elif [ -f "$GG_FAKE_READY" ]; then
        echo SUCCESS
      else
        echo PENDING
      fi
      exit 0
      ;;
    *"--jq .reviewDecision"*)
      echo APPROVED
      exit 0
      ;;
  esac

  if [ -f "$GG_FAKE_MERGED" ]; then state=MERGED; else state=OPEN; fi
  printf '{"number":41,"title":"Land entry","state":"%s","url":"https://github.com/test/repo/pull/41","headRefName":"testuser/land-wait--c-1111111","isDraft":false,"mergeable":"MERGEABLE","reviews":[],"reviewDecision":"APPROVED"}\n' "$state"
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
  touch "$GG_FAKE_MERGED"
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "edit" ]; then
  if [ -f "$GG_FAKE_RETARGET_FAIL" ]; then
    echo "retarget failed" >&2
    exit 1
  fi
  exit 0
fi

echo "unexpected gh invocation: $*" >&2
exit 1
"#,
    )
    .expect("write fake gh");
    let mut permissions = fs::metadata(path).expect("stat fake gh").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make fake gh executable");
}

#[cfg(unix)]
fn write_fake_land_glab(path: &Path) {
    fs::write(
        path,
        r#"#!/bin/sh
set -eu

if [ "$1" = "--version" ]; then
  echo "glab version 1.0.0"
  exit 0
fi

if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  if [ -f "$GG_FAKE_AUTH_NETWORK_FAIL" ]; then
    echo "Could not resolve host: gitlab.com" >&2
    exit 1
  fi
  exit 0
fi

if [ "$1" = "api" ] && [ "$2" = "projects/:id" ]; then
  if [ -f "$GG_FAKE_MERGE_TRAINS" ]; then
    echo '{"merge_trains_enabled":true}'
  else
    echo '{"merge_trains_enabled":false}'
  fi
  exit 0
fi

if [ "$1" = "api" ] && [ "$2" = "-X" ] && [ "$3" = "POST" ]; then
  touch "$GG_FAKE_QUEUED"
  exit 0
fi

if [ "$1" = "api" ]; then
  case "$2" in
    projects/:id/merge_requests/*/approvals)
      echo '{"approved":true}'
      exit 0
      ;;
    projects/:id/merge_trains/*scope=active*)
      if [ ! -f "$GG_FAKE_QUEUED" ]; then
        echo '[]'
        exit 0
      fi
      touch "$GG_FAKE_POLLED"
      if [ -f "$GG_FAKE_MERGED" ]; then
        echo '[]'
      else
        echo '[{"merge_request":{"iid":41},"status":"fresh","pipeline":{"status":"success"}}]'
      fi
      exit 0
      ;;
    projects/:id/merge_trains/*scope=complete*)
      if [ -f "$GG_FAKE_MERGED" ]; then
        echo '[{"merge_request":{"iid":41},"status":"merged","pipeline":{"status":"success"}}]'
      else
        echo '[]'
      fi
      exit 0
      ;;
  esac
fi

if [ "$1" = "mr" ] && [ "$2" = "view" ]; then
  if [ -f "$GG_FAKE_MERGED" ]; then state=merged; else state=opened; fi
  printf '{"iid":41,"title":"Land entry","state":"%s","web_url":"https://gitlab.example/test/repo/-/merge_requests/41","source_branch":"testuser/land-wait/c-1111111","draft":false,"work_in_progress":false,"detailed_merge_status":"mergeable","head_pipeline":{"status":"success"}}\n' "$state"
  exit 0
fi

if [ "$1" = "mr" ] && [ "$2" = "merge" ]; then
  touch "$GG_FAKE_MERGED"
  touch "$GG_FAKE_MERGED.glab"
  exit 0
fi

echo "unexpected glab invocation: $*" >&2
exit 1
"#,
    )
    .expect("write fake glab");
    let mut permissions = fs::metadata(path).expect("stat fake glab").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make fake glab executable");
}

#[cfg(unix)]
struct WaitingLandFixture {
    _temp_dir: tempfile::TempDir,
    repo_path: std::path::PathBuf,
    other_worktree: std::path::PathBuf,
    path: OsString,
    polled: std::path::PathBuf,
    ready: std::path::PathBuf,
    merged: std::path::PathBuf,
    ci_calls: std::path::PathBuf,
    regress: std::path::PathBuf,
    merge_trains: std::path::PathBuf,
    auth_network_fail: std::path::PathBuf,
    queued: std::path::PathBuf,
    retarget_fail: std::path::PathBuf,
    clean_provider_fail: std::path::PathBuf,
    pr_view_calls: std::path::PathBuf,
    test_home: std::path::PathBuf,
}

#[cfg(unix)]
impl WaitingLandFixture {
    fn new() -> Self {
        let (temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();
        let gg_dir = repo_path.join(".git/gg");
        fs::create_dir_all(&gg_dir).expect("create gg dir");
        fs::write(
            gg_dir.join("config.json"),
            r#"{
  "defaults": {"branch_username":"testuser","provider":"github","base":"main"},
  "stacks": {"land-wait":{"base":"main","mrs":{"c-1111111":41}}}
}"#,
        )
        .expect("write config");

        run_git(&repo_path, &["checkout", "-b", "testuser/land-wait"]);
        fs::write(repo_path.join("land.txt"), "land\n").expect("write land entry");
        run_git(&repo_path, &["add", "land.txt"]);
        run_git(
            &repo_path,
            &["commit", "-m", "Land entry\n\nGG-ID: c-1111111"],
        );

        let other_worktree = repo_path.parent().unwrap().join("other-worktree");
        let worktree_output = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "testuser/other",
                other_worktree.to_str().unwrap(),
                "main",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("create other worktree");
        assert!(worktree_output.status.success());

        let fake_bin = repo_path.join("fake-bin-land-wait");
        fs::create_dir_all(&fake_bin).expect("create fake bin");
        write_waiting_fake_gh(&fake_bin.join("gh"));
        let mut path = OsString::from(fake_bin.as_os_str());
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());

        let polled = repo_path.join("fake-polled");
        let ready = repo_path.join("fake-ready");
        let merged = repo_path.join("fake-merged");
        let ci_calls = repo_path.join("fake-ci-calls");
        let regress = repo_path.join("fake-regress");
        let merge_trains = repo_path.join("fake-merge-trains");
        let auth_network_fail = repo_path.join("fake-auth-network-fail");
        let queued = repo_path.join("fake-queued");
        let retarget_fail = repo_path.join("fake-retarget-fail");
        let clean_provider_fail = repo_path.join("fake-clean-provider-fail");
        let pr_view_calls = repo_path.join("fake-pr-view-calls");
        let test_home = repo_path.join(".test-home");
        fs::create_dir_all(&test_home).expect("create test home");

        Self {
            _temp_dir: temp_dir,
            repo_path,
            other_worktree,
            path,
            polled,
            ready,
            merged,
            ci_calls,
            regress,
            merge_trains,
            auth_network_fail,
            queued,
            retarget_fail,
            clean_provider_fail,
            pr_view_calls,
            test_home,
        }
    }

    fn start_land(&self) -> std::process::Child {
        self.start_land_with_args(&["land", "--all", "--wait", "--no-clean"])
    }

    fn start_land_with_args(&self, args: &[&str]) -> std::process::Child {
        Command::new(env!("CARGO_BIN_EXE_gg"))
            .args(args)
            .current_dir(&self.repo_path)
            .env("HOME", &self.test_home)
            .env("PATH", &self.path)
            .env("GG_FAKE_POLLED", &self.polled)
            .env("GG_FAKE_READY", &self.ready)
            .env("GG_FAKE_MERGED", &self.merged)
            .env("GG_FAKE_CI_CALLS", &self.ci_calls)
            .env("GG_FAKE_REGRESS", &self.regress)
            .env("GG_FAKE_MERGE_TRAINS", &self.merge_trains)
            .env("GG_FAKE_AUTH_NETWORK_FAIL", &self.auth_network_fail)
            .env("GG_FAKE_QUEUED", &self.queued)
            .env("GG_FAKE_RETARGET_FAIL", &self.retarget_fail)
            .env("GG_FAKE_CLEAN_PROVIDER_FAIL", &self.clean_provider_fail)
            .env("GG_FAKE_PR_VIEW_CALLS", &self.pr_view_calls)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start waiting land")
    }

    fn wait_until_polling(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.polled.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(self.polled.exists(), "land never started polling CI");
    }

    fn wait_until_queued(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.queued.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(self.queued.exists(), "land never queued the MR");
    }

    fn release_ci(&self) {
        fs::write(&self.ready, "ready\n").expect("release fake CI");
    }

    fn add_second_entry(&self) {
        fs::write(self.repo_path.join("second.txt"), "second\n").expect("write second entry");
        run_git(&self.repo_path, &["add", "second.txt"]);
        run_git(
            &self.repo_path,
            &["commit", "-m", "Second entry\n\nGG-ID: c-2222222"],
        );

        let config_path = self.repo_path.join(".git/gg/config.json");
        let mut config: Value =
            serde_json::from_slice(&fs::read(&config_path).expect("read config"))
                .expect("parse config");
        config["stacks"]["land-wait"]["mrs"]["c-2222222"] = Value::from(42);
        fs::write(
            &config_path,
            serde_json::to_vec_pretty(&config).expect("serialize config"),
        )
        .expect("write config");
    }

    fn use_gitlab_provider(&self) {
        let config_path = self.repo_path.join(".git/gg/config.json");
        let mut config: Value =
            serde_json::from_slice(&fs::read(&config_path).expect("read config"))
                .expect("parse config");
        config["defaults"]["provider"] = Value::String("gitlab".to_string());
        fs::write(
            &config_path,
            serde_json::to_vec_pretty(&config).expect("serialize config"),
        )
        .expect("write config");
        write_fake_land_glab(&self.repo_path.join("fake-bin-land-wait").join("glab"));
    }

    fn enable_gitlab_merge_trains(&self) {
        fs::write(&self.merge_trains, "enabled\n").expect("enable fake merge trains");
    }

    fn fail_auth_with_network_error(&self) {
        fs::write(&self.auth_network_fail, "fail\n").expect("enable fake auth failure");
    }

    fn fail_retargeting(&self) {
        fs::write(&self.retarget_fail, "fail\n").expect("enable fake retarget failure");
    }

    fn fail_cleanup_provider_after_first_view(&self) {
        fs::write(&self.clean_provider_fail, "fail\n")
            .expect("enable fake cleanup provider failure");
    }

    fn fail_downstream_push(&self) {
        fs::write(
            self.repo_path.join("fake-bin-land-wait").join("git"),
            r#"#!/bin/sh
if [ "$1" = "push" ] && [ "$2" = "--force-with-lease" ]; then
  echo "force-with-lease rejected" >&2
  exit 1
fi
exec /usr/bin/git "$@"
"#,
        )
        .expect("write fake git");
        let path = self.repo_path.join("fake-bin-land-wait").join("git");
        let mut permissions = fs::metadata(&path).expect("stat fake git").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("make fake git executable");
    }
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_streams_wait_entry_and_summary() {
    let fixture = WaitingLandFixture::new();
    let mut land = fixture.start_land_with_args(&["land", "--wait", "--no-clean", "--jsonl"]);
    let stdout = land.stdout.take().expect("capture land stdout");
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut events = Vec::new();
        loop {
            let mut line = String::new();
            assert!(
                reader.read_line(&mut line).expect("read JSONL event") > 0,
                "land exited before emitting a wait heartbeat"
            );
            let event: Value = serde_json::from_str(line.trim_end()).expect("parse JSONL event");
            let is_wait = event["event"] == "wait";
            events.push(event);
            if is_wait {
                sender.send((reader, events)).expect("return JSONL reader");
                break;
            }
        }
    });
    let (mut reader, mut events) = match receiver.recv_timeout(Duration::from_secs(20)) {
        Ok(result) => result,
        Err(error) => {
            let _ = land.kill();
            panic!("land did not flush a wait heartbeat: {error}");
        }
    };

    // The command cannot finish until this file exists, so observing the wait
    // event first proves StreamingJson flushed the heartbeat immediately.
    fixture.release_ci();
    let mut remaining_stdout = String::new();
    reader
        .read_to_string(&mut remaining_stdout)
        .expect("read remaining JSONL events");

    let output = land.wait_with_output().expect("wait for land");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(
        output.status.success(),
        "land --jsonl failed: stdout={remaining_stdout} stderr={stderr}"
    );
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    events.extend(
        remaining_stdout
            .lines()
            .map(|line| serde_json::from_str(line).expect("every JSONL line must parse")),
    );
    assert_eq!(events.first().unwrap()["event"], "start");
    assert_eq!(events.first().unwrap()["total_entries"], 1);

    let wait = events
        .iter()
        .find(|event| event["event"] == "wait")
        .expect("pending CI should emit a wait heartbeat");
    assert_eq!(wait["phase"], "readiness");
    assert_eq!(wait["position"], 1);
    assert_eq!(wait["pr_number"], 41);
    assert_eq!(wait["poll"], 1);
    assert_eq!(wait["ci_status"], "pending");
    assert_eq!(wait["approved"], true);
    assert!(wait["elapsed_seconds"].is_number());

    let entry = events
        .iter()
        .find(|event| event["event"] == "entry")
        .expect("merge should emit an entry event");
    assert_eq!(entry["action"], "merged");
    assert_eq!(entry["pr_number"], 41);

    let summary = events.last().unwrap();
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["landed"][0]["action"], "merged");
    assert_eq!(summary["remaining"], 0);
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_admin_emits_no_human_warning() {
    let fixture = WaitingLandFixture::new();
    fixture.release_ci();

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()),
        "every JSONL line must parse: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_index_lock_wait_emits_no_human_warning() {
    let fixture = WaitingLandFixture::new();
    fixture.release_ci();
    let index_lock = fixture.repo_path.join(".git/index.lock");
    fs::write(&index_lock, "locked\n").expect("create index lock");

    let land = fixture.start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"]);
    std::thread::sleep(Duration::from_millis(250));
    fs::remove_file(&index_lock).expect("release index lock");

    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()),
        "every JSONL line must parse: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_default_scope_reports_one_total_entry() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    fixture.release_ci();

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");

    let start: Value =
        serde_json::from_str(stdout.lines().next().expect("start event")).expect("parse start");
    assert_eq!(start["event"], "start");
    assert_eq!(start["total_entries"], 1);

    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["remaining"], 1);
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_default_scope_counts_terminal_prefix_entries() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    fs::write(&fixture.merged, "already merged\n").expect("mark fake PR merged");

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("parse JSONL event"))
        .collect::<Vec<_>>();
    assert_eq!(events.first().unwrap()["event"], "start");
    assert_eq!(events.first().unwrap()["total_entries"], 2);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event"] == "entry")
            .count(),
        2
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_reports_retarget_failure_warning() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    fixture.fail_retargeting();
    fixture.release_ci();

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert!(
        summary["warnings"]
            .as_array()
            .expect("warnings must be an array")
            .iter()
            .any(|warning| warning
                .as_str()
                .is_some_and(|warning| warning.contains("Failed to update PR #42 base"))),
        "summary should include retarget failure warning: {summary}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_reports_downstream_push_failure_warning() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    fixture.release_ci();
    fixture.fail_downstream_push();
    run_git(
        &fixture.repo_path,
        &["branch", "testuser/land-wait--c-2222222"],
    );

    let output = fixture
        .start_land_with_args(&["land", "--all", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert!(
        summary["warnings"]
            .as_array()
            .expect("warnings must be an array")
            .iter()
            .any(|warning| warning.as_str().is_some_and(
                |warning| warning.contains("Failed to push testuser/land-wait--c-2222222")
            )),
        "summary should include downstream push failure warning: {summary}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_jsonl_admin_emits_no_human_warning() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();

    let output = fixture
        .start_land_with_args(&["land", "--all", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(
        fixture.merged.with_extension("glab").exists(),
        "fake GitLab MR should be merged"
    );
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()),
        "every JSONL line must parse: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_clean_silences_provider_lookup_debug() {
    let fixture = WaitingLandFixture::new();
    fs::write(&fixture.merged, "already merged\n").expect("mark fake PR merged");
    fixture.fail_cleanup_provider_after_first_view();

    let output = fixture
        .start_land_with_args(&["land", "--all", "--jsonl", "--clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()),
        "every JSONL line must parse: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_jsonl_network_auth_fallback_emits_no_human_warning() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.fail_auth_with_network_error();

    let output = fixture
        .start_land_with_args(&["land", "--all", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()),
        "every JSONL line must parse: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_jsonl_records_merge_train_merge_before_stale_stack_error() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();

    let land = fixture.start_land_with_args(&["land", "--all", "--wait", "--jsonl", "--no-clean"]);
    fixture.wait_until_queued();
    fixture.wait_until_polling();

    fs::write(fixture.repo_path.join("new.txt"), "new\n").expect("write new entry");
    run_git(&fixture.repo_path, &["add", "new.txt"]);
    run_git(
        &fixture.repo_path,
        &["commit", "-m", "New entry\n\nGG-ID: c-2222222"],
    );
    fs::write(&fixture.merged, "merged\n").expect("complete fake merge train");

    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(
        output.status.success(),
        "structured land reports the error in JSONL: stderr={stderr}"
    );
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let events: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("every JSONL line must parse"))
        .collect();
    assert!(
        events.iter().any(|event| event["event"] == "entry"
            && event["pr_number"] == 41
            && event["action"] == "merged"),
        "merged entry should be emitted before stale-stack error: {stdout}"
    );
    let summary = events.last().expect("summary event");
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["landed"][0]["action"], "merged");
    assert_eq!(summary["remaining"], 0);
    assert!(
        summary["error"]
            .as_str()
            .is_some_and(|error| error.contains("Stack changed while gg land was waiting")),
        "summary should still report the stale-stack error: {summary}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_clean_does_not_prompt_for_a_configured_worktree() {
    let fixture = WaitingLandFixture::new();
    fixture.release_ci();
    let config_path = fixture.repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).expect("read config"))
        .expect("parse config");
    config["stacks"]["land-wait"]["worktree_path"] = Value::String(
        fixture
            .repo_path
            .join("configured-worktree")
            .display()
            .to_string(),
    );
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).expect("serialize config"),
    )
    .expect("write config");

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(
        output.status.success(),
        "land failed: stdout={stdout} stderr={stderr}"
    );
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()),
        "every JSONL line must parse: {stdout}"
    );
    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["cleaned"], false);
    assert!(
        summary["warnings"]
            .as_array()
            .expect("warnings must be an array")
            .iter()
            .any(|warning| warning
                .as_str()
                .is_some_and(|warning| warning.contains("worktree"))),
        "summary should explain why cleanup was skipped: {summary}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_wait_releases_operation_lock_for_sync_in_another_worktree() {
    let fixture = WaitingLandFixture::new();
    let land = fixture.start_land();
    fixture.wait_until_polling();

    let sync = Command::new(env!("CARGO_BIN_EXE_gg"))
        .arg("sync")
        .current_dir(&fixture.other_worktree)
        .env("HOME", &fixture.test_home)
        .output()
        .expect("run sync in other worktree");
    let config_path = fixture.repo_path.join(".git/gg/config.json");
    let mut concurrent_config: Value =
        serde_json::from_slice(&fs::read(&config_path).expect("read config after concurrent sync"))
            .expect("parse config after concurrent sync");
    concurrent_config["stacks"]["other"] = serde_json::json!({"base":"main"});
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&concurrent_config).expect("serialize concurrent config"),
    )
    .expect("simulate unrelated stack config update");
    fixture.release_ci();
    let land_output = land.wait_with_output().expect("wait for land");
    assert!(
        sync.status.success(),
        "sync should run while land is waiting: {}",
        String::from_utf8_lossy(&sync.stderr)
    );
    assert!(
        land_output.status.success(),
        "land should finish: stdout={} stderr={}",
        String::from_utf8_lossy(&land_output.stdout),
        String::from_utf8_lossy(&land_output.stderr)
    );
    assert!(
        fixture.merged.exists(),
        "land should merge after CI becomes ready"
    );
    let final_config: Value =
        serde_json::from_slice(&fs::read(&config_path).expect("read config after land"))
            .expect("parse config after land");
    assert_eq!(final_config["stacks"]["other"]["base"], "main");

    let operations_dir = fixture.repo_path.join(".git/gg/operations");
    for entry in fs::read_dir(operations_dir).expect("read operation records") {
        let path = entry.expect("read operation record entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let record: Value = serde_json::from_slice(&fs::read(path).expect("read operation record"))
            .expect("parse operation record");
        assert_eq!(record["status"], "committed");
    }
}

#[cfg(unix)]
#[test]
fn test_land_wait_aborts_when_target_stack_changes() {
    let fixture = WaitingLandFixture::new();
    let land = fixture.start_land();
    fixture.wait_until_polling();

    fs::write(fixture.repo_path.join("new.txt"), "new\n").expect("write new entry");
    run_git(&fixture.repo_path, &["add", "new.txt"]);
    run_git(
        &fixture.repo_path,
        &["commit", "-m", "New entry\n\nGG-ID: c-2222222"],
    );

    fixture.release_ci();
    let output = land.wait_with_output().expect("wait for land");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "land should reject stale stack state"
    );
    assert!(
        stderr.contains("Stack changed while gg land was waiting"),
        "unexpected error: {stderr}"
    );
    assert!(
        !fixture.merged.exists(),
        "land must not merge after its target stack changes"
    );
}

#[cfg(unix)]
#[test]
fn test_land_wait_rechecks_readiness_after_reacquiring_lock() {
    let fixture = WaitingLandFixture::new();
    fixture.release_ci();
    fs::write(&fixture.regress, "regress once\n").expect("enable readiness regression");

    let land = fixture.start_land();
    let output = land.wait_with_output().expect("wait for land");
    assert!(
        output.status.success(),
        "land should resume waiting and finish: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let ci_calls: u32 = fs::read_to_string(&fixture.ci_calls)
        .expect("read CI call count")
        .trim()
        .parse()
        .expect("parse CI call count");
    assert!(
        ci_calls >= 4,
        "land should recheck readiness and observe the regression; calls={ci_calls}"
    );
}

#[test]
fn test_gg_land_help_has_until() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success);
    assert!(stdout.contains("--until"));
}

#[test]
fn test_gg_land_json_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success);
    assert!(stdout.contains("--json"));
}

#[test]
fn test_gg_land_jsonl_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success);
    assert!(stdout.contains("--jsonl"), "help should mention --jsonl");
}

#[test]
fn test_gg_land_rejects_json_and_jsonl_together() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, _stdout, stderr) = run_gg(&repo_path, &["land", "--json", "--jsonl"]);

    assert!(!success, "--json and --jsonl should conflict");
    assert!(stderr.contains("cannot be used with"));
}

#[test]
fn test_gg_land_json_error_without_provider() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "json-land-error"]);
    assert!(success, "Failed to create stack: {}", stderr);

    let (success, stdout, stderr) = run_gg(&repo_path, &["land", "--json"]);
    assert!(!success, "land --json should fail without provider");
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSON mode"
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert!(parsed["error"].is_string(), "error field must be string");
}

#[test]
fn test_gg_land_jsonl_error_without_provider() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "jsonl-land-error"]);
    assert!(success, "Failed to create stack: {stderr}");

    let (success, stdout, stderr) = run_gg(&repo_path, &["land", "--jsonl"]);
    assert!(!success, "land --jsonl should fail without provider");
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSONL mode: {stderr}"
    );

    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "fatal error should emit one event");
    let event: Value = serde_json::from_str(lines[0]).expect("error line must be valid JSON");
    assert_eq!(event["version"], 1);
    assert_eq!(event["command"], "land");
    assert_eq!(event["status"], "error");
    assert_eq!(event["event"], "error");
    assert!(event["message"].is_string());
}

#[test]
fn test_land_help_shows_no_squash_option() {
    let (_temp_dir, repo_path) = create_test_repo();

    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success, "Help should succeed");
    assert!(
        stdout.contains("--no-squash"),
        "Should show --no-squash option: {}",
        stdout
    );
    assert!(
        stdout.contains("squash") && stdout.contains("default"),
        "Should mention squash is default: {}",
        stdout
    );
}

#[test]
fn test_land_help_shows_admin_option() {
    let (_temp_dir, repo_path) = create_test_repo();

    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success, "Help should succeed");
    assert!(
        stdout.contains("--admin"),
        "Should show --admin option: {}",
        stdout
    );
    assert!(
        stdout.contains("GitHub only") || stdout.contains("GitHub-only"),
        "Should indicate --admin is GitHub-only: {}",
        stdout
    );
}

#[test]
fn test_land_admin_flag_accepted() {
    // Test that the --admin flag is recognized and doesn't cause a clap error
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config with username
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-stack"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Verify --admin flag is accepted (it will fail for other reasons,
    // like no PRs to land, but should not fail on unknown argument)
    let (_, _stdout, stderr) = run_gg(&repo_path, &["land", "--admin"]);

    // Should not contain clap errors
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("invalid value"),
        "The --admin flag should be recognized, stderr: {}",
        stderr
    );
}

#[test]
fn test_land_admin_config_default() {
    // Test that land_admin config defaults to false (not present in minimal config)
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(gg_dir.join("config.json"), r#"{"defaults":{}}"#).expect("Failed to write config");

    let config_path = gg_dir.join("config.json");
    let content = fs::read_to_string(config_path).expect("Failed to read config");

    assert!(
        !content.contains("land_admin"),
        "Default config should not contain land_admin when false"
    );
}

#[test]
fn test_land_admin_config_enabled() {
    // Test that land_admin can be set to true in config
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"land_admin":true}}"#,
    )
    .expect("Failed to write config");

    let config_path = gg_dir.join("config.json");
    let content = fs::read_to_string(config_path).expect("Failed to read config");

    assert!(
        content.contains("\"land_admin\":true"),
        "Config should contain land_admin when enabled"
    );
}
