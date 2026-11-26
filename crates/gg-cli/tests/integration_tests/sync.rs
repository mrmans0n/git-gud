#[cfg(unix)]
use crate::helpers::ProcessGroupGuard;
use crate::helpers::{
    create_test_repo, create_test_repo_with_remote, run_gg, run_gg_with_env, run_git,
};

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use nix::{
    libc,
    sys::signal::{kill, Signal},
    unistd::Pid,
};
#[cfg(unix)]
use std::io::{BufRead, BufReader};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::{fs::PermissionsExt, net::UnixStream};

#[cfg(unix)]
#[path = "sync_if_needed.rs"]
mod if_needed;

fn checked_git(repo: &Path, args: &[&str]) -> String {
    let (success, output) = run_git(repo, args);
    assert!(success, "git {args:?} failed: {output}");
    output.trim().to_string()
}

fn operation_record_ids(repo_path: &Path) -> BTreeSet<String> {
    let operations_dir = repo_path.join(".git/gg/operations");
    let Ok(entries) = fs::read_dir(operations_dir) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(|entry| {
            let entry = entry.expect("read operation record");
            if !entry
                .file_type()
                .expect("read operation record file type")
                .is_file()
            {
                return None;
            }
            entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".json"))
                .map(str::to_string)
        })
        .collect()
}

fn operation_record(repo_path: &Path, id: &str) -> Value {
    serde_json::from_slice(
        &fs::read(
            repo_path
                .join(".git/gg/operations")
                .join(format!("{id}.json")),
        )
        .expect("read exact operation record"),
    )
    .expect("parse exact operation record")
}

#[cfg(unix)]
fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write executable");
    let mut permissions = fs::metadata(path).expect("stat executable").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make executable");
}

fn install_fake_gh(repo_path: &Path) -> std::ffi::OsString {
    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("Failed to create fake bin dir");
    fs::write(
        fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu

if [ -n "${GG_TEST_GH_LOG:-}" ]; then
  printf '%s\n' "$*" >> "$GG_TEST_GH_LOG"
fi

if [ "$1" = "--version" ]; then
  echo "gh version 2.0.0"
  exit 0
fi

if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  count=$(cat "$(dirname "$0")/create-count" 2>/dev/null || echo 0)
  number=$((101 + count))
  echo $((count + 1)) > "$(dirname "$0")/create-count"
  shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --head) printf '%s\n' "$2" > "$(dirname "$0")/created-head-$number"; shift 2;;
      --base) printf '%s\n' "$2" > "$(dirname "$0")/created-base-$number"; shift 2;;
      *) shift;;
    esac
  done
  echo "https://github.com/test/repo/pull/$number"
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "edit" ]; then
  number=$3
  shift 3
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --base) printf '%s\n' "$2" > "$(dirname "$0")/created-base-$number"; shift 2;;
      *) shift;;
    esac
  done
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  number=$3
  count_file="$(dirname "$0")/view-count-$number"
  count=$(cat "$count_file" 2>/dev/null || echo 0)
  count=$((count + 1))
  printf '%s\n' "$count" > "$count_file"
  head=$(cat "$(dirname "$0")/created-head-$number" 2>/dev/null || true)
  base=$(cat "$(dirname "$0")/created-base-$number" 2>/dev/null || true)
  if [ "${GG_TEST_AUTH_RACE_REVIEW:-}" = "$number" ] && [ "$count" -ge "${GG_TEST_AUTH_RACE_ON_VIEW:-999999}" ]; then
    base=${GG_TEST_AUTH_RACE_BASE:-unrelated}
  fi
  printf '{"number":%s,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/%s","headRefName":"%s","baseRefName":"%s","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}\n' "$number" "$number" "$head" "$base"
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "edit" ]; then
  exit 0
fi

if [ "$1" = "api" ] && [ "$2" = "-X" ] && [ "$3" = "POST" ]; then
  echo '{}'
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "close" ]; then
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

if [ -n "${GG_TEST_GIT_LOG:-}" ]; then
  printf '%s\n' "$*" >> "$GG_TEST_GIT_LOG"
fi

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
      obstruct_config_after_delete)
        pending=''
        for record in "$GG_TEST_GIT_DIR/gg/operations/"*.json; do
          grep -q '"status": "pending"' "$record" || continue
          [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
          pending=$(basename "$record" .json)
        done
        [ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
        printf '%s\n' "$pending" > "$(dirname "$0")/delete-operation-id"
        echo 1 > "$(dirname "$0")/delete-count"
        mkdir "$GG_TEST_GIT_DIR/gg/config.tmp"
        ;;
      block_post_delete_completion)
        pending=''
        for record in "$GG_TEST_GIT_DIR/gg/operations/"*.json; do
          grep -q '"status": "pending"' "$record" || continue
          [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
          pending=$(basename "$record" .json)
        done
        [ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
        printf '%s\n' "$pending" > "$(dirname "$0")/delete-operation-id"
        echo 1 > "$(dirname "$0")/delete-count"
        : > "$(dirname "$0")/delete-complete"
        while [ ! -f "$(dirname "$0")/config-lock-held" ]; do sleep 0.01; done
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

fn run_gg_bounded_with_env(
    repo_path: &Path,
    args: &[&str],
    envs: &[(&str, &std::ffi::OsStr)],
) -> (bool, String, String) {
    run_gg_bounded_with_env_and_timeout(repo_path, args, envs, Duration::from_secs(30), None)
}

#[cfg(not(unix))]
fn stop_bounded_process(child: &mut std::process::Child) {
    let _ = child.kill();
    child.wait().expect("reap stopped gg process");
}

fn run_gg_bounded_with_env_and_timeout(
    repo_path: &Path,
    args: &[&str],
    envs: &[(&str, &std::ffi::OsStr)],
    timeout: Duration,
    readiness_path: Option<&Path>,
) -> (bool, String, String) {
    let test_home = repo_path.join(".git/test-home");
    let output_dir = repo_path.join(".git/gg-test-output");
    fs::create_dir_all(&test_home).expect("create bounded process home directory");
    fs::create_dir_all(&output_dir).expect("create bounded process output directory");
    let stdout_path = output_dir.join("stdout");
    let stderr_path = output_dir.join("stderr");
    let mut command = Command::new(env!("CARGO_BIN_EXE_gg"));
    command
        .args(args)
        .current_dir(repo_path)
        .env("HOME", test_home)
        .stdout(Stdio::from(
            fs::File::create(&stdout_path).expect("create bounded stdout file"),
        ))
        .stderr(Stdio::from(
            fs::File::create(&stderr_path).expect("create bounded stderr file"),
        ));
    for (key, value) in envs {
        command.env(key, value);
    }
    #[cfg(unix)]
    let mut child = ProcessGroupGuard::spawn(&mut command).expect("spawn bounded gg process");
    #[cfg(not(unix))]
    let mut child = command.spawn().expect("spawn bounded gg process");
    if let Some(readiness_path) = readiness_path {
        let setup_timeout = Duration::from_secs(10);
        let setup_deadline = Instant::now() + setup_timeout;
        while !readiness_path.exists() {
            if Instant::now() >= setup_deadline {
                #[cfg(unix)]
                drop(child);
                #[cfg(not(unix))]
                stop_bounded_process(&mut child);
                panic!(
                    "gg setup timed out after {setup_timeout:?} waiting for {}\nstdout:\n{}\nstderr:\n{}",
                    readiness_path.display(),
                    fs::read_to_string(&stdout_path).expect("read setup-timeout stdout"),
                    fs::read_to_string(&stderr_path).expect("read setup-timeout stderr")
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    let deadline = Instant::now() + timeout;
    let status = loop {
        #[cfg(unix)]
        let status = child
            .poll_exit_status_non_reaping()
            .expect("poll bounded gg process");
        #[cfg(not(unix))]
        let status = child.try_wait().expect("poll bounded gg process");
        if let Some(status) = status {
            break status;
        }
        if Instant::now() >= deadline {
            #[cfg(unix)]
            drop(child);
            #[cfg(not(unix))]
            stop_bounded_process(&mut child);
            panic!(
                "gg timed out after {timeout:?}\nstdout:\n{}\nstderr:\n{}",
                fs::read_to_string(&stdout_path).expect("read timed-out stdout"),
                fs::read_to_string(&stderr_path).expect("read timed-out stderr")
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    drop(child);
    (
        status.success(),
        fs::read_to_string(stdout_path).expect("read bounded stdout"),
        fs::read_to_string(stderr_path).expect("read bounded stderr"),
    )
}

#[test]
fn operation_record_ids_accepts_only_regular_json_files() {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = temp.path();
    let operations = repo_path.join(".git/gg/operations");
    fs::create_dir_all(&operations).unwrap();
    fs::write(operations.join("op-valid.json"), "{}").unwrap();
    fs::write(operations.join("op-temp.json.tmp"), "{}").unwrap();
    fs::write(operations.join("op-upper.JSON"), "{}").unwrap();
    fs::write(operations.join("op-no-suffix"), "{}").unwrap();
    fs::create_dir(operations.join("op-directory.json")).unwrap();

    assert_eq!(
        operation_record_ids(repo_path),
        ["op-valid".to_string()].into_iter().collect()
    );
}

#[cfg(unix)]
#[test]
fn process_group_guard_drop_stops_group_after_panic() {
    let temp = tempfile::tempdir().unwrap();
    let heartbeat = temp.path().join("heartbeat");
    let ready = temp.path().join("ready");
    let script = temp.path().join("guard-child.sh");
    write_executable(
        &script,
        r#"#!/bin/sh
set -eu
: > "$GUARD_HEARTBEAT"
(while :; do printf x >> "$GUARD_HEARTBEAT"; sleep 0.05; done) &
touch "$GUARD_READY"
wait
"#,
    );
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut command = Command::new(&script);
        command
            .env("GUARD_HEARTBEAT", &heartbeat)
            .env("GUARD_READY", &ready);
        let mut child = ProcessGroupGuard::spawn(&mut command).expect("spawn guarded group");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(child.try_wait().unwrap().is_none(), "guarded child exited");
            assert!(Instant::now() < deadline, "guarded child was not ready");
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("exercise guard drop");
    }));
    assert!(panicked.is_err());
    let heartbeat_len = fs::metadata(&heartbeat).unwrap().len();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        fs::metadata(&heartbeat).unwrap().len(),
        heartbeat_len,
        "descendant survived guard drop"
    );
    fs::write(&heartbeat, "reused").expect("heartbeat file must be reusable after cleanup");
}

#[cfg(unix)]
#[test]
fn process_group_guard_poll_keeps_exited_leader_owned_until_drop() {
    let temp = tempfile::tempdir().unwrap();
    let heartbeat = temp.path().join("heartbeat-after-leader-exit");
    let descendant_pid = temp.path().join("descendant-pid");
    let script = temp.path().join("exiting-guard-leader.sh");
    write_executable(
        &script,
        r#"#!/bin/sh
set -eu
: > "$GUARD_HEARTBEAT"
(while :; do printf x >> "$GUARD_HEARTBEAT"; sleep 0.05; done) &
printf '%s\n' "$!" > "$GUARD_DESCENDANT_PID"
exit 23
"#,
    );

    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut command = Command::new(&script);
        command
            .env("GUARD_HEARTBEAT", &heartbeat)
            .env("GUARD_DESCENDANT_PID", &descendant_pid);
        let mut child = ProcessGroupGuard::spawn(&mut command).expect("spawn guarded group");
        wait_for_file(&descendant_pid);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().expect("poll exited group leader") {
                assert_eq!(status.code(), Some(23));
                break;
            }
            assert!(Instant::now() < deadline, "group leader did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("drop guard after observing leader exit");
    }));
    assert!(panicked.is_err());

    wait_for_file(&heartbeat);
    let heartbeat_len = fs::metadata(&heartbeat).unwrap().len();
    std::thread::sleep(Duration::from_millis(250));
    let descendant_stopped = fs::metadata(&heartbeat).unwrap().len() == heartbeat_len;
    if !descendant_stopped {
        let pid = fs::read_to_string(&descendant_pid)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
    }
    assert!(
        descendant_stopped,
        "descendant survived after status polling reaped the group leader"
    );
    fs::write(&heartbeat, "reused").expect("heartbeat file must be reusable after cleanup");
}

#[cfg(unix)]
#[test]
fn bounded_test_command_timeout_waits_for_readiness_and_stops_process_group() {
    let (_temp, repo_path, _remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let (ok, out, err) = run_gg(&repo_path, &["co", "timeout-tree"]);
    assert!(ok, "checkout failed: {out}\n{err}");
    fs::write(repo_path.join("entry"), "entry").unwrap();
    checked_git(&repo_path, &["add", "entry"]);
    checked_git(&repo_path, &["commit", "-m", "Entry"]);

    let fake_bin = repo_path.join(".git/timeout-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("timeout-child"),
        r#"#!/bin/sh
set -eu
: > "$GG_TEST_TIMEOUT_HEARTBEAT"
: > "$GG_TEST_TIMEOUT_CHILD_READY"
while :; do printf x >> "$GG_TEST_TIMEOUT_HEARTBEAT"; sleep 0.05; done
"#,
    );
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
sleep 1
"$(dirname "$0")/timeout-child" &
while [ ! -f "$GG_TEST_TIMEOUT_CHILD_READY" ]; do sleep 0.01; done
: > "$GG_TEST_TIMEOUT_READY"
while :; do sleep 60; done
"#,
    );
    let mut paths = vec![fake_bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let readiness = repo_path.join(".git/timeout-ready");
    let child_ready = repo_path.join(".git/timeout-child-ready");
    let heartbeat = repo_path.join(".git/timeout-heartbeat");

    let timed_out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_gg_bounded_with_env_and_timeout(
            &repo_path,
            &["sync", "--no-rebase-check"],
            &[
                ("PATH", path.as_os_str()),
                ("GG_TEST_TIMEOUT_READY", readiness.as_os_str()),
                ("GG_TEST_TIMEOUT_CHILD_READY", child_ready.as_os_str()),
                ("GG_TEST_TIMEOUT_HEARTBEAT", heartbeat.as_os_str()),
            ],
            Duration::from_millis(300),
            Some(&readiness),
        )
    }));
    let panic = timed_out.expect_err("bounded command unexpectedly completed");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .expect("timeout panic must contain a message");
    assert!(
        message.contains("gg timed out after 300ms"),
        "unexpected panic: {message}"
    );
    assert!(
        readiness.exists(),
        "execution deadline started before provider readiness"
    );
    wait_for_file(&heartbeat);
    let heartbeat_len = fs::metadata(&heartbeat).unwrap().len();
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        heartbeat_len > 0,
        "descendant did not write its heartbeat before timeout"
    );
    assert_eq!(
        fs::metadata(&heartbeat).unwrap().len(),
        heartbeat_len,
        "timed-out descendant continued running after process-group cleanup"
    );
    fs::write(&heartbeat, "reused").expect("heartbeat file must be reusable after cleanup");
    fs::remove_file(heartbeat).unwrap();
}

#[cfg(unix)]
#[test]
fn bounded_test_command_stops_descendant_after_normal_leader_exit() {
    let (_temp, repo_path, _remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let (ok, out, err) = run_gg(&repo_path, &["co", "normal-exit-tree"]);
    assert!(ok, "checkout failed: {out}\n{err}");
    fs::write(repo_path.join("entry"), "entry").unwrap();
    checked_git(&repo_path, &["add", "entry"]);
    checked_git(&repo_path, &["commit", "-m", "Entry"]);

    let fake_bin = repo_path.join(".git/normal-exit-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
if [ "$1" = "--version" ]; then
  echo 'gh version 2.0.0'
  exit 0
fi
if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  (
    trap '' HUP
    : > "$GG_TEST_NORMAL_EXIT_HEARTBEAT"
    printf x >> "$GG_TEST_NORMAL_EXIT_HEARTBEAT"
    : > "$GG_TEST_NORMAL_EXIT_READY"
    while [ ! -e "$GG_TEST_NORMAL_EXIT_STOP" ]; do
      printf x >> "$GG_TEST_NORMAL_EXIT_HEARTBEAT"
      sleep 0.05
    done
    : > "$GG_TEST_NORMAL_EXIT_STOPPED"
  ) </dev/null >/dev/null 2>&1 &
  attempts=0
  while [ ! -e "$GG_TEST_NORMAL_EXIT_READY" ]; do
    attempts=$((attempts + 1))
    if [ "$attempts" -ge 400 ]; then
      echo 'descendant readiness timed out' >&2
      exit 2
    fi
    sleep 0.01
  done
  echo 'not authenticated' >&2
  exit 1
fi
echo "unexpected gh invocation: $*" >&2
exit 2
"#,
    );
    let mut paths = vec![fake_bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let ready = repo_path.join(".git/normal-exit-ready");
    let heartbeat = repo_path.join(".git/normal-exit-heartbeat");
    let stop = repo_path.join(".git/normal-exit-stop");
    let stopped = repo_path.join(".git/normal-exit-stopped");

    let (ok, out, err) = run_gg_bounded_with_env_and_timeout(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_NORMAL_EXIT_READY", ready.as_os_str()),
            ("GG_TEST_NORMAL_EXIT_HEARTBEAT", heartbeat.as_os_str()),
            ("GG_TEST_NORMAL_EXIT_STOP", stop.as_os_str()),
            ("GG_TEST_NORMAL_EXIT_STOPPED", stopped.as_os_str()),
        ],
        Duration::from_secs(10),
        Some(&ready),
    );
    assert!(!ok, "auth failure unexpectedly succeeded: {out}\n{err}");
    assert!(err.contains("Not authenticated with GitHub"), "{err}");

    let heartbeat_len = fs::metadata(&heartbeat).unwrap().len();
    std::thread::sleep(Duration::from_millis(250));
    let descendant_stopped = fs::metadata(&heartbeat).unwrap().len() == heartbeat_len;
    if !descendant_stopped {
        fs::write(&stop, "").expect("request leaked descendant cleanup");
        wait_for_file(&stopped);
    }
    assert!(
        descendant_stopped,
        "bounded runner returned while a provider descendant was still active"
    );
    fs::write(&heartbeat, "reused").expect("heartbeat file must be reusable after cleanup");
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

    let operations_before = operation_record_ids(&repo_path);
    let (success, stdout, stderr) = run_gg_bounded_with_env(&repo_path, &["sync", "--jsonl"], &[]);
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
    assert_eq!(
        operation_record_ids(&repo_path),
        operations_before,
        "provider preflight failure must not create an operation record"
    );
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

#[cfg(unix)]
#[test]
fn test_empty_stack_auth_failure_precedes_pending_remote_deletion() {
    let (_temp_dir, repo_path, remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "empty-auth"]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");

    let pending_branch = "testuser/empty-auth--c-1111111";
    let expected_oid = checked_git(&repo_path, &["rev-parse", "HEAD"]);
    checked_git(
        &repo_path,
        &[
            "push",
            "origin",
            &format!("{expected_oid}:refs/heads/{pending_branch}"),
        ],
    );
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["pending_remote_branch_deletions"] = serde_json::json!([{
        "remote": "origin",
        "branch": pending_branch,
        "expected_oid": expected_oid,
        "source_operation_id": "op_test_pending_delete",
        "state": "pending"
    }]);
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let pending_before = config["pending_remote_branch_deletions"].clone();

    let fake_bin = repo_path.join("fake-auth-bin");
    fs::create_dir(&fake_bin).unwrap();
    let requests = repo_path.join(".git/provider-requests");
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$GG_TEST_PROVIDER_REQUESTS"
if [ "$1" = "--version" ]; then
  echo 'gh version 2.0.0'
  exit 0
fi
if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  echo 'not authenticated' >&2
  exit 1
fi
echo "unexpected provider mutation: $*" >&2
exit 2
"#,
    );
    write_executable(
        &repo_path.join(".git/hooks/pre-push"),
        "#!/bin/sh\nprintf 'push\n' >> \"$(git rev-parse --absolute-git-dir)/pushes\"\n",
    );
    let mut paths = vec![fake_bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let operations_before = operation_record_ids(&repo_path);

    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_PROVIDER_REQUESTS", requests.as_os_str()),
        ],
    );

    assert!(!success, "auth failure must stop sync: {stdout}\n{stderr}");
    assert!(stderr.contains("Not authenticated with GitHub"), "{stderr}");
    assert_eq!(
        fs::read_to_string(&requests)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["--version", "auth status"],
        "only executable and auth preflight requests are allowed"
    );
    assert_eq!(operation_record_ids(&repo_path), operations_before);
    assert!(
        !repo_path.join(".git/pushes").exists(),
        "unexpected push or deletion"
    );
    assert!(
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{pending_branch}")
            ]
        )
        .0,
        "pending branch must not be deleted"
    );
    let config: Value = serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();
    assert_eq!(config["pending_remote_branch_deletions"], pending_before);
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
fn sync_lint_rebase_conflict_preserves_exact_pending_operation() {
    let (_temp, repo, _remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo);
    let path = install_fake_gh(&repo);
    let config_path = repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["lint"] = serde_json::json!([
        "if [ \"$(git log -1 --format=%s)\" = \"Lower\" ]; then printf 'linted\\n' > conflict.txt; fi"
    ]);
    fs::write(config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let (success, stdout, stderr) = run_gg(&repo, &["co", "lint-rebase-conflict"]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");
    fs::write(repo.join("conflict.txt"), "lower\n").unwrap();
    checked_git(&repo, &["add", "conflict.txt"]);
    checked_git(&repo, &["commit", "-m", "Lower\n\nGG-ID: c-1111111"]);
    fs::write(repo.join("conflict.txt"), "descendant\n").unwrap();
    checked_git(&repo, &["add", "conflict.txt"]);
    checked_git(
        &repo,
        &[
            "commit",
            "-m",
            "Descendant\n\nGG-ID: c-2222222\nGG-Parent: c-1111111",
        ],
    );
    let operations_before = operation_record_ids(&repo);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo,
        &["sync", "--lint", "--no-rebase-check"],
        &[("PATH", path.as_os_str())],
    );

    assert!(
        !success,
        "conflicting lint unexpectedly succeeded: {stdout}\n{stderr}"
    );
    let error = format!("{stdout}{stderr}");
    assert!(
        error.contains("Rebase conflict occurred. Resolve conflicts and run `gg continue`."),
        "sync lost the original lint error: {error}"
    );
    assert!(
        repo.join(".git/rebase-merge").exists() || repo.join(".git/rebase-apply").exists(),
        "lint must leave the descendant replay conflict active"
    );
    let new_ids = operation_record_ids(&repo)
        .difference(&operations_before)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(new_ids.len(), 1, "conflict must use one sync operation");
    let operation_id = new_ids.into_iter().next().unwrap();
    let operation = operation_record(&repo, &operation_id);
    assert_eq!(operation["kind"], "sync", "{operation}");
    let marker_path = repo.join(".git/gg/interrupted-rebase-operation.json");
    assert_eq!(
        (operation["status"].as_str(), marker_path.exists()),
        (Some("pending"), true),
        "lint conflict must preserve the pending operation and exact-ownership marker: {operation}"
    );
    let marker: Value = serde_json::from_slice(&fs::read(marker_path).unwrap()).unwrap();
    assert_eq!(marker["operation_id"], operation_id);
}

#[test]
fn sync_until_publishes_only_the_rewritten_untagged_target() {
    for (rebase, already_applied) in [(false, false), (true, false), (true, true)] {
        let (_temp, repo, remote) = create_test_repo_with_remote();
        write_remote_prune_config(&repo);
        let path = install_fake_gh(&repo.join(".git"));
        let config_path = repo.join(".git/gg/config.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        config["defaults"]["lint"] = serde_json::json!(["echo fixed > selected"]);
        config["defaults"]["sync_behind_threshold"] = 1.into();
        config["defaults"]["sync_auto_rebase"] = true.into();
        fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        checked_git(&repo, &["checkout", "-b", "testuser/partial"]);
        for file in ["selected", "later"] {
            fs::write(repo.join(file), file).unwrap();
            checked_git(&repo, &["add", file]);
            checked_git(&repo, &["commit", "-m", file]);
        }
        if rebase {
            checked_git(&repo, &["checkout", "main"]);
            checked_git(&repo, &["commit", "--allow-empty", "-m", "Advance base"]);
            if already_applied {
                checked_git(&repo, &["cherry-pick", "testuser/partial~1"]);
            }
            checked_git(&repo, &["push", "origin", "main"]);
            checked_git(&repo, &["checkout", "testuser/partial"]);
        }

        let (ok, out, err) = run_gg_with_env(
            &repo,
            &["sync", "--until", "1", "--lint", "--json"],
            &[("PATH", path.as_os_str())],
        );
        let branches = checked_git(
            &remote,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads/testuser/partial--*",
            ],
        );
        if already_applied {
            assert!(
                !ok && out.contains("Cannot identify --until entry"),
                "{out}\n{err}"
            );
            assert!(
                branches.is_empty(),
                "published past the dropped target: {branches}"
            );
            continue;
        }
        assert!(ok, "{out}\n{err}");
        let result: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(result["sync"]["rebased_before_sync"], rebase);
        assert_eq!(branches.lines().count(), 1);
        assert_eq!(
            checked_git(&remote, &["show", &format!("{branches}:selected")]),
            "fixed"
        );
        assert!(!run_git(&remote, &["cat-file", "-e", &format!("{branches}:later")]).0);
    }
}

#[cfg(unix)]
#[test]
fn jsonl_disconnect_after_navigation_comment_stops_before_next_comment() {
    let (_temp, repo, _remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo);
    let config_path = repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["provider"] = "github".into();
    config["defaults"]["sync_behind_threshold"] = 0.into();
    config["defaults"]["stack_nav_comments"] = true.into();
    config["defaults"]["github"]["stacks_integration"] = "off".into();
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let (ok, out, err) = run_gg(&repo, &["co", "nav-disconnect"]);
    assert!(ok, "checkout failed: {out}\n{err}");
    for (file, message) in [
        ("one", "One\n\nGG-ID: c-1111111"),
        ("two", "Two\n\nGG-ID: c-2222222\nGG-Parent: c-1111111"),
    ] {
        fs::write(repo.join(file), file).unwrap();
        checked_git(&repo, &["add", file]);
        checked_git(&repo, &["commit", "-m", message]);
    }
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["nav-disconnect"]["mrs"]["c-1111111"] = 41.into();
    config["stacks"]["nav-disconnect"]["mrs"]["c-2222222"] = 42.into();
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    checked_git(
        &repo,
        &[
            "push",
            "origin",
            "HEAD~1:refs/heads/testuser/nav-disconnect--c-1111111",
            "HEAD:refs/heads/testuser/nav-disconnect--c-2222222",
        ],
    );

    let fake_bin = repo.join("fake-nav-disconnect-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr view')
    n=$3
    if [ "$n" = 41 ]; then head=testuser/nav-disconnect--c-1111111; base=main; else head=testuser/nav-disconnect--c-2222222; base=testuser/nav-disconnect--c-1111111; fi
    printf '{"number":%s,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/%s","headRefName":"%s","baseRefName":"%s","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}\n' "$n" "$n" "$head" "$base";;
  'pr edit') exit 0;;
  'api repos/'*) echo '[]';;
  'api -X')
    [ "$3" = POST ] || { echo "unexpected gh $*" >&2; exit 1; }
    attempts=$(cat "$dir/comment-attempts" 2>/dev/null || echo 0)
    attempts=$((attempts + 1))
    echo "$attempts" > "$dir/comment-attempts"
    printf '%s\n' "$4" >> "$dir/comment-targets"
    if [ "$attempts" = 1 ]; then
      pending=''
      for record in "$TEST_GIT_DIR/gg/operations/"*.json; do
        grep -q '"status": "pending"' "$record" || continue
        [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
        pending=$(basename "$record" .json)
      done
      [ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
      printf '%s\n' "$pending" > "$dir/comment-operation-id"
      touch "$dir/comment-started"
      while [ ! -f "$dir/release-comment" ]; do sleep 0.01; done
    fi
    echo '{}';;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();

    let (read_stream, write_stream) = UnixStream::pair().unwrap();
    read_stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let stdout_fd: std::os::fd::OwnedFd = write_stream.into();
    let stderr_path = repo.join(".git/nav-disconnect-stderr");
    let mut command = Command::new(env!("CARGO_BIN_EXE_gg"));
    command
        .args(["sync", "--no-rebase-check", "--jsonl"])
        .current_dir(&repo)
        .env("PATH", path)
        .env("TEST_GIT_DIR", repo.join(".git"))
        .stdout(Stdio::from(stdout_fd))
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
    let mut child = ProcessGroupGuard::spawn(&mut command).expect("start sync");
    let mut reader = BufReader::new(read_stream);
    let mut first = String::new();
    reader.read_line(&mut first).expect("read start event");
    assert_eq!(
        serde_json::from_str::<Value>(&first).unwrap()["event"],
        "start"
    );

    let readiness_deadline = Instant::now() + Duration::from_secs(10);
    while !fake_bin.join("comment-started").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "sync exited before navigation comment ({status}): {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        if Instant::now() >= readiness_deadline {
            let _ = fs::write(fake_bin.join("release-comment"), "release");
            panic!("sync did not reach navigation comment");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let operation_id = fs::read_to_string(fake_bin.join("comment-operation-id"))
        .unwrap()
        .trim()
        .to_string();
    let pending = operation_record(&repo, &operation_id);
    assert_eq!(pending["status"], "pending", "{pending}");
    assert_eq!(pending["touched_remote"], true, "{pending}");

    drop(reader);
    fs::write(fake_bin.join("release-comment"), "release").unwrap();
    let process_deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= process_deadline {
            panic!("sync did not stop after navigation-comment disconnect");
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(!status.success(), "closed JSONL consumer must abort sync");
    assert_eq!(
        fs::read_to_string(fake_bin.join("comment-attempts"))
            .unwrap()
            .trim(),
        "1"
    );
    let record = operation_record(&repo, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
}

#[cfg(unix)]
#[test]
fn ordinary_navigation_mutations_revalidate_review() {
    for (mode, enabled, forbidden) in [
        ("create", true, "api -X POST"),
        ("update", true, "api -X PATCH"),
        ("delete", false, "api -X DELETE"),
    ] {
        let (_temp, repo, _remote) = create_test_repo_with_remote();
        write_remote_prune_config(&repo);
        let config_path = repo.join(".git/gg/config.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        config["defaults"]["provider"] = "github".into();
        config["defaults"]["sync_behind_threshold"] = 0.into();
        config["defaults"]["stack_nav_comments"] = enabled.into();
        fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let (ok, out, err) = run_gg(&repo, &["co", "nav-race"]);
        assert!(ok, "checkout failed: {out}\n{err}");
        for (file, message) in [
            ("one", "One\n\nGG-ID: c-1111111"),
            ("two", "Two\n\nGG-ID: c-2222222\nGG-Parent: c-1111111"),
        ] {
            fs::write(repo.join(file), file).unwrap();
            checked_git(&repo, &["add", file]);
            checked_git(&repo, &["commit", "-m", message]);
        }
        let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        config["stacks"]["nav-race"]["mrs"]["c-1111111"] = 41.into();
        config["stacks"]["nav-race"]["mrs"]["c-2222222"] = 42.into();
        fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        checked_git(
            &repo,
            &[
                "push",
                "origin",
                "HEAD~1:refs/heads/testuser/nav-race--c-1111111",
                "HEAD:refs/heads/testuser/nav-race--c-2222222",
            ],
        );

        let fake_bin = repo.join("fake-nav-race-bin");
        fs::create_dir(&fake_bin).unwrap();
        let log = fake_bin.join("requests");
        write_executable(
            &fake_bin.join("gh"),
            r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s\n' "$*" >> "$dir/requests"
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr view')
    n=$3
    views=$(cat "$dir/views$n" 2>/dev/null || echo 0)
    views=$((views + 1))
    printf '%s\n' "$views" > "$dir/views$n"
    state=OPEN
    [ ! -f "$dir/listed$n" ] || state=CLOSED
    if [ "$n" = 41 ]; then head=testuser/nav-race--c-1111111; base=main; else head=testuser/nav-race--c-2222222; base=testuser/nav-race--c-1111111; fi
    printf '{"number":%s,"title":"Entry","state":"%s","url":"https://github.com/test/repo/pull/%s","headRefName":"%s","baseRefName":"%s","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}\n' "$n" "$state" "$n" "$head" "$base";;
  'pr edit') exit 0;;
  'api '*)
    case "$2" in
      *'/issues/'*'/comments?per_page=100&page=1')
        n=$(printf '%s' "$2" | sed -E 's#.*issues/([0-9]+)/.*#\1#')
        : > "$dir/listed$n"
        case "$GG_NAV_RACE" in
          create) echo '[]';;
          *) echo '[{"id":77,"body":"old\\n<!-- gg:stack-nav -->"}]';;
        esac;;
      *) echo "unexpected gh $*" >&2; exit 1;;
    esac;;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
        );
        let mut paths = vec![fake_bin.clone()];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let path = std::env::join_paths(paths).unwrap();

        let (ok, out, err) = run_gg_bounded_with_env(
            &repo,
            &["sync", "--no-rebase-check"],
            &[("PATH", path.as_os_str()), ("GG_NAV_RACE", mode.as_ref())],
        );

        assert!(
            !ok,
            "{mode}: stale review allowed nav mutation: {out}\n{err}"
        );
        assert!(
            format!("{out}{err}").contains("not open at the expected source and target"),
            "{mode}: {out}\n{err}"
        );
        assert_eq!(
            fs::read_to_string(fake_bin.join("views41")).unwrap().trim(),
            "5",
            "{mode}: unexpected review read count"
        );
        let requests = fs::read_to_string(&log).unwrap();
        assert!(!requests.contains(forbidden), "{mode}: {requests}");
    }
}

#[cfg(unix)]
#[test]
fn ordinary_replacement_followups_revalidate_old_review() {
    for (mode, expected_views, forbidden) in [
        ("before-comment", "4", "api -X POST"),
        ("before-close", "5", "pr close 428"),
    ] {
        let (_temp, repo, _remote) = create_test_repo_with_remote();
        write_remote_prune_config(&repo);
        let config_path = repo.join(".git/gg/config.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        config["defaults"]["provider"] = "github".into();
        config["defaults"]["sync_behind_threshold"] = 0.into();
        fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        let (ok, out, err) = run_gg(&repo, &["co", "replacement-race"]);
        assert!(ok, "checkout failed: {out}\n{err}");
        fs::write(repo.join("entry"), "entry").unwrap();
        checked_git(&repo, &["add", "entry"]);
        checked_git(&repo, &["commit", "-m", "Entry\n\nGG-ID: c-1111111"]);
        let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        config["stacks"]["replacement-race"]["mrs"]["c-1111111"] = 428.into();
        fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();

        let fake_bin = repo.join("fake-replacement-race-bin");
        fs::create_dir(&fake_bin).unwrap();
        let log = fake_bin.join("requests");
        let views = fake_bin.join("views");
        write_executable(
            &fake_bin.join("gh"),
            r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s\n' "$*" >> "$dir/requests"
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr view')
    views=$(cat "$dir/views" 2>/dev/null || echo 0)
    views=$((views + 1))
    printf '%s\n' "$views" > "$dir/views"
    state=OPEN
    if [ "$GG_REPLACEMENT_RACE" = before-comment ] && [ -f "$dir/created" ]; then state=CLOSED; fi
    if [ "$GG_REPLACEMENT_RACE" = before-close ] && [ -f "$dir/commented" ]; then state=CLOSED; fi
    printf '{"number":428,"title":"Entry","state":"%s","url":"https://github.com/test/repo/pull/428","headRefName":"testuser/old-stack--c-1111111","baseRefName":"main","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}\n' "$state";;
  'pr create')
    : > "$dir/created"
    echo 'https://github.com/test/repo/pull/900';;
  'pr close')
    printf '%s\n' close >> "$dir/mutations";;
  'api -X')
    [ "$3" = POST ] || exit 1
    : > "$dir/commented"
    printf '%s\n' comment >> "$dir/mutations";;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
        );
        let mut paths = vec![fake_bin.clone()];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let path = std::env::join_paths(paths).unwrap();

        let (ok, out, err) = run_gg_bounded_with_env(
            &repo,
            &["sync", "--until", "1", "--no-rebase-check"],
            &[
                ("PATH", path.as_os_str()),
                ("GG_REPLACEMENT_RACE", mode.as_ref()),
            ],
        );

        assert!(
            !ok,
            "{mode}: stale old review allowed follow-up: {out}\n{err}"
        );
        assert!(
            format!("{out}{err}").contains("not open at the expected source and target"),
            "{mode}: {out}\n{err}"
        );
        assert_eq!(fs::read_to_string(&views).unwrap().trim(), expected_views);
        let requests = fs::read_to_string(&log).unwrap();
        assert!(requests.contains("pr create --head"), "{mode}: {requests}");
        assert!(!requests.contains(forbidden), "{mode}: {requests}");
        if mode == "before-close" {
            assert!(requests.contains("api -X POST"), "{requests}");
        }
    }
}

#[cfg(unix)]
#[test]
fn replacement_ambiguous_create_recovers_without_duplicate_review() {
    let (_temp, repo, _remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo);
    let config_path = repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["sync_update_descriptions"] = false.into();
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let (ok, out, err) = run_gg(&repo, &["co", "replacement-ambiguous"]);
    assert!(ok, "checkout failed: {out}\n{err}");
    checked_git(
        &repo,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-1111111"],
    );
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["replacement-ambiguous"]["mrs"]["c-1111111"] = 428.into();
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let fake_bin = repo.join("fake-replacement-ambiguous-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s|%s|%s\n' "$1" "${2:-}" "${3:-}" >> "$dir/requests"
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr view')
    case "$3" in
      428) printf '{"number":428,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/428","headRefName":"testuser/old-stack--c-1111111","baseRefName":"main","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}\n';;
      900) printf '{"number":900,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/900","headRefName":"testuser/replacement-ambiguous--c-1111111","baseRefName":"main","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}\n';;
      *) echo "unexpected review $3" >&2; exit 1;;
    esac;;
  'pr create')
    attempts=$(cat "$dir/create-attempts" 2>/dev/null || echo 0)
    echo $((attempts + 1)) > "$dir/create-attempts"
    pending=''
    pending_count=0
    for record in "$TEST_GIT_DIR/gg/operations/"*.json; do
      grep -q '"status": "pending"' "$record" || continue
      pending_count=$((pending_count + 1))
      pending=$(basename "$record" .json)
    done
    [ "$pending_count" -eq 1 ] || exit 1
    printf '%s\n' "$pending" > "$dir/create-operation-id"
    : > "$dir/replacement-exists"
    echo 'simulated lost replacement response' >&2
    exit 1;;
  'pr list')
    [ "$3" = --head ]
    [ "$4" = testuser/replacement-ambiguous--c-1111111 ]
    [ -f "$dir/replacement-exists" ] && echo 900;;
  'pr edit')
    [ "$3" = 900 ]
    printf '%s\n' edit >> "$dir/mutations";;
  'api -X')
    [ "$3" = POST ]
    printf '%s\n' comment >> "$dir/mutations";;
  'pr close')
    [ "$3" = 428 ]
    printf '%s\n' close >> "$dir/mutations";;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let test_git_dir = repo.join(".git");
    let envs = [
        ("PATH", path.as_os_str()),
        ("TEST_GIT_DIR", test_git_dir.as_os_str()),
    ];
    let args = ["sync", "--until", "1", "--no-rebase-check", "--json"];

    let (first_ok, first_out, first_err) = run_gg_bounded_with_env(&repo, &args, &envs);
    assert!(
        !first_ok,
        "ambiguous replacement must fail: {first_out}\n{first_err}"
    );
    let operation_id = fs::read_to_string(fake_bin.join("create-operation-id"))
        .unwrap()
        .trim()
        .to_string();
    let after_first: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        after_first["stacks"]["replacement-ambiguous"]["mrs"]["c-1111111"],
        428
    );
    assert_eq!(
        after_first["stacks"]["replacement-ambiguous"]["review_creation_intents"]["c-1111111"]
            ["origin_operation_id"],
        operation_id
    );
    let origin = operation_record(&repo, &operation_id);
    assert_eq!(origin["status"], "committed");
    assert_eq!(origin["touched_remote"], true);
    assert!(origin["remote_effects"].as_array().unwrap().is_empty());
    let first_requests = fs::read_to_string(fake_bin.join("requests")).unwrap();
    let first_lines = first_requests.lines().collect::<Vec<_>>();
    let create_index = first_lines
        .iter()
        .position(|line| line.starts_with("pr|create|"))
        .unwrap();
    assert_eq!(
        create_index + 1,
        first_lines.len(),
        "post-create request: {first_requests}"
    );
    for forbidden in ["api|", "pr|close|", "pr|comment|"] {
        assert_eq!(
            first_lines
                .iter()
                .filter(|line| line.starts_with(forbidden))
                .count(),
            0,
            "forbidden request after ambiguous replacement: {first_requests}"
        );
    }

    let (second_ok, second_out, second_err) = run_gg_bounded_with_env(&repo, &args, &envs);
    assert!(
        second_ok,
        "replacement recovery failed: {second_out}\n{second_err}"
    );
    assert_eq!(
        fs::read_to_string(fake_bin.join("create-attempts"))
            .unwrap()
            .trim(),
        "1"
    );
    let persisted: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        persisted["stacks"]["replacement-ambiguous"]["mrs"]["c-1111111"],
        900
    );
    assert!(
        persisted["stacks"]["replacement-ambiguous"]["review_creation_intents"]
            .as_object()
            .is_none_or(serde_json::Map::is_empty)
    );
    let reconciled = operation_record(&repo, &operation_id);
    assert_eq!(
        reconciled["remote_effects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|effect| {
                effect["kind"] == "pr_created"
                    && effect["number"] == 900
                    && effect["url"] == "https://github.com/test/repo/pull/900"
            })
            .count(),
        1
    );
    let requests = fs::read_to_string(fake_bin.join("requests")).unwrap();
    let count = |prefix: &str| {
        requests
            .lines()
            .filter(|line| line.starts_with(prefix))
            .count()
    };
    assert_eq!(count("--version|"), 2);
    assert_eq!(count("auth|status|"), 2);
    assert_eq!(count("pr|create|"), 1);
    assert_eq!(count("pr|list|--head"), 1);
    assert_eq!(count("pr|view|428"), 3);
    assert_eq!(count("pr|view|900"), 2);
    assert_eq!(count("pr|edit|900"), 1);
    assert_eq!(count("api|-X|POST"), 0);
    assert_eq!(count("pr|close|428"), 0);
    assert_eq!(
        requests.lines().count(),
        12,
        "uncounted request: {requests}"
    );
    assert_eq!(
        fs::read_to_string(fake_bin.join("mutations")).unwrap(),
        "edit\n"
    );
}

#[cfg(unix)]
#[test]
fn disconnect_before_start_event_finalizes_local_segment() {
    let (_temp, repo, remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo);
    let config_path = repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["provider"] = "github".into();
    config["defaults"]["sync_behind_threshold"] = 0.into();
    config["defaults"]["github"]["stacks_integration"] = "off".into();
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let (ok, out, err) = run_gg(&repo, &["co", "pre-start-disconnect"]);
    assert!(ok, "checkout failed: {out}\n{err}");
    checked_git(
        &repo,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7100005"],
    );
    let fake_bin = repo.join("pre-start-disconnect-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') touch "$(dirname "$0")/ready";;
  'pr create') touch "$(dirname "$0")/remote-mutation"; echo 'https://github.com/test/repo/pull/714';;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let operation_ids_before = operation_record_ids(&repo);
    let (read_stream, write_stream) = UnixStream::pair().unwrap();
    drop(read_stream);
    let stdout_fd: std::os::fd::OwnedFd = write_stream.into();
    let stderr_path = repo.join(".git/pre-start-disconnect-stderr");
    let mut command = Command::new(env!("CARGO_BIN_EXE_gg"));
    command
        .args(["sync", "--until", "1", "--no-rebase-check", "--jsonl"])
        .current_dir(&repo)
        .env("PATH", path)
        .stdout(Stdio::from(stdout_fd))
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
    let mut child = ProcessGroupGuard::spawn(&mut command).expect("start sync");
    let readiness_deadline = Instant::now() + Duration::from_secs(10);
    while !fake_bin.join("ready").exists() {
        if Instant::now() >= readiness_deadline {
            panic!("sync did not complete provider readiness");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let process_deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= process_deadline {
            panic!("sync did not stop after pre-start disconnect");
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(!status.success(), "closed consumer must abort sync");
    let operation_ids = operation_record_ids(&repo)
        .difference(&operation_ids_before)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(operation_ids.len(), 1, "{operation_ids:?}");
    let operation_id = &operation_ids[0];
    let record = operation_record(&repo, operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], false, "{record}");
    assert_eq!(record["remote_effects"], serde_json::json!([]), "{record}");
    assert!(!fake_bin.join("remote-mutation").exists());
    assert!(
        !run_git(
            &remote,
            &[
                "show-ref",
                "--verify",
                "refs/heads/testuser/pre-start-disconnect--c-7100005"
            ]
        )
        .0,
        "entry branch was pushed after disconnect"
    );
}

#[cfg(unix)]
#[test]
fn disconnect_before_first_remote_mutation_finalizes_local_segment() {
    let (_temp, repo, remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo);
    let config_path = repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["provider"] = "github".into();
    config["defaults"]["sync_behind_threshold"] = 0.into();
    config["defaults"]["github"]["stacks_integration"] = "off".into();
    config["defaults"]["lint"] = serde_json::json!(["./block-lint.sh"]);
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let (ok, out, err) = run_gg(&repo, &["co", "pre-mutation-disconnect"]);
    assert!(ok, "checkout failed: {out}\n{err}");
    checked_git(
        &repo,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7100004"],
    );
    write_executable(
        &repo.join("block-lint.sh"),
        r#"#!/bin/sh
set -eu
operations=$(git rev-parse --absolute-git-dir)/gg/operations
pending=''
for record in "$operations/"*.json; do
  grep -q '"status": "pending"' "$record" || continue
  [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
  pending=$(basename "$record" .json)
done
[ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
printf '%s\n' "$pending" > .git/pre-mutation-operation-id
touch .git/pre-mutation-lint-started
while [ ! -f .git/release-pre-mutation-lint ]; do sleep 0.01; done
"#,
    );
    let fake_bin = repo.join("pre-mutation-disconnect-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr create') touch "$(dirname "$0")/remote-mutation"; echo 'https://github.com/test/repo/pull/713';;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();

    let (read_stream, write_stream) = UnixStream::pair().unwrap();
    read_stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let stdout_fd: std::os::fd::OwnedFd = write_stream.into();
    let stderr_path = repo.join(".git/pre-mutation-disconnect-stderr");
    let mut command = Command::new(env!("CARGO_BIN_EXE_gg"));
    command
        .args([
            "sync",
            "--lint",
            "--until",
            "1",
            "--no-rebase-check",
            "--jsonl",
        ])
        .current_dir(&repo)
        .env("PATH", path)
        .stdout(Stdio::from(stdout_fd))
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
    let mut child = ProcessGroupGuard::spawn(&mut command).expect("start sync");
    let mut reader = BufReader::new(read_stream);
    let mut start = String::new();
    reader.read_line(&mut start).expect("read start event");
    assert_eq!(
        serde_json::from_str::<Value>(&start).unwrap()["event"],
        "start"
    );
    let readiness = repo.join(".git/pre-mutation-lint-started");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !readiness.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "sync exited before lint readiness ({status}): {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        if Instant::now() >= deadline {
            let _ = fs::write(repo.join(".git/release-pre-mutation-lint"), "release");
            panic!("sync did not reach pre-mutation lint readiness");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let operation_id = fs::read_to_string(repo.join(".git/pre-mutation-operation-id"))
        .unwrap()
        .trim()
        .to_string();
    drop(reader);
    fs::write(repo.join(".git/release-pre-mutation-lint"), "release").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            panic!("sync did not stop after pre-mutation disconnect");
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(!status.success(), "closed consumer must abort sync");
    let record = operation_record(&repo, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], false, "{record}");
    assert_eq!(record["remote_effects"], serde_json::json!([]), "{record}");
    assert!(!fake_bin.join("remote-mutation").exists());
    assert!(
        !run_git(
            &remote,
            &[
                "show-ref",
                "--verify",
                "refs/heads/testuser/pre-mutation-disconnect--c-7100004"
            ]
        )
        .0,
        "entry branch was pushed after disconnect"
    );
}

#[cfg(unix)]
#[test]
fn mutation_and_finalization_failures_are_reported_together() {
    let (_temp, repo, _remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo);
    let config_path = repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["provider"] = "github".into();
    config["defaults"]["sync_behind_threshold"] = 0.into();
    config["defaults"]["github"]["stacks_integration"] = "off".into();
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let (ok, out, err) = run_gg(&repo, &["co", "double-failure"]);
    assert!(ok, "checkout failed: {out}\n{err}");
    checked_git(
        &repo,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7100003"],
    );

    let fake_bin = repo.join("double-failure-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr create')
    pending=''
    for record in "$TEST_GIT_DIR/gg/operations/"*.json; do
      grep -q '"status": "pending"' "$record" || continue
      [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
      pending=$(basename "$record" .json)
    done
    [ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
    printf '%s\n' "$pending" > "$dir/operation-id"
    mkdir "$TEST_GIT_DIR/gg/operations/$pending.json.tmp"
    echo 'simulated create mutation failure' >&2
    exit 1;;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();

    let (ok, stdout, stderr) = run_gg_bounded_with_env(
        &repo,
        &["sync", "--until", "1", "--no-rebase-check", "--jsonl"],
        &[
            ("PATH", path.as_os_str()),
            ("TEST_GIT_DIR", repo.join(".git").as_os_str()),
        ],
    );
    let operation_id = fs::read_to_string(fake_bin.join("operation-id"))
        .unwrap()
        .trim()
        .to_string();
    fs::remove_dir(
        repo.join(".git/gg/operations")
            .join(format!("{operation_id}.json.tmp")),
    )
    .unwrap();

    assert!(!ok, "double failure must fail: {stdout}\n{stderr}");
    let diagnostic = format!("{stdout}\n{stderr}");
    assert!(
        diagnostic.contains("simulated create mutation failure"),
        "mutation error was lost: {diagnostic}"
    );
    assert!(
        diagnostic.contains("additionally failed to finalize sync operation"),
        "finalization error was lost: {diagnostic}"
    );
    let record = operation_record(&repo, &operation_id);
    assert_eq!(record["status"], "pending", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
}

#[cfg(unix)]
#[test]
fn ordinary_create_config_save_failure_preserves_known_result() {
    let (_temp, repo, _remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo);
    let config_path = repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["provider"] = "github".into();
    config["defaults"]["sync_behind_threshold"] = 0.into();
    config["defaults"]["github"]["stacks_integration"] = "off".into();
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let (ok, out, err) = run_gg(&repo, &["co", "ordinary-create-save-failure"]);
    assert!(ok, "checkout failed: {out}\n{err}");
    checked_git(
        &repo,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7100001"],
    );

    let fake_bin = repo.join("ordinary-create-save-failure-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s\n' "$*" >> "$dir/requests"
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr create')
    pending=''
    for record in "$TEST_GIT_DIR/gg/operations/"*.json; do
      grep -q '"status": "pending"' "$record" || continue
      [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
      pending=$(basename "$record" .json)
    done
    [ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
    printf '%s\n' "$pending" > "$dir/operation-id"
    mkdir "$TEST_GIT_DIR/gg/config.tmp"
    echo 1 > "$dir/create-count"
    echo 'https://github.com/test/repo/pull/711';;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();

    let (ok, stdout, stderr) = run_gg_bounded_with_env(
        &repo,
        &["sync", "--until", "1", "--no-rebase-check", "--jsonl"],
        &[
            ("PATH", path.as_os_str()),
            ("TEST_GIT_DIR", repo.join(".git").as_os_str()),
        ],
    );
    fs::remove_dir(repo.join(".git/gg/config.tmp")).unwrap();

    assert!(!ok, "blocked config save must fail: {stdout}\n{stderr}");
    let operation_id = fs::read_to_string(fake_bin.join("operation-id"))
        .unwrap()
        .trim()
        .to_string();
    let record = operation_record(&repo, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
    let effects = record["remote_effects"].as_array().unwrap();
    assert_eq!(effects.len(), 1, "{record}");
    assert_eq!(effects[0]["kind"], "pr_created", "{record}");
    assert_eq!(effects[0]["number"], 711, "{record}");
    assert_eq!(
        effects[0]["url"], "https://github.com/test/repo/pull/711",
        "{record}"
    );
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(events.iter().all(|event| event["event"] != "pr_created"));
    assert!(events.iter().all(|event| event["event"] != "summary"));
    assert_eq!(
        fs::read_to_string(fake_bin.join("create-count")).unwrap(),
        "1\n"
    );
    let requests = fs::read_to_string(fake_bin.join("requests")).unwrap();
    assert_eq!(
        requests
            .lines()
            .filter(|request| request.starts_with("pr create "))
            .count(),
        1,
        "{requests}"
    );
    assert!(
        requests.lines().all(|line| !line.starts_with("pr edit ")
            && !line.starts_with("pr close ")
            && !line.starts_with("api ")),
        "follow-up mutation escaped after config failure: {requests}"
    );
}

#[cfg(unix)]
#[test]
fn replacement_create_config_save_failure_preserves_known_result() {
    let (_temp, repo, _remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo);
    let config_path = repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["provider"] = "github".into();
    config["defaults"]["sync_behind_threshold"] = 0.into();
    config["defaults"]["sync_update_descriptions"] = false.into();
    config["defaults"]["github"]["stacks_integration"] = "off".into();
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let (ok, out, err) = run_gg(&repo, &["co", "replacement-create-save-failure"]);
    assert!(ok, "checkout failed: {out}\n{err}");
    checked_git(
        &repo,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7100002"],
    );
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["replacement-create-save-failure"]["mrs"]["c-7100002"] = 428.into();
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();

    let fake_bin = repo.join("replacement-create-save-failure-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s\n' "$*" >> "$dir/requests"
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr view')
    echo '{"number":428,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/428","headRefName":"testuser/old-stack--c-7100002","baseRefName":"main","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
    ;;
  'pr create')
    pending=''
    for record in "$TEST_GIT_DIR/gg/operations/"*.json; do
      grep -q '"status": "pending"' "$record" || continue
      [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
      pending=$(basename "$record" .json)
    done
    [ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
    printf '%s\n' "$pending" > "$dir/operation-id"
    mkdir "$TEST_GIT_DIR/gg/config.tmp"
    echo 1 > "$dir/create-count"
    echo 'https://github.com/test/repo/pull/712';;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();

    let (ok, stdout, stderr) = run_gg_bounded_with_env(
        &repo,
        &["sync", "--until", "1", "--no-rebase-check", "--jsonl"],
        &[
            ("PATH", path.as_os_str()),
            ("TEST_GIT_DIR", repo.join(".git").as_os_str()),
        ],
    );
    fs::remove_dir(repo.join(".git/gg/config.tmp")).unwrap();

    assert!(!ok, "blocked config save must fail: {stdout}\n{stderr}");
    let operation_id = fs::read_to_string(fake_bin.join("operation-id"))
        .unwrap()
        .trim()
        .to_string();
    let record = operation_record(&repo, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
    let effects = record["remote_effects"].as_array().unwrap();
    assert_eq!(effects.len(), 1, "{record}");
    assert_eq!(effects[0]["kind"], "pr_created", "{record}");
    assert_eq!(effects[0]["number"], 712, "{record}");
    assert_eq!(
        effects[0]["url"], "https://github.com/test/repo/pull/712",
        "{record}"
    );
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(events.iter().all(|event| event["event"] != "pr_created"));
    assert!(events.iter().all(|event| event["event"] != "summary"));
    assert_eq!(
        fs::read_to_string(fake_bin.join("create-count")).unwrap(),
        "1\n"
    );
    let requests = fs::read_to_string(fake_bin.join("requests")).unwrap();
    assert_eq!(
        requests
            .lines()
            .filter(|request| request.starts_with("pr create "))
            .count(),
        1,
        "{requests}"
    );
    assert!(
        requests
            .lines()
            .all(|line| !line.starts_with("api ") && !line.starts_with("pr close ")),
        "replacement follow-up escaped after config failure: {requests}"
    );
}

#[cfg(unix)]
#[test]
fn replacement_mapping_survives_post_create_authorization_failure() {
    let (_temp, repo, _remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo);
    let config_path = repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["provider"] = "github".into();
    config["defaults"]["sync_behind_threshold"] = 0.into();
    config["defaults"]["sync_update_descriptions"] = false.into();
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();

    let (ok, out, err) = run_gg(&repo, &["co", "replacement-persistence"]);
    assert!(ok, "checkout failed: {out}\n{err}");
    fs::write(repo.join("entry"), "entry").unwrap();
    checked_git(&repo, &["add", "entry"]);
    checked_git(&repo, &["commit", "-m", "Entry\n\nGG-ID: c-1111111"]);
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["replacement-persistence"]["mrs"]["c-1111111"] = 428.into();
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();

    let fake_bin = repo.join("fake-replacement-persistence-bin");
    fs::create_dir(&fake_bin).unwrap();
    let log = fake_bin.join("requests");
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s\n' "$*" >> "$dir/requests"
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr view')
    number=$3
    if [ "$number" = 428 ]; then
      if [ -f "$dir/created" ] && [ ! -f "$dir/transient-cleared" ]; then
        echo 'simulated transient authorization read failure' >&2
        exit 1
      fi
      echo '{"number":428,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/428","headRefName":"testuser/old-stack--c-1111111","baseRefName":"main","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
    else
      printf '{"number":%s,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/%s","headRefName":"testuser/replacement-persistence--c-1111111","baseRefName":"main","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}\n' "$number" "$number"
    fi;;
  'pr create')
    count=$(cat "$dir/create-count" 2>/dev/null || echo 0)
    number=$((900 + count))
    printf '%s\n' $((count + 1)) > "$dir/create-count"
    : > "$dir/created"
    printf 'https://github.com/test/repo/pull/%s\n' "$number";;
  'pr edit') exit 0;;
  'pr close') exit 0;;
  'api -X') [ "$3" = POST ] || exit 1;;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let args = ["sync", "--until", "1", "--no-rebase-check"];

    let (ok, out, err) = run_gg_bounded_with_env(&repo, &args, &[("PATH", path.as_os_str())]);
    assert!(!ok, "transient authorization read must fail: {out}\n{err}");
    let persisted_after_failure: Value =
        serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();

    fs::write(fake_bin.join("transient-cleared"), "").unwrap();
    let (ok, out, err) = run_gg_bounded_with_env(&repo, &args, &[("PATH", path.as_os_str())]);
    assert!(ok, "retry failed: {out}\n{err}");

    assert_eq!(
        persisted_after_failure["stacks"]["replacement-persistence"]["mrs"]["c-1111111"], 900,
        "known-success replacement mapping must survive the later authorization failure"
    );
    let persisted_after_retry: Value =
        serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        persisted_after_retry["stacks"]["replacement-persistence"]["mrs"]["c-1111111"], 900,
        "retry must reuse the persisted replacement"
    );
    let requests = fs::read_to_string(&log).unwrap();
    assert_eq!(
        requests
            .lines()
            .filter(|request| request.starts_with("pr create "))
            .count(),
        1,
        "retry must not create a duplicate replacement: {requests}"
    );
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
      echo '{"number":428,"title":"Entry A","state":"OPEN","url":"https://github.com/test/repo/pull/428","headRefName":"testuser/u312b203606--c-8b999da","baseRefName":"main","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
      exit 0
      ;;
    429)
      echo '{"number":429,"title":"Entry B","state":"OPEN","url":"https://github.com/test/repo/pull/429","headRefName":"testuser/u312b203606--c-fa7d2e9","baseRefName":"testuser/u312b203606--c-8b999da","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
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
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let fake_path = install_fake_gh(&repo_path);
    let gh_log = repo_path.join("fake-bin/gh-invocations.log");

    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "rebase-detection-test"]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");

    // Leave metadata normalization work for sync so the old behavior advances
    // beyond provider preflight and opens a segment before noticing the rebase.
    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file");
    checked_git(&repo_path, &["add", "file1.txt"]);
    checked_git(&repo_path, &["commit", "-m", "Commit 1"]);

    let rebase_dir = repo_path.join(".git/rebase-merge");
    fs::create_dir_all(&rebase_dir).expect("Failed to create rebase-merge dir");
    fs::write(
        rebase_dir.join("head-name"),
        "refs/heads/testuser/rebase-detection-test",
    )
    .expect("Failed to write head-name");
    let marker_path = repo_path.join(".git/gg/interrupted-rebase-operation.json");
    let marker_bytes = br#"{
  "operation_id": "op_0000000000000_preexistingowner",
  "sentinel": "preserve these exact bytes"
}
"#;
    fs::write(&marker_path, marker_bytes).expect("write existing ownership marker");
    let operations_before = operation_record_ids(&repo_path);
    // A later fetch failure exercises the old broad `finish_on_error` path:
    // it must not turn a rebase that predates sync into sync-owned state.
    let missing_origin = repo_path.join("missing-origin.git");
    checked_git(
        &repo_path,
        &[
            "remote",
            "set-url",
            "origin",
            missing_origin.to_str().expect("UTF-8 test path"),
        ],
    );

    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", gh_log.as_os_str()),
        ],
    );

    assert!(!success, "sync should fail when a rebase is in progress");
    assert_eq!(
        fs::read(&marker_path).expect("read existing ownership marker"),
        marker_bytes,
        "sync must not claim a rebase that was already active"
    );
    assert_eq!(
        operation_record_ids(&repo_path),
        operations_before,
        "rejecting a pre-existing rebase must not open a sync operation"
    );
    assert!(
        !gh_log.exists(),
        "rebase rejection must happen before provider preflight: {}",
        fs::read_to_string(&gh_log).unwrap_or_default()
    );

    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("A rebase is in progress"),
        "error must identify the active rebase: {combined}"
    );
    assert!(
        combined.contains("gg continue") && combined.contains("gg abort"),
        "error must explain how to resolve the active rebase: {combined}"
    );
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

#[cfg(unix)]
fn setup_empty_stack_with_ambiguous_creation(
    stack_name: &str,
    gg_id: &str,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    std::ffi::OsString,
    String,
) {
    let (temp, repo_path, remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let (ok, out, err) = run_gg(&repo_path, &["co", stack_name]);
    assert!(ok, "{out}\n{err}");
    checked_git(
        &repo_path,
        &[
            "commit",
            "--allow-empty",
            "-m",
            &format!("Entry\n\nGG-ID: {gg_id}"),
        ],
    );
    let branch = format!("testuser/{stack_name}--{gg_id}");
    let fake_bin = repo_path.join("fake-empty-intent-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s %s\n' "$1" "${2:-}" >> "$dir/requests"
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr create')
    attempts=$(cat "$dir/create-attempts" 2>/dev/null || echo 0)
    echo $((attempts + 1)) > "$dir/create-attempts"
    echo 'simulated lost create response' >&2
    exit 1;;
  'pr list')
    echo 'simulated authoritative recovery failure' >&2
    exit 1;;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let mut paths = vec![fake_bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let (ok, out, err) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check", "--json"],
        &[("PATH", path.as_os_str())],
    );
    assert!(!ok, "ambiguous create must fail: {out}\n{err}");
    let expected_oid = checked_git(&repo_path, &["rev-parse", &branch]);
    checked_git(&repo_path, &["reset", "--hard", "main"]);
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["pending_remote_branch_deletions"] = serde_json::json!([{
        "remote": "origin",
        "branch": branch,
        "expected_oid": expected_oid,
        "source_operation_id": "op_test_pending_delete",
        "state": "pending"
    }]);
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    (temp, repo_path, remote_path, path, branch)
}

#[cfg(unix)]
#[test]
fn empty_stack_creation_recovery_failure_precedes_pending_remote_prune() {
    let (_temp, repo_path, remote_path, path, branch) =
        setup_empty_stack_with_ambiguous_creation("empty-intent-prune", "c-7654324");
    let config_path = repo_path.join(".git/gg/config.json");
    let before: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        before["pending_remote_branch_deletions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let intent = &before["stacks"]["empty-intent-prune"]["review_creation_intents"]["c-7654324"];
    let origin_id = intent["origin_operation_id"].as_str().unwrap();
    let operations_before = operation_record_ids(&repo_path);

    let (ok, out, err) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check", "--json"],
        &[("PATH", path.as_os_str())],
    );

    assert!(!ok, "recovery failure must abort empty sync: {out}\n{err}");
    assert_eq!(operation_record_ids(&repo_path), operations_before);
    let after: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        after["pending_remote_branch_deletions"],
        before["pending_remote_branch_deletions"]
    );
    assert_eq!(
        after["stacks"]["empty-intent-prune"]["review_creation_intents"]["c-7654324"]
            ["origin_operation_id"],
        origin_id
    );
    assert!(
        run_git(
            &remote_path,
            &["show-ref", "--verify", &format!("refs/heads/{branch}")]
        )
        .0,
        "recovery failure must not prune the pending branch"
    );
    let requests = fs::read_to_string(repo_path.join("fake-empty-intent-bin/requests")).unwrap();
    assert_eq!(
        requests
            .lines()
            .filter(|line| line.starts_with("pr create"))
            .count(),
        1
    );
    assert_eq!(
        requests
            .lines()
            .filter(|line| line.starts_with("pr list"))
            .count(),
        1
    );
    assert_eq!(
        requests
            .lines()
            .filter(|line| line.starts_with("pr view"))
            .count(),
        0
    );
    assert!(
        requests.lines().all(|line| {
            line.starts_with("--version")
                || line.starts_with("auth status")
                || line.starts_with("pr create")
                || line.starts_with("pr list")
        }),
        "uncounted provider request:\n{requests}"
    );
}

#[cfg(unix)]
#[test]
fn empty_stack_creation_intent_without_deletion_requires_recovery() {
    let (_temp, repo_path, _remote_path, path, _branch) =
        setup_empty_stack_with_ambiguous_creation("empty-intent-only", "c-7654325");
    let config_path = repo_path.join(".git/gg/config.json");
    let mut before: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    before["pending_remote_branch_deletions"] = serde_json::json!([]);
    fs::write(&config_path, serde_json::to_vec_pretty(&before).unwrap()).unwrap();
    let origin_id = before["stacks"]["empty-intent-only"]["review_creation_intents"]["c-7654325"]
        ["origin_operation_id"]
        .as_str()
        .unwrap()
        .to_string();
    let operations_before = operation_record_ids(&repo_path);

    let (ok, out, err) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check", "--json"],
        &[("PATH", path.as_os_str())],
    );

    assert!(
        !ok,
        "unresolved intent must not silently succeed: {out}\n{err}"
    );
    assert_eq!(operation_record_ids(&repo_path), operations_before);
    let after: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        after["stacks"]["empty-intent-only"]["review_creation_intents"]["c-7654325"]
            ["origin_operation_id"],
        origin_id
    );
    assert!(after["pending_remote_branch_deletions"]
        .as_array()
        .unwrap()
        .is_empty());
    let requests = fs::read_to_string(repo_path.join("fake-empty-intent-bin/requests")).unwrap();
    assert_eq!(
        requests
            .lines()
            .filter(|line| line.starts_with("pr create"))
            .count(),
        1
    );
    assert_eq!(
        requests
            .lines()
            .filter(|line| line.starts_with("pr list"))
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn sync_recovers_lost_create_response_without_duplicate_review() {
    let (_temp, repo_path, _remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["sync_update_descriptions"] = false.into();
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let (ok, out, err) = run_gg(&repo_path, &["co", "create-intent-lost"]);
    assert!(ok, "{out}\n{err}");
    checked_git(
        &repo_path,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7654321"],
    );
    let branch = "testuser/create-intent-lost--c-7654321";
    let fake_bin = repo_path.join("fake-create-intent-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        &format!(
            r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s\n' "$*" >> "$dir/requests"
case "$1 ${{2:-}}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr create')
    attempts=$(cat "$dir/create-attempts" 2>/dev/null || echo 0)
    echo $((attempts + 1)) > "$dir/create-attempts"
    touch "$dir/review-exists"
    echo 'simulated lost create response' >&2
    exit 1;;
  'pr list')
    if [ -f "$dir/lookup-fail" ]; then
      echo 'simulated review lookup failure' >&2
      exit 1
    fi
    [ "$3" = --head ] && [ "$4" = '{branch}' ] && [ -f "$dir/review-exists" ] && echo 101
    ;;
  'pr view')
    echo '{{"number":101,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/101","headRefName":"{branch}","baseRefName":"main","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}}'
    ;;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#
        ),
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let args = ["sync", "--until", "1", "--rebase-if-needed", "--json"];

    let (first_ok, first_out, first_err) =
        run_gg_bounded_with_env(&repo_path, &args, &[("PATH", path.as_os_str())]);
    assert!(
        !first_ok,
        "ambiguous create must fail: {first_out}\n{first_err}"
    );
    let after_first: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    let intent =
        &after_first["stacks"]["create-intent-lost"]["review_creation_intents"]["c-7654321"];
    assert_eq!(intent["source_branch"], branch);
    assert_eq!(intent["target_branch"], "main");
    let origin_id = intent["origin_operation_id"]
        .as_str()
        .expect("intent has required origin operation id")
        .to_string();

    let operation_ids_before_recovery = operation_record_ids(&repo_path);
    fs::write(fake_bin.join("lookup-fail"), "fail").unwrap();
    let (lookup_ok, lookup_out, lookup_err) =
        run_gg_bounded_with_env(&repo_path, &args, &[("PATH", path.as_os_str())]);
    assert!(
        !lookup_ok,
        "lookup failure must block recovery: {lookup_out}\n{lookup_err}"
    );
    assert_eq!(
        operation_record_ids(&repo_path),
        operation_ids_before_recovery,
        "lookup failure must not open a successor sync segment"
    );
    let lookup_blocked: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        lookup_blocked["stacks"]["create-intent-lost"]["review_creation_intents"]["c-7654321"]
            ["origin_operation_id"],
        origin_id
    );
    fs::remove_file(fake_bin.join("lookup-fail")).unwrap();

    let origin_tmp = repo_path
        .join(".git/gg/operations")
        .join(format!("{origin_id}.json.tmp"));
    fs::create_dir(&origin_tmp).unwrap();
    let (blocked_ok, blocked_out, blocked_err) =
        run_gg_bounded_with_env(&repo_path, &args, &[("PATH", path.as_os_str())]);
    assert!(
        !blocked_ok,
        "origin save failure must block recovery: {blocked_out}\n{blocked_err}"
    );
    assert_eq!(
        operation_record_ids(&repo_path),
        operation_ids_before_recovery,
        "failed origin reconciliation must not open a successor sync segment"
    );
    let blocked_config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        blocked_config["stacks"]["create-intent-lost"]["review_creation_intents"]["c-7654321"]
            ["origin_operation_id"],
        origin_id
    );
    fs::remove_dir(&origin_tmp).unwrap();

    let (second_ok, second_out, second_err) =
        run_gg_bounded_with_env(&repo_path, &args, &[("PATH", path.as_os_str())]);
    assert!(second_ok, "{second_out}\n{second_err}");
    let second: Value = serde_json::from_str(&second_out).unwrap();
    assert_eq!(second["sync"]["entries"][0]["pr_number"], 101);
    assert_eq!(
        fs::read_to_string(fake_bin.join("create-attempts"))
            .unwrap()
            .trim(),
        "1",
        "an ambiguous create must never be retried"
    );
    let requests = fs::read_to_string(fake_bin.join("requests")).unwrap();
    assert_eq!(
        requests
            .lines()
            .filter(|line| line.starts_with("pr list --head"))
            .count(),
        3,
        "the lookup failure, blocked reconciliation, and retry each perform one source discovery"
    );
    assert_eq!(
        requests
            .lines()
            .filter(|line| line.starts_with("pr create"))
            .count(),
        1
    );
    let persisted: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        persisted["stacks"]["create-intent-lost"]["mrs"]["c-7654321"],
        101
    );
    assert!(
        persisted["stacks"]["create-intent-lost"]["review_creation_intents"]
            .as_object()
            .is_none_or(serde_json::Map::is_empty)
    );
    let origin = operation_record(&repo_path, &origin_id);
    assert_eq!(origin["status"], "committed");
    assert_eq!(origin["touched_remote"], true);
    assert_eq!(
        origin["remote_effects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|effect| effect["kind"] == "pr_created" && effect["number"] == 101)
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn sync_origin_load_failure_retains_intent_without_lookup_create_or_successor_segment() {
    let (_temp, repo_path, _remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let (ok, out, err) = run_gg(&repo_path, &["co", "create-intent-missing-origin"]);
    assert!(ok, "{out}\n{err}");
    checked_git(
        &repo_path,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7654323"],
    );
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["create-intent-missing-origin"]["review_creation_intents"] = serde_json::json!({
        "c-7654323": {
            "source_branch": "testuser/create-intent-missing-origin--c-7654323",
            "target_branch": "main",
            "origin_operation_id": "op_0000000000999_missing"
        }
    });
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let fake_bin = repo_path.join("fake-create-missing-origin-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s\n' "$*" >> "$dir/requests"
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  *) echo "unexpected provider access after missing origin: gh $*" >&2; exit 1;;
esac
"#,
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let operation_ids_before = operation_record_ids(&repo_path);

    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--until", "1", "--rebase-if-needed", "--json"],
        &[("PATH", path.as_os_str())],
    );
    assert!(!success, "missing origin must fail: {stdout}\n{stderr}");
    assert_eq!(operation_record_ids(&repo_path), operation_ids_before);
    let requests = fs::read_to_string(fake_bin.join("requests")).unwrap();
    assert!(!requests.lines().any(|line| line.starts_with("pr list")));
    assert!(!requests.lines().any(|line| line.starts_with("pr create")));
    let persisted: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        persisted["stacks"]["create-intent-missing-origin"]["review_creation_intents"]["c-7654323"]
            ["origin_operation_id"],
        "op_0000000000999_missing"
    );
}

#[cfg(unix)]
#[test]
fn sync_recovers_post_create_config_failure_without_duplicate_review_or_effect() {
    let (_temp, repo_path, _remote) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["sync_update_descriptions"] = false.into();
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let (ok, out, err) = run_gg(&repo_path, &["co", "create-intent-config"]);
    assert!(ok, "{out}\n{err}");
    checked_git(
        &repo_path,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7654322"],
    );
    let branch = "testuser/create-intent-config--c-7654322";
    let fake_bin = repo_path.join("fake-create-config-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        &format!(
            r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s %s\n' "$1" "${{2:-}}" >> "$dir/requests"
case "$1 ${{2:-}}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr create')
    attempts=$(cat "$dir/create-attempts" 2>/dev/null || echo 0)
    echo $((attempts + 1)) > "$dir/create-attempts"
    touch "$dir/review-exists"
    echo 'simulated lost create response' >&2
    exit 1;;
  'pr list')
    [ "$3" = --head ] && [ "$4" = '{branch}' ] && [ -f "$dir/review-exists" ] && echo 102
    ;;
  'pr view')
    if [ -f "$dir/block-config-on-view" ]; then
      rm "$dir/block-config-on-view"
      mkdir "$GG_CONFIG_TMP"
    fi
    echo '{{"number":102,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/102","headRefName":"{branch}","baseRefName":"main","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}}'
    ;;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#
        ),
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();
    let config_tmp = repo_path.join(".git/gg/config.tmp");
    let args = ["sync", "--until", "1", "--rebase-if-needed", "--json"];
    let envs = [
        ("PATH", path.as_os_str()),
        ("GG_CONFIG_TMP", config_tmp.as_os_str()),
    ];

    let (first_ok, first_out, first_err) = run_gg_bounded_with_env(&repo_path, &args, &envs);
    assert!(
        !first_ok,
        "ambiguous create must fail: {first_out}\n{first_err}"
    );
    let after_first: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    let intent =
        &after_first["stacks"]["create-intent-config"]["review_creation_intents"]["c-7654322"];
    assert_eq!(intent["source_branch"], branch);
    let origin_id = intent["origin_operation_id"]
        .as_str()
        .expect("intent has required origin operation id")
        .to_string();
    let operations_after_ambiguous = operation_record_ids(&repo_path);
    assert!(
        operations_after_ambiguous.contains(&origin_id),
        "captured origin must be persisted"
    );

    fs::write(fake_bin.join("block-config-on-view"), "block").unwrap();
    let (recovery_ok, recovery_out, recovery_err) =
        run_gg_bounded_with_env(&repo_path, &args, &envs);
    assert!(
        !recovery_ok,
        "recovery-time config save must fail: {recovery_out}\n{recovery_err}"
    );
    let recovery_config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert!(recovery_config["stacks"]["create-intent-config"]["mrs"]["c-7654322"].is_null());
    assert_eq!(
        recovery_config["stacks"]["create-intent-config"]["review_creation_intents"]["c-7654322"]
            ["origin_operation_id"],
        origin_id
    );
    assert_eq!(
        operation_record_ids(&repo_path),
        operations_after_ambiguous,
        "failed recovery config save must not open a successor sync segment"
    );
    let recovered_origin = operation_record(&repo_path, &origin_id);
    assert_eq!(recovered_origin["status"], "committed");
    assert_eq!(recovered_origin["touched_remote"], true);
    assert_eq!(
        recovered_origin["remote_effects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|effect| {
                effect["kind"] == "pr_created"
                    && effect["number"] == 102
                    && effect["url"] == "https://github.com/test/repo/pull/102"
            })
            .count(),
        1
    );
    let requests_after_recovery = fs::read_to_string(fake_bin.join("requests")).unwrap();
    assert_eq!(
        requests_after_recovery
            .lines()
            .filter(|line| line.starts_with("pr create"))
            .count(),
        1
    );
    assert_eq!(
        requests_after_recovery
            .lines()
            .filter(|line| line.starts_with("pr list"))
            .count(),
        1
    );
    assert_eq!(
        requests_after_recovery
            .lines()
            .filter(|line| line.starts_with("pr view"))
            .count(),
        1
    );
    assert!(requests_after_recovery.lines().all(|line| {
        line.starts_with("--version")
            || line.starts_with("auth status")
            || line.starts_with("pr create")
            || line.starts_with("pr list")
            || line.starts_with("pr view")
    }));
    fs::remove_dir(&config_tmp).unwrap();

    let (second_ok, second_out, second_err) = run_gg_bounded_with_env(&repo_path, &args, &envs);
    assert!(second_ok, "{second_out}\n{second_err}");
    assert_eq!(
        fs::read_to_string(fake_bin.join("create-attempts"))
            .unwrap()
            .trim(),
        "1"
    );
    let persisted: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        persisted["stacks"]["create-intent-config"]["mrs"]["c-7654322"],
        102
    );
    let origin = operation_record(&repo_path, &origin_id);
    assert_eq!(origin["status"], "committed");
    assert_eq!(
        origin["remote_effects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|effect| effect["kind"] == "pr_created" && effect["number"] == 102)
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn jsonl_disconnect_during_failed_create_stops_before_remote_prune() {
    let (_temp, repo_path, remote_path, fake_path) =
        setup_published_prune_stack("failed-create-disconnect");
    let dropped_branch = "testuser/failed-create-disconnect--c-3333333";
    let (ok, out, err) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(ok, "{out}\n{err}");
    clear_stack_mappings(&repo_path, "failed-create-disconnect");
    let fake_bin = repo_path.join("fake-bin");
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s %s\n' "$1" "${2:-}" >> "$dir/failed-create-requests"
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr create')
    attempts=$(cat "$dir/create-attempts" 2>/dev/null || echo 0)
    echo $((attempts + 1)) > "$dir/create-attempts"
    pending=''
    pending_count=0
    for record in "$TEST_GIT_DIR/gg/operations/"*.json; do
      grep -q '"status": "pending"' "$record" || continue
      pending_count=$((pending_count + 1))
      pending=$(basename "$record" .json)
    done
    [ "$pending_count" -eq 1 ] || {
      echo "expected exactly one pending operation, found $pending_count" >&2
      exit 1
    }
    printf '%s\n' "$pending" > "$dir/create-operation-id"
    touch "$dir/create-started"
    while [ ! -f "$dir/release-create" ]; do sleep 0.01; done
    echo 'simulated failed create' >&2
    exit 1;;
  'pr list') exit 0;;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let real_git = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    write_executable(
        &fake_bin.join("git"),
        &format!(
            r#"#!/bin/sh
set -eu
case " $* " in
  *" :refs/heads/{dropped_branch} "*) echo delete >> "$(dirname "$0")/delete-attempts";;
esac
exec "$REAL_GIT" "$@"
"#
        ),
    );

    let (read_stream, write_stream) = UnixStream::pair().unwrap();
    read_stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let stdout_fd: std::os::fd::OwnedFd = write_stream.into();
    let stderr_path = repo_path.join(".git/failed-create-disconnect-stderr");
    let mut command = Command::new(env!("CARGO_BIN_EXE_gg"));
    command
        .args(["sync", "--until", "1", "--no-rebase-check", "--jsonl"])
        .current_dir(&repo_path)
        .env("PATH", &fake_path)
        .env("REAL_GIT", real_git.trim())
        .env("TEST_GIT_DIR", repo_path.join(".git"))
        .stdout(Stdio::from(stdout_fd))
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
    let mut child = ProcessGroupGuard::spawn(&mut command).expect("start sync");
    let mut reader = BufReader::new(read_stream);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read start event");
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["event"],
        "start"
    );

    let readiness_deadline = Instant::now() + Duration::from_secs(10);
    while !fake_bin.join("create-started").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "sync exited before create ({status}): {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        assert!(
            Instant::now() < readiness_deadline,
            "sync did not reach create readiness"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let operation_id = fs::read_to_string(fake_bin.join("create-operation-id"))
        .unwrap()
        .trim()
        .to_string();
    let pending = operation_record(&repo_path, &operation_id);
    assert_eq!(pending["status"], "pending", "{pending}");
    assert_eq!(pending["touched_remote"], true, "{pending}");

    drop(reader);
    fs::write(fake_bin.join("release-create"), "release").unwrap();
    let process_deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < process_deadline,
            "sync did not stop after failed-create disconnect"
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(!status.success(), "disconnect must abort sync");
    assert_eq!(
        fs::read_to_string(fake_bin.join("create-attempts"))
            .unwrap()
            .trim(),
        "1"
    );
    assert!(!fake_bin.join("delete-attempts").exists());
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
        "dropped branch was pruned after disconnect"
    );
    let record = operation_record(&repo_path, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
    assert!(record["remote_effects"].as_array().unwrap().is_empty());
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert_eq!(
        config["pending_remote_branch_deletions"][0]["state"],
        "pending"
    );
    assert_eq!(
        config["stacks"]["failed-create-disconnect"]["review_creation_intents"]["c-1111111"]
            ["source_branch"],
        "testuser/failed-create-disconnect--c-1111111"
    );
    assert_eq!(
        config["stacks"]["failed-create-disconnect"]["review_creation_intents"]["c-1111111"]
            ["origin_operation_id"],
        operation_id
    );
    let requests = fs::read_to_string(fake_bin.join("failed-create-requests")).unwrap();
    let request_count = |prefix: &str| {
        requests
            .lines()
            .filter(|line| line.starts_with(prefix))
            .count()
    };
    assert_eq!(request_count("--version"), 1);
    assert_eq!(request_count("auth status"), 1);
    assert_eq!(request_count("pr list"), 0);
    assert_eq!(request_count("pr view"), 0);
    assert_eq!(request_count("pr create"), 1);
    assert_eq!(request_count("pr comment"), 0);
    assert_eq!(request_count("pr close"), 0);
    assert_eq!(
        request_count("api"),
        0,
        "no native-stack or navigation request"
    );
    assert_eq!(
        requests.lines().count(),
        3,
        "uncounted provider request: {requests}"
    );
    assert!(
        !fake_bin.join("delete-attempts").exists(),
        "prune count must be zero"
    );
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

fn setup_published_two_entry_stack(
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
    for (file, title, id) in [
        ("entry-a", "Entry A", "c-1111111"),
        ("entry-b", "Entry B", "c-2222222"),
    ] {
        fs::write(repo_path.join(file), format!("{title}\n")).unwrap();
        checked_git(&repo_path, &["add", file]);
        checked_git(
            &repo_path,
            &["commit", "-m", &format!("{title}\n\nGG-ID: {id}")],
        );
    }
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "initial sync failed: {stdout}\n{stderr}");

    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert_eq!(config["stacks"][stack_name]["mrs"]["c-1111111"], 101);
    assert_eq!(config["stacks"][stack_name]["mrs"]["c-2222222"], 102);
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-101"))
            .unwrap()
            .trim(),
        "main"
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-102"))
            .unwrap()
            .trim(),
        format!("testuser/{stack_name}--c-1111111")
    );

    (temp, repo_path, remote_path, fake_path)
}

fn setup_published_three_entry_stack(
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
    for (file, title, id) in [
        ("entry-a", "Entry A", "c-1111111"),
        ("entry-b", "Entry B", "c-2222222"),
        ("entry-c", "Entry C", "c-3333333"),
    ] {
        fs::write(repo_path.join(file), format!("{title}\n")).unwrap();
        checked_git(&repo_path, &["add", file]);
        checked_git(
            &repo_path,
            &["commit", "-m", &format!("{title}\n\nGG-ID: {id}")],
        );
    }
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "initial sync failed: {stdout}\n{stderr}");

    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert_eq!(config["stacks"][stack_name]["mrs"]["c-1111111"], 101);
    assert_eq!(config["stacks"][stack_name]["mrs"]["c-2222222"], 102);
    assert_eq!(config["stacks"][stack_name]["mrs"]["c-3333333"], 103);

    (temp, repo_path, remote_path, fake_path)
}

#[cfg(unix)]
fn install_partial_unstack_topology_gh(repo_path: &Path) {
    write_executable(
        &repo_path.join("fake-bin/gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s\n' "$*" >> "$GG_TEST_GH_LOG"

case "$1 ${2:-}" in
  '--version ')
    echo 'gh version 2.0.0'
    ;;
  'auth status')
    exit 0
    ;;
  'pr view')
    number=$3
    count_file="$dir/topology-view-count-$number"
    count=$(cat "$count_file" 2>/dev/null || echo 0)
    count=$((count + 1))
    printf '%s\n' "$count" > "$count_file"
    case "$number" in
      103)
        if [ "$count" -eq 2 ]; then
          if git --git-dir="$GG_TEST_REMOTE" show-ref --verify --quiet "refs/heads/$GG_TEST_NEW_C_HEAD"; then
            echo 'C was pushed before immediate authorization' >&2
            exit 1
          fi
          if [ -f "$dir/topology-created-c" ]; then
            echo 'C replacement was created before immediate authorization' >&2
            exit 1
          fi
          : > "$dir/topology-authorized-c"
        fi
        head=$GG_TEST_OLD_C_HEAD
        base=$GG_TEST_OLD_C_BASE
        ;;
      104)
        head=$GG_TEST_NEW_B_HEAD
        base=main
        ;;
      105)
        head=$GG_TEST_NEW_C_HEAD
        base=$GG_TEST_NEW_B_HEAD
        ;;
      *)
        echo "unexpected review $number" >&2
        exit 1
        ;;
    esac
    printf '{"number":%s,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/%s","headRefName":"%s","baseRefName":"%s","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}\n' "$number" "$number" "$head" "$base"
    ;;
  'pr create')
    count=$(cat "$dir/create-count" 2>/dev/null || echo 0)
    number=$((101 + count))
    printf '%s\n' $((count + 1)) > "$dir/create-count"
    shift 2
    head=
    base=
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --head) head=$2; shift 2;;
        --base) base=$2; shift 2;;
        *) shift;;
      esac
    done
    if [ "$head" = "$GG_TEST_NEW_C_HEAD" ]; then
      [ -f "$dir/topology-authorized-c" ] || {
        echo 'C replacement creation lacked immediate authorization' >&2
        exit 1
      }
      : > "$dir/topology-created-c"
    fi
    printf '%s\n' "$head" > "$dir/created-head-$number"
    printf '%s\n' "$base" > "$dir/created-base-$number"
    printf 'https://github.com/test/repo/pull/%s\n' "$number"
    ;;
  'pr edit')
    number=$3
    shift 3
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --base) printf '%s\n' "$2" > "$dir/created-base-$number"; shift 2;;
        *) shift;;
      esac
    done
    ;;
  'pr close')
    [ "$3" = 103 ] || {
      echo "unexpected close of review $3" >&2
      exit 1
    }
    [ -f "$dir/topology-commented-c" ] || {
      echo 'C was closed before its replacement comment' >&2
      exit 1
    }
    : > "$dir/topology-closed-c"
    ;;
  'api -X')
    if [ "$3" = POST ]; then
      case "$4" in
        */issues/103/comments)
          : > "$dir/topology-commented-c"
          ;;
        *)
          echo "unexpected comment endpoint $4" >&2
          exit 1
          ;;
      esac
    else
      echo "unexpected mutation method $3" >&2
      exit 1
    fi
    ;;
  'api '*)
    echo '[]'
    ;;
  *)
    echo "unexpected gh invocation: $*" >&2
    exit 1
    ;;
esac
"#,
    );
}

fn reset_fake_gh_observations(repo_path: &Path, log: &Path) {
    fs::write(log, "").unwrap();
    for number in [101, 102] {
        let count = repo_path.join(format!("fake-bin/view-count-{number}"));
        if count.exists() {
            fs::remove_file(count).unwrap();
        }
    }
}

#[cfg(unix)]
fn install_rebased_predecessor_gh(repo_path: &Path) {
    write_executable(
        &repo_path.join("fake-bin/gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s\n' "$*" >> "$GG_TEST_GH_LOG"

case "$1 ${2:-}" in
  '--version ')
    echo 'gh version 2.0.0'
    ;;
  'auth status')
    exit 0
    ;;
  'pr view')
    number=$3
    if [ "${4:-}" = --json ] && [ "${5:-}" = body ]; then
      echo 'legacy body without managed markers'
      exit 0
    fi
    if [ "$number" = 101 ] && [ "${GG_TEST_PREDECESSOR_MODE:-}" = lookup-fail ]; then
      echo 'simulated predecessor lookup failure' >&2
      exit 1
    fi
    head=$(cat "$dir/created-head-$number")
    base=$(cat "$dir/created-base-$number")
    oid=$(git --git-dir="$GG_TEST_REMOTE" rev-parse "refs/heads/$head")
    state=OPEN
    merge_commit=null
    reported_number=$number
    if [ "$number" = 101 ]; then
      state=MERGED
      merge_commit=$(printf '{"oid":"%s"}' "$GG_TEST_MERGE_OID")
      case "${GG_TEST_PREDECESSOR_MODE:-}" in
        wrong-source) head=testuser/other-stack--c-1111111;;
        open-state) state=OPEN; merge_commit=null;;
        closed-state) state=CLOSED; merge_commit=null;;
        wrong-number) reported_number=999;;
      esac
    fi
    if [ "$number" = 102 ] \
      && [ "${GG_TEST_FAIL_RETARGET_AFTER_PUSH:-}" = 1 ] \
      && [ ! -f "$dir/failed-retarget-auth" ] \
      && [ -f "$dir/rewritten-b-pushed" ]; then
      base=unrelated/branch
      : > "$dir/failed-retarget-auth"
    fi
    printf '{"number":%s,"title":"Entry","state":"%s","url":"https://github.com/test/repo/pull/%s","headRefName":"%s","headRefOid":"%s","baseRefName":"%s","mergeCommit":%s,"isDraft":false,"mergeable":"MERGEABLE","reviews":[]}\n' \
      "$reported_number" "$state" "$number" "$head" "$oid" "$base" "$merge_commit"
    ;;
  'pr edit')
    number=$3
    shift 3
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --base) printf '%s\n' "$2" > "$dir/created-base-$number"; shift 2;;
        *) shift;;
      esac
    done
    ;;
  'pr list')
    exit 0
    ;;
  'api '*)
    echo '[]'
    ;;
  *)
    echo "unexpected gh invocation: $*" >&2
    exit 1
    ;;
esac
"#,
    );
}

#[cfg(unix)]
fn setup_externally_merged_rebased_stack(
    stack_name: &str,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    std::ffi::OsString,
    String,
    String,
    String,
    String,
) {
    let (temp, repo_path, remote_path, fake_path) = setup_published_two_entry_stack(stack_name);
    let a_branch = format!("testuser/{stack_name}--c-1111111");
    let b_branch = format!("testuser/{stack_name}--c-2222222");
    let original_b_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{b_branch}")],
    );
    let original_base = checked_git(&repo_path, &["rev-parse", "main"]);
    let a_tree = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{a_branch}^{{tree}}")],
    );
    let merge_oid = checked_git(
        &repo_path,
        &[
            "commit-tree",
            &a_tree,
            "-p",
            &original_base,
            "-m",
            "Squash merge Entry A",
        ],
    );
    checked_git(
        &repo_path,
        &[
            "push",
            remote_path.to_str().unwrap(),
            &format!("{merge_oid}:refs/heads/main"),
        ],
    );

    install_rebased_predecessor_gh(&repo_path);
    let request_log = repo_path.join("fake-bin/rebased-requests");
    fs::write(&request_log, "").unwrap();
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["rebase"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", request_log.as_os_str()),
            ("GG_TEST_REMOTE", remote_path.as_os_str()),
            ("GG_TEST_MERGE_OID", merge_oid.as_ref()),
        ],
    );
    assert!(success, "actual gg rebase failed: {stdout}\n{stderr}");

    let (success, stdout, stderr) = run_gg(&repo_path, &["ls", "--json"]);
    assert!(success, "post-rebase ls failed: {stdout}\n{stderr}");
    let listing: Value = serde_json::from_str(&stdout).unwrap();
    let entries = listing["stack"]["entries"].as_array().unwrap();
    assert_eq!(
        entries.len(),
        1,
        "actual rebase must leave only B: {listing}"
    );
    assert_eq!(entries[0]["gg_id"], "c-2222222", "{listing}");

    let config_path = repo_path.join(".git/gg/config.json");
    let config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(config["stacks"][stack_name]["mrs"]["c-1111111"], 101);
    assert_eq!(config["stacks"][stack_name]["mrs"]["c-2222222"], 102);
    let rewritten_b_oid = checked_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_ne!(
        rewritten_b_oid, original_b_oid,
        "the squash merge must force B through the rewritten-branch path"
    );
    fs::write(&request_log, "").unwrap();

    (
        temp,
        repo_path,
        remote_path,
        fake_path,
        merge_oid,
        original_b_oid,
        rewritten_b_oid,
        b_branch,
    )
}

#[cfg(unix)]
#[test]
fn ordinary_sync_after_actual_rebase_requires_mapped_merged_predecessor_evidence() {
    let (
        _temp,
        repo_path,
        remote_path,
        fake_path,
        merge_oid,
        original_b_oid,
        _rewritten_b_oid,
        b_branch,
    ) = setup_externally_merged_rebased_stack("rebase-then-sync");
    let request_log = repo_path.join("fake-bin/rebased-requests");
    write_executable(
        &repo_path.join(".git/hooks/pre-push"),
        r#"#!/bin/sh
set -eu
: > "$(dirname "$GG_TEST_GH_LOG")/rewritten-b-pushed"
"#,
    );
    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", request_log.as_os_str()),
            ("GG_TEST_REMOTE", remote_path.as_os_str()),
            ("GG_TEST_MERGE_OID", merge_oid.as_ref()),
            (
                "GG_TEST_FAIL_RETARGET_AFTER_PUSH",
                std::ffi::OsStr::new("1"),
            ),
        ],
    );
    let unexpected_requests = fs::read_to_string(&request_log).unwrap();
    let unexpected_remote_b = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{b_branch}")],
    );
    assert!(
        !success,
        "the injected fresh retarget authorization failure must interrupt sync: {stdout}\n{stderr}\nremote B: {unexpected_remote_b}\npush marker: {}\nrequests:\n{unexpected_requests}",
        repo_path.join("fake-bin/rewritten-b-pushed").exists()
    );
    let authorization_error = format!("{stdout}{stderr}");
    assert!(
        authorization_error.contains("not open at the expected source and target"),
        "B must pass mapped-predecessor preflight and fail only at fresh retarget authorization: {}",
        authorization_error
    );
    let published_b_oid = checked_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_ne!(published_b_oid, original_b_oid);
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{b_branch}")]
        ),
        published_b_oid,
        "B must be pushed before the injected retarget authorization failure"
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-102"))
            .unwrap()
            .trim(),
        "testuser/rebase-then-sync--c-1111111",
        "failed authorization must prevent retargeting"
    );
    let failed_requests = fs::read_to_string(&request_log).unwrap();
    let failed_views = failed_requests
        .lines()
        .filter(|request| request.starts_with("pr view ") && request.contains("number,title,state"))
        .map(|request| request.split_whitespace().nth(2).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        failed_views,
        ["102", "101", "102", "102"],
        "classification must read B and its mapped predecessor before fresh push and retarget authorizations: {failed_requests}"
    );
    assert!(!failed_requests.contains("pr edit"), "{failed_requests}");

    fs::write(&request_log, "").unwrap();
    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", request_log.as_os_str()),
            ("GG_TEST_REMOTE", remote_path.as_os_str()),
            ("GG_TEST_MERGE_OID", merge_oid.as_ref()),
            (
                "GG_TEST_FAIL_RETARGET_AFTER_PUSH",
                std::ffi::OsStr::new("1"),
            ),
        ],
    );
    assert!(success, "retry must retarget B safely: {stdout}\n{stderr}");
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-102"))
            .unwrap()
            .trim(),
        "main"
    );
    let retry_requests = fs::read_to_string(&request_log).unwrap();
    let retry_sequence = retry_requests
        .lines()
        .filter_map(|request| {
            if request.starts_with("pr view ") {
                if !request.contains("number,title,state") {
                    return None;
                }
                Some(format!(
                    "view {}",
                    request.split_whitespace().nth(2).unwrap()
                ))
            } else if request.starts_with("pr edit ") {
                Some("edit 102".to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        retry_sequence,
        ["view 102", "view 101", "view 102", "edit 102"],
        "retry must preserve predecessor evidence and authorize immediately before retarget: {retry_requests}"
    );

    let (
        _boundary_temp,
        boundary_repo,
        boundary_remote,
        boundary_path,
        boundary_merge,
        boundary_original_b,
        _,
        boundary_b,
    ) = setup_externally_merged_rebased_stack("rebase-evidence-boundaries");
    let boundary_log = boundary_repo.join("fake-bin/rebased-requests");
    let config_path = boundary_repo.join(".git/gg/config.json");
    let original_config = fs::read(&config_path).unwrap();
    let mut config: Value = serde_json::from_slice(&original_config).unwrap();
    config["stacks"]["rebase-evidence-boundaries"]["mrs"]
        .as_object_mut()
        .unwrap()
        .remove("c-1111111");
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    for mode in [
        "mapping-absent",
        "wrong-source",
        "open-state",
        "closed-state",
        "wrong-number",
        "lookup-fail",
    ] {
        if mode != "mapping-absent" {
            fs::write(&config_path, &original_config).unwrap();
        }
        fs::write(&boundary_log, "").unwrap();
        let predecessor_mode = if mode == "mapping-absent" { "" } else { mode };
        let (success, stdout, stderr) = run_gg_bounded_with_env(
            &boundary_repo,
            &["sync", "--no-rebase-check"],
            &[
                ("PATH", boundary_path.as_os_str()),
                ("GG_TEST_GH_LOG", boundary_log.as_os_str()),
                ("GG_TEST_REMOTE", boundary_remote.as_os_str()),
                ("GG_TEST_MERGE_OID", boundary_merge.as_ref()),
                ("GG_TEST_PREDECESSOR_MODE", predecessor_mode.as_ref()),
            ],
        );
        assert!(
            !success,
            "{mode}: non-authoritative predecessor evidence was accepted: {stdout}\n{stderr}"
        );
        assert_eq!(
            checked_git(
                &boundary_remote,
                &["rev-parse", &format!("refs/heads/{boundary_b}")]
            ),
            boundary_original_b,
            "{mode}: rejection must happen before B is pushed"
        );
        let requests = fs::read_to_string(&boundary_log).unwrap();
        assert!(!requests.contains("pr edit"), "{mode}: {requests}");
        if mode == "mapping-absent" {
            assert!(
                !requests
                    .lines()
                    .any(|request| request.starts_with("pr view 101 ")),
                "an absent mapping must not authorize an arbitrary provider lookup: {requests}"
            );
        } else {
            assert!(
                requests
                    .lines()
                    .any(|request| request.starts_with("pr view 101 ")),
                "{mode}: the exact mapped predecessor must be read before rejection: {requests}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn sync_rejects_unstacked_root_review_without_authoritative_predecessor_mapping() {
    let (_temp, repo_path, remote_path, fake_path) =
        setup_published_two_entry_stack("unstack-missing-predecessor");
    let old_predecessor_branch = "testuser/unstack-missing-predecessor--c-1111111";
    let old_source_branch = "testuser/unstack-missing-predecessor--c-2222222";
    let new_source_branch = "testuser/upper--c-2222222";
    let old_source_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{old_source_branch}")],
    );
    let old_predecessor_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{old_predecessor_branch}")],
    );

    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["unstack", "--target", "2", "--name", "upper", "--no-tui"],
    );
    assert!(success, "unstack failed: {stdout}\n{stderr}");

    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["unstack-missing-predecessor"]["mrs"]
        .as_object_mut()
        .expect("old stack mappings")
        .remove("c-1111111");
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    assert_eq!(config["stacks"]["upper"]["mrs"]["c-2222222"], 102);
    assert!(config["stacks"]["unstack-missing-predecessor"]["mrs"]["c-2222222"].is_null());
    assert!(config["stacks"]["unstack-missing-predecessor"]["mrs"]["c-1111111"].is_null());

    let log = repo_path.join("fake-bin/requests");
    reset_fake_gh_observations(&repo_path, &log);
    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", log.as_os_str()),
        ],
    );
    assert!(
        !success,
        "review without authoritative unstack evidence was accepted: {stdout}\n{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("does not have the expected source and target"),
        "sync must reject the stale review during preflight: {stdout}\n{stderr}"
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{old_source_branch}")]
        ),
        old_source_oid
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{old_predecessor_branch}")]
        ),
        old_predecessor_oid
    );
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{new_source_branch}")
            ]
        )
        .0,
        "preflight failure must precede the replacement branch push"
    );
    let requests = fs::read_to_string(&log).unwrap();
    assert!(
        requests
            .lines()
            .any(|request| request.starts_with("pr view 102 ")),
        "the mapped review must be inspected before rejection: {requests}"
    );
    for forbidden in ["pr create", "pr edit", "pr close", "api -X POST"] {
        assert!(
            !requests.contains(forbidden),
            "unexpected {forbidden}: {requests}"
        );
    }
    let config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(config["stacks"]["upper"]["mrs"]["c-2222222"], 102);
    assert!(config["stacks"]["unstack-missing-predecessor"]["mrs"]["c-1111111"].is_null());
}

#[cfg(unix)]
#[test]
fn sync_replaces_unstacked_root_review_after_actual_unstack() {
    let (_temp, repo_path, remote_path, fake_path) =
        setup_published_two_entry_stack("published-unstack");
    let old_root_branch = "testuser/published-unstack--c-1111111";
    let old_source_branch = "testuser/published-unstack--c-2222222";
    let new_source_branch = "testuser/upper--c-2222222";
    let old_source_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{old_source_branch}")],
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-head-102"))
            .unwrap()
            .trim(),
        old_source_branch
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-102"))
            .unwrap()
            .trim(),
        old_root_branch
    );

    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["unstack", "--target", "2", "--name", "upper", "--no-tui"],
    );
    assert!(success, "unstack failed: {stdout}\n{stderr}");
    let config_path = repo_path.join(".git/gg/config.json");
    let config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        config["stacks"]["published-unstack"]["mrs"]["c-1111111"],
        101
    );
    assert!(config["stacks"]["published-unstack"]["mrs"]["c-2222222"].is_null());
    assert_eq!(config["stacks"]["upper"]["mrs"]["c-2222222"], 102);

    let log = repo_path.join("fake-bin/requests");
    reset_fake_gh_observations(&repo_path, &log);
    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", log.as_os_str()),
            ("GG_TEST_AUTH_RACE_REVIEW", std::ffi::OsStr::new("102")),
            ("GG_TEST_AUTH_RACE_ON_VIEW", std::ffi::OsStr::new("2")),
            (
                "GG_TEST_AUTH_RACE_BASE",
                std::ffi::OsStr::new("unrelated/branch"),
            ),
        ],
    );
    assert!(
        !success,
        "stale unstacked review authorization unexpectedly allowed a push: {stdout}\n{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("not open at the expected source and target"),
        "sync must classify the migrated root, then reject the strict authorization race: {stdout}\n{stderr}"
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{old_source_branch}")]
        ),
        old_source_oid,
        "failed authorization must not alter the old published source"
    );
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{new_source_branch}")
            ]
        )
        .0,
        "failed authorization must precede the replacement branch push"
    );
    let failed_requests = fs::read_to_string(&log).unwrap();
    assert!(!failed_requests.contains("pr create"), "{failed_requests}");
    let config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(config["stacks"]["upper"]["mrs"]["c-2222222"], 102);

    reset_fake_gh_observations(&repo_path, &log);
    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", log.as_os_str()),
        ],
    );
    assert!(success, "unstacked root sync failed: {stdout}\n{stderr}");
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-head-103"))
            .unwrap()
            .trim(),
        new_source_branch
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-103"))
            .unwrap()
            .trim(),
        "main"
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{new_source_branch}")]
        ),
        checked_git(&repo_path, &["rev-parse", new_source_branch])
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{old_source_branch}")]
        ),
        old_source_oid,
        "replacement must not mutate the old published source branch"
    );
    let config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(config["stacks"]["upper"]["mrs"]["c-2222222"], 103);
    let requests = fs::read_to_string(&log).unwrap();
    let authorization = requests
        .lines()
        .enumerate()
        .filter(|(_, request)| request.starts_with("pr view 102 "))
        .nth(1)
        .map(|(index, _)| index)
        .expect("old review must be re-read immediately before the replacement branch push");
    let replacement = requests
        .lines()
        .position(|request| {
            request.starts_with(&format!(
                "pr create --head {new_source_branch} --base main --title Entry B --body "
            ))
        })
        .unwrap_or_else(|| panic!("replacement review was not created: {requests}"));
    assert!(authorization < replacement, "{requests}");
    assert!(
        requests
            .lines()
            .any(|request| request.starts_with("pr view 102 --json ")),
        "old review must be re-read before replacement mutations: {requests}"
    );
}

#[cfg(unix)]
#[test]
fn sync_replaces_non_root_review_after_partial_unstack_replacement() {
    let (_temp, repo_path, remote_path, fake_path) =
        setup_published_three_entry_stack("partial-unstack");
    let old_a_branch = "testuser/partial-unstack--c-1111111";
    let old_b_branch = "testuser/partial-unstack--c-2222222";
    let old_c_branch = "testuser/partial-unstack--c-3333333";
    let new_b_branch = "testuser/upper--c-2222222";
    let new_c_branch = "testuser/upper--c-3333333";
    let old_c_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{old_c_branch}")],
    );

    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["unstack", "--target", "2", "--name", "upper", "--no-tui"],
    );
    assert!(success, "unstack failed: {stdout}\n{stderr}");
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(config["stacks"]["partial-unstack"]["mrs"]["c-1111111"], 101);
    assert!(config["stacks"]["partial-unstack"]["mrs"]["c-2222222"].is_null());
    assert!(config["stacks"]["partial-unstack"]["mrs"]["c-3333333"].is_null());
    assert_eq!(config["stacks"]["upper"]["mrs"]["c-2222222"], 102);
    assert_eq!(config["stacks"]["upper"]["mrs"]["c-3333333"], 103);
    config["defaults"]["sync_update_descriptions"] = Value::Bool(false);
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "partial sync failed: {stdout}\n{stderr}");
    let config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(config["stacks"]["upper"]["mrs"]["c-2222222"], 104);
    assert_eq!(config["stacks"]["upper"]["mrs"]["c-3333333"], 103);
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-head-104"))
            .unwrap()
            .trim(),
        new_b_branch
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-104"))
            .unwrap()
            .trim(),
        "main"
    );
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{new_c_branch}")
            ]
        )
        .0,
        "partial sync must not publish C"
    );

    install_partial_unstack_topology_gh(&repo_path);
    let log = repo_path.join("fake-bin/topology-requests");
    fs::write(&log, "").unwrap();
    write_executable(
        &repo_path.join(".git/hooks/pre-push"),
        r#"#!/bin/sh
set -eu
while read -r _local_ref _local_oid remote_ref _remote_oid; do
  if [ "$remote_ref" = "refs/heads/$GG_TEST_NEW_C_HEAD" ]; then
    [ -f "$(dirname "$GG_TEST_GH_LOG")/topology-authorized-c" ] || {
      echo 'C push lacked immediate review authorization' >&2
      exit 1
    }
    printf 'git push %s\n' "$remote_ref" >> "$GG_TEST_GH_LOG"
  fi
done
"#,
    );
    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", log.as_os_str()),
            ("GG_TEST_REMOTE", remote_path.as_os_str()),
            ("GG_TEST_OLD_C_HEAD", old_c_branch.as_ref()),
            ("GG_TEST_OLD_C_BASE", old_b_branch.as_ref()),
            ("GG_TEST_NEW_B_HEAD", new_b_branch.as_ref()),
            ("GG_TEST_NEW_C_HEAD", new_c_branch.as_ref()),
        ],
    );
    assert!(success, "full sync failed: {stdout}\n{stderr}");

    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{old_c_branch}")]
        ),
        old_c_oid,
        "replacement must not mutate C's old source branch"
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{new_c_branch}")]
        ),
        checked_git(&repo_path, &["rev-parse", new_c_branch])
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-105"))
            .unwrap()
            .trim(),
        new_b_branch
    );
    let config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(config["stacks"]["upper"]["mrs"]["c-2222222"], 104);
    assert_eq!(config["stacks"]["upper"]["mrs"]["c-3333333"], 105);

    let requests = fs::read_to_string(&log).unwrap();
    assert_eq!(
        requests
            .lines()
            .filter(|request| request.starts_with("pr create "))
            .count(),
        1,
        "full sync must create exactly one C replacement: {requests}"
    );
    let c_authorization = requests
        .lines()
        .enumerate()
        .filter(|(_, request)| request.starts_with("pr view 103 "))
        .nth(1)
        .map(|(index, _)| index)
        .expect("old C review must be re-read immediately before push");
    let c_push = requests
        .lines()
        .position(|request| request == format!("git push refs/heads/{new_c_branch}"))
        .unwrap_or_else(|| panic!("C replacement branch was not pushed: {requests}"));
    let c_replacement = requests
        .lines()
        .position(|request| request.starts_with(&format!("pr create --head {new_c_branch} ")))
        .unwrap_or_else(|| panic!("C replacement review was not created: {requests}"));
    assert!(c_authorization < c_push, "{requests}");
    assert!(c_push < c_replacement, "{requests}");
    assert_eq!(
        requests
            .lines()
            .filter(|request| request.starts_with("api -X POST "))
            .count(),
        1,
        "only old C should receive a replacement comment: {requests}"
    );
    assert!(requests.contains("/issues/103/comments"), "{requests}");
    assert_eq!(
        requests
            .lines()
            .filter(|request| request.starts_with("pr close "))
            .collect::<Vec<_>>(),
        ["pr close 103"],
        "only old C should be closed: {requests}"
    );
    assert!(repo_path.join("fake-bin/topology-commented-c").exists());
    assert!(repo_path.join("fake-bin/topology-closed-c").exists());
    assert!(
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{old_a_branch}")
            ]
        )
        .0
    );
}

#[cfg(unix)]
#[test]
fn sync_rejects_mismatched_source_targeting_unmapped_predecessor() {
    let (_temp, repo_path, _remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let fake_path = install_fake_gh(&repo_path);
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "unmapped-predecessor"]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");
    for (file, message) in [
        ("predecessor", "Predecessor\n\nGG-ID: c-1111111"),
        ("mapped", "Mapped\n\nGG-ID: c-2222222\nGG-Parent: c-1111111"),
    ] {
        fs::write(repo_path.join(file), file).unwrap();
        checked_git(&repo_path, &["add", file]);
        checked_git(&repo_path, &["commit", "-m", message]);
    }

    let predecessor_branch = "testuser/unmapped-predecessor--c-1111111";
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["unmapped-predecessor"]["mrs"] = serde_json::json!({
        "c-2222222": 428
    });
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    fs::write(
        repo_path.join("fake-bin/created-head-428"),
        "testuser/other-stack--c-2222222\n",
    )
    .unwrap();
    fs::write(
        repo_path.join("fake-bin/created-base-428"),
        format!("{predecessor_branch}\n"),
    )
    .unwrap();
    write_executable(
        &repo_path.join(".git/hooks/pre-push"),
        "#!/bin/sh\nprintf 'push\\n' >> \"$(git rev-parse --absolute-git-dir)/pushes\"\n",
    );
    let requests = repo_path.join("fake-bin/requests");

    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", requests.as_os_str()),
        ],
    );

    assert!(
        !success,
        "mismatched review targeting an unmapped predecessor was accepted: {stdout}\n{stderr}"
    );
    assert!(
        !repo_path.join(".git/pushes").exists(),
        "classification must reject before any push"
    );
    let requests = fs::read_to_string(&requests).unwrap();
    assert!(
        !requests.lines().any(|request| {
            request.starts_with("pr create ")
                || request.starts_with("pr edit ")
                || request.starts_with("pr close ")
                || request.starts_with("api ")
        }),
        "classification must reject before provider mutation: {requests}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("does not have the expected source and target"),
        "unexpected rejection: {stdout}\n{stderr}"
    );
    let config: Value = serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();
    assert_eq!(
        config["stacks"]["unmapped-predecessor"]["mrs"],
        serde_json::json!({"c-2222222": 428}),
        "rejection must preserve the existing review mapping"
    );
}

#[cfg(unix)]
#[test]
fn sync_retargets_published_reviews_after_local_reorder() {
    let (_temp, repo_path, remote_path, fake_path) =
        setup_published_two_entry_stack("published-reorder");
    let first_branch = "testuser/published-reorder--c-1111111";
    let second_branch = "testuser/published-reorder--c-2222222";
    let first_published_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{first_branch}")],
    );
    let second_published_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{second_branch}")],
    );
    let (success, stdout, stderr) = run_gg(&repo_path, &["reorder", "--order", "2,1"]);
    assert!(success, "reorder failed: {stdout}\n{stderr}");

    let log = repo_path.join("fake-bin/requests");
    reset_fake_gh_observations(&repo_path, &log);
    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", log.as_os_str()),
            ("GG_TEST_AUTH_RACE_REVIEW", std::ffi::OsStr::new("102")),
            ("GG_TEST_AUTH_RACE_ON_VIEW", std::ffi::OsStr::new("2")),
            (
                "GG_TEST_AUTH_RACE_BASE",
                std::ffi::OsStr::new("unrelated/branch"),
            ),
        ],
    );
    assert!(
        !success,
        "stale review authorization unexpectedly allowed a push: {stdout}\n{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("not open at the expected source and target"),
        "sync must reach strict immediate authorization before rejecting the race: {stdout}\n{stderr}"
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{first_branch}")]
        ),
        first_published_oid,
        "failed authorization must not push the other published entry"
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{second_branch}")]
        ),
        second_published_oid,
        "failed authorization must not push the reordered first entry"
    );
    assert!(
        !fs::read_to_string(&log).unwrap().contains("pr edit"),
        "failed authorization must precede review mutation"
    );

    reset_fake_gh_observations(&repo_path, &log);
    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", log.as_os_str()),
        ],
    );
    assert!(success, "reordered sync failed: {stdout}\n{stderr}");
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-102"))
            .unwrap()
            .trim(),
        "main"
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-101"))
            .unwrap()
            .trim(),
        second_branch
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{first_branch}")]
        ),
        checked_git(&repo_path, &["rev-parse", first_branch])
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{second_branch}")]
        ),
        checked_git(&repo_path, &["rev-parse", second_branch])
    );
    let requests = fs::read_to_string(&log).unwrap();
    let second_authorization = requests
        .lines()
        .enumerate()
        .filter(|(_, request)| request.starts_with("pr view 102 "))
        .nth(1)
        .map(|(index, _)| index)
        .expect("reordered first review must be re-read before mutation");
    let first_retarget = requests
        .lines()
        .position(|request| request == "pr edit 102 --base main")
        .expect("reordered first review must be retargeted to main");
    assert!(second_authorization < first_retarget, "{requests}");
}

#[cfg(unix)]
#[test]
fn sync_retries_retarget_after_pushing_rewritten_survivor_of_bottom_drop() {
    let (_temp, repo_path, remote_path, fake_path) =
        setup_published_two_entry_stack("published-bottom-drop");
    let dropped_branch = "testuser/published-bottom-drop--c-1111111";
    let surviving_branch = "testuser/published-bottom-drop--c-2222222";
    let surviving_published_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{surviving_branch}")],
    );
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "1", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    let rewritten_surviving_oid = checked_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_ne!(rewritten_surviving_oid, surviving_published_oid);

    let config_path = repo_path.join(".git/gg/config.json");
    let pending_after_drop: Value =
        serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    let pending_after_drop = pending_after_drop["pending_remote_branch_deletions"].clone();
    assert_eq!(pending_after_drop.as_array().unwrap().len(), 1);
    assert_eq!(pending_after_drop[0]["branch"], dropped_branch);

    let log = repo_path.join("fake-bin/requests");
    reset_fake_gh_observations(&repo_path, &log);
    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", log.as_os_str()),
            ("GG_TEST_AUTH_RACE_REVIEW", std::ffi::OsStr::new("102")),
            ("GG_TEST_AUTH_RACE_ON_VIEW", std::ffi::OsStr::new("3")),
            (
                "GG_TEST_AUTH_RACE_BASE",
                std::ffi::OsStr::new("unrelated/branch"),
            ),
        ],
    );
    assert!(
        !success,
        "stale review authorization unexpectedly allowed a retarget: {stdout}\n{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("not open at the expected source and target"),
        "sync must reject the race immediately before retargeting: {stdout}\n{stderr}"
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{surviving_branch}")]
        ),
        rewritten_surviving_oid,
        "the rewritten surviving branch push must land before retarget authorization fails"
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-102"))
            .unwrap()
            .trim(),
        dropped_branch,
        "failed authorization must leave the review targeting the dropped predecessor"
    );
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
        "failed authorization must not prune the dropped branch"
    );
    let config_after_failure: Value =
        serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        config_after_failure["pending_remote_branch_deletions"], pending_after_drop,
        "failed retarget authorization must preserve the pending deletion intent"
    );
    let failed_requests = fs::read_to_string(&log).unwrap();
    assert!(!failed_requests.contains("pr edit"), "{failed_requests}");
    assert!(!failed_requests.contains("pr list"), "{failed_requests}");

    reset_fake_gh_observations(&repo_path, &log);
    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", log.as_os_str()),
        ],
    );
    assert!(success, "post-drop retry failed: {stdout}\n{stderr}");
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-102"))
            .unwrap()
            .trim(),
        "main"
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{surviving_branch}")]
        ),
        rewritten_surviving_oid
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
        "the dropped branch must be deleted after its dependent review is retargeted"
    );
    let config_after_retry: Value =
        serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();
    assert!(
        config_after_retry["pending_remote_branch_deletions"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "successful retry must consume the pending deletion intent: {config_after_retry}"
    );
    let requests = fs::read_to_string(&log).unwrap();
    let authorization = requests
        .lines()
        .enumerate()
        .filter(|(_, request)| request.starts_with("pr view 102 "))
        .nth(1)
        .map(|(index, _)| index)
        .expect("surviving review must be re-read before mutation");
    let retarget = requests
        .lines()
        .position(|request| request == "pr edit 102 --base main")
        .expect("surviving review must be retargeted to main");
    let deletion_check = requests
        .lines()
        .position(|request| {
            request == format!("pr list --base {dropped_branch} --json number --jq .[].number")
        })
        .unwrap_or_else(|| {
            panic!("safe deletion must check for remaining dependent reviews: {requests}")
        });
    assert!(authorization < retarget, "{requests}");
    assert!(retarget < deletion_check, "{requests}");
}

#[cfg(unix)]
#[test]
fn sync_rejects_post_push_drop_recovery_when_dropped_branch_moved() {
    let (_temp, repo_path, remote_path, fake_path) =
        setup_published_two_entry_stack("moved-bottom-drop");
    let dropped_branch = "testuser/moved-bottom-drop--c-1111111";
    let surviving_branch = "testuser/moved-bottom-drop--c-2222222";
    let dropped_published_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{dropped_branch}")],
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "1", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    let rewritten_surviving_oid = checked_git(&repo_path, &["rev-parse", "HEAD"]);

    let log = repo_path.join("fake-bin/requests");
    reset_fake_gh_observations(&repo_path, &log);
    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", log.as_os_str()),
            ("GG_TEST_AUTH_RACE_REVIEW", std::ffi::OsStr::new("102")),
            ("GG_TEST_AUTH_RACE_ON_VIEW", std::ffi::OsStr::new("3")),
            (
                "GG_TEST_AUTH_RACE_BASE",
                std::ffi::OsStr::new("unrelated/branch"),
            ),
        ],
    );
    assert!(
        !success,
        "stale review authorization unexpectedly allowed a retarget: {stdout}\n{stderr}"
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{surviving_branch}")]
        ),
        rewritten_surviving_oid,
        "the rewritten survivor push must land before the retarget failure"
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-102"))
            .unwrap()
            .trim(),
        dropped_branch
    );

    checked_git(
        &repo_path,
        &[
            "push",
            "--force",
            "origin",
            &format!("HEAD:refs/heads/{dropped_branch}"),
        ],
    );
    let moved_dropped_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{dropped_branch}")],
    );
    assert_eq!(moved_dropped_oid, rewritten_surviving_oid);
    assert_ne!(moved_dropped_oid, dropped_published_oid);

    let config_path = repo_path.join(".git/gg/config.json");
    let config_before_retry: Value =
        serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    let pending_before_retry = config_before_retry["pending_remote_branch_deletions"].clone();
    assert_eq!(pending_before_retry.as_array().unwrap().len(), 1);
    assert_eq!(pending_before_retry[0]["branch"], dropped_branch);
    assert_eq!(
        pending_before_retry[0]["expected_oid"],
        dropped_published_oid
    );

    reset_fake_gh_observations(&repo_path, &log);
    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_GH_LOG", log.as_os_str()),
        ],
    );
    assert!(
        !success,
        "moved dropped branch unexpectedly authorized retry: {stdout}\n{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("does not have the expected source and target"),
        "retry must reject the moved pending target during preflight: {stdout}\n{stderr}"
    );
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/created-base-102"))
            .unwrap()
            .trim(),
        dropped_branch,
        "preflight rejection must leave the review target unchanged"
    );
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{dropped_branch}")]
        ),
        moved_dropped_oid,
        "preflight rejection must not prune the moved dropped branch"
    );
    let config_after_retry: Value =
        serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();
    assert_eq!(
        config_after_retry["pending_remote_branch_deletions"], pending_before_retry,
        "rejected recovery must preserve the pending deletion intent"
    );
    let requests = fs::read_to_string(log).unwrap();
    assert!(
        requests
            .lines()
            .any(|request| request.starts_with("pr view 102 ")),
        "retry must classify the mapped review before rejecting it: {requests}"
    );
    assert!(!requests.contains("pr edit"), "{requests}");
    assert!(!requests.contains("pr list"), "{requests}");
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
    assert_eq!(
        operation["status"], "committed",
        "post-mutation prune errors must finalize their segment"
    );
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

#[cfg(unix)]
#[test]
fn jsonl_disconnect_after_replacement_comment_stops_before_old_review_close() {
    let (_temp, repo_path, _remote_path) = create_test_repo_with_remote();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github","base":"main","sync_behind_threshold":0}}"#,
    )
    .unwrap();
    let (ok, out, err) = run_gg(&repo_path, &["co", "replacement-disconnect"]);
    assert!(ok, "{out}\n{err}");
    checked_git(
        &repo_path,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7777777"],
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
    "replacement-disconnect": {
      "base": "main",
      "mrs": {"c-7777777": 428}
    }
  }
}"#,
    )
    .unwrap();

    let fake_bin = repo_path.join("replacement-disconnect-bin");
    fs::create_dir_all(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr view')
    echo '{"number":428,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/428","headRefName":"testuser/old-stack--c-7777777","baseRefName":"main","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
    ;;
  'pr create')
    attempts=$(cat "$dir/create-attempts" 2>/dev/null || echo 0)
    echo $((attempts + 1)) > "$dir/create-attempts"
    echo 'https://github.com/test/repo/pull/900'
    ;;
  'api -X')
    case "$*" in
      *issues/428/comments*)
        attempts=$(cat "$dir/comment-attempts" 2>/dev/null || echo 0)
        echo $((attempts + 1)) > "$dir/comment-attempts"
        operation_id=''
        count=0
        for record in "$TEST_GIT_DIR/gg/operations/"*.json; do
          if grep -Eq '"status"[[:space:]]*:[[:space:]]*"pending"' "$record"; then
            operation_id=$(basename "$record" .json)
            count=$((count + 1))
          fi
        done
        [ "$count" -eq 1 ]
        printf '%s\n' "$operation_id" > "$dir/comment-operation-id"
        touch "$dir/comment-started"
        while [ ! -f "$dir/release-comment" ]; do sleep 0.01; done
        echo '{}'
        ;;
      *) echo '[]';;
    esac
    ;;
  'api repos/'*) echo '[]';;
  'pr close')
    attempts=$(cat "$dir/close-attempts" 2>/dev/null || echo 0)
    echo $((attempts + 1)) > "$dir/close-attempts"
    ;;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut path = std::ffi::OsString::from(fake_bin.as_os_str());
    path.push(":");
    path.push(old_path);

    let (read_stream, write_stream) = UnixStream::pair().unwrap();
    read_stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let stdout_fd: std::os::fd::OwnedFd = write_stream.into();
    let stderr_path = repo_path.join(".git/replacement-disconnect-stderr");
    let mut command = Command::new(env!("CARGO_BIN_EXE_gg"));
    command
        .args(["sync", "--until", "1", "--no-rebase-check", "--jsonl"])
        .current_dir(&repo_path)
        .env("PATH", path)
        .env("TEST_GIT_DIR", repo_path.join(".git"))
        .stdout(Stdio::from(stdout_fd))
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
    let mut child = ProcessGroupGuard::spawn(&mut command).expect("start sync");
    let mut reader = BufReader::new(read_stream);
    let mut first = String::new();
    reader.read_line(&mut first).expect("read start event");
    assert_eq!(
        serde_json::from_str::<Value>(&first).unwrap()["event"],
        "start"
    );

    let readiness_deadline = Instant::now() + Duration::from_secs(10);
    while !fake_bin.join("comment-started").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            let mut remaining = String::new();
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => remaining.push_str(&line),
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        break;
                    }
                    Err(error) => panic!("read failed replacement output: {error}"),
                }
            }
            panic!(
                "sync exited before replacement comment ({status}): {remaining}\nstderr: {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        if Instant::now() >= readiness_deadline {
            let _ = fs::write(fake_bin.join("release-comment"), "release");
            panic!(
                "sync did not reach replacement comment: {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let operation_id = fs::read_to_string(fake_bin.join("comment-operation-id"))
        .unwrap()
        .trim()
        .to_string();
    let pending = operation_record(&repo_path, &operation_id);
    assert_eq!(pending["status"], "pending", "{pending}");
    assert_eq!(pending["touched_remote"], true, "{pending}");

    drop(reader);
    fs::write(fake_bin.join("release-comment"), "release").unwrap();
    let process_deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= process_deadline {
            panic!(
                "sync did not stop after JSONL disconnect: {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(!status.success(), "disconnect must abort sync");
    assert_eq!(
        fs::read_to_string(fake_bin.join("create-attempts"))
            .unwrap()
            .trim(),
        "1"
    );
    assert_eq!(
        fs::read_to_string(fake_bin.join("comment-attempts"))
            .unwrap()
            .trim(),
        "1"
    );
    assert!(
        !fake_bin.join("close-attempts").exists(),
        "old review was closed after disconnected replacement-comment event"
    );
    let record = operation_record(&repo_path, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
}

#[cfg(unix)]
#[test]
fn replacement_close_persistence_failure_emits_no_success_and_keeps_uncertainty() {
    let (_temp, repo_path, _remote_path) = create_test_repo_with_remote();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github","base":"main","sync_behind_threshold":0}}"#,
    )
    .unwrap();
    let (ok, out, err) = run_gg(&repo_path, &["co", "replacement-close-persistence"]);
    assert!(ok, "{out}\n{err}");
    checked_git(
        &repo_path,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7777778"],
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
    "replacement-close-persistence": {
      "base": "main",
      "mrs": {"c-7777778": 429}
    }
  }
}"#,
    )
    .unwrap();

    let fake_bin = repo_path.join("replacement-close-persistence-bin");
    fs::create_dir_all(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr view')
    echo '{"number":429,"title":"Entry","state":"OPEN","url":"https://github.com/test/repo/pull/429","headRefName":"testuser/old-stack--c-7777778","baseRefName":"main","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
    ;;
  'pr create')
    echo 'https://github.com/test/repo/pull/901'
    ;;
  'api -X') echo '{}';;
  'api repos/'*) echo '[]';;
  'pr close')
    attempts=$(cat "$dir/close-attempts" 2>/dev/null || echo 0)
    echo $((attempts + 1)) > "$dir/close-attempts"
    operation_id=''
    count=0
    for record in "$TEST_GIT_DIR/gg/operations/"*.json; do
      if grep -Eq '"status"[[:space:]]*:[[:space:]]*"pending"' "$record"; then
        operation_id=$(basename "$record" .json)
        count=$((count + 1))
      fi
    done
    [ "$count" -eq 1 ]
    printf '%s\n' "$operation_id" > "$dir/close-operation-id"
    mkdir "$TEST_GIT_DIR/gg/operations/$operation_id.json.tmp"
    ;;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut path = std::ffi::OsString::from(fake_bin.as_os_str());
    path.push(":");
    path.push(old_path);

    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check", "--jsonl"],
        &[
            ("PATH", path.as_os_str()),
            ("TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
        ],
    );
    assert!(
        !success,
        "persistence failure must abort sync: {stdout}\n{stderr}"
    );
    let operation_id = fs::read_to_string(fake_bin.join("close-operation-id"))
        .unwrap()
        .trim()
        .to_string();
    let obstruction = gg_dir
        .join("operations")
        .join(format!("{operation_id}.json.tmp"));
    fs::remove_dir(&obstruction).unwrap();

    let events: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        events.iter().all(|event| event["event"] != "pr_closed"),
        "close success escaped before durable persistence: {events:?}"
    );
    assert!(
        events.iter().all(|event| event["event"] != "summary"),
        "later summary escaped after close persistence failure: {events:?}"
    );
    assert_eq!(
        fs::read_to_string(fake_bin.join("close-attempts"))
            .unwrap()
            .trim(),
        "1"
    );
    let record = operation_record(&repo_path, &operation_id);
    assert_eq!(record["status"], "pending", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
    assert!(
        record["remote_effects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|effect| effect["kind"] != "pr_closed"),
        "failed durable write leaked a close effect: {record}"
    );
}

#[cfg(unix)]
#[test]
fn jsonl_disconnect_after_push_stops_before_review_creation() {
    let (_temp, repo_path, _remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["github"]["stacks_integration"] = serde_json::json!("off");
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "disconnect-push"]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");
    checked_git(
        &repo_path,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7000002"],
    );

    let fake_bin = repo_path.join("fake-disconnect-push-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr create') touch "$dir/create-attempted"; echo 'https://github.com/test/repo/pull/101';;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let real_git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("locate git");
    assert!(real_git.status.success());
    let real_git = String::from_utf8(real_git.stdout).unwrap();
    write_executable(
        &fake_bin.join("git"),
        r#"#!/bin/sh
set -eu
"$GG_TEST_REAL_GIT" "$@"
case " $* " in
  *" push "*)
    operations="$($GG_TEST_REAL_GIT rev-parse --absolute-git-dir)/gg/operations"
    pending=
    for record in "$operations"/*.json; do
      grep -q '"status": "pending"' "$record" || continue
      [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
      pending="$record"
    done
    [ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
    basename "$pending" .json > "$(dirname "$0")/operation-id"
    touch "$(dirname "$0")/push-complete"
    while [ ! -f "$(dirname "$0")/release-push" ]; do sleep 0.01; done
    ;;
esac
"#,
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();

    let (read_stream, write_stream) = UnixStream::pair().unwrap();
    read_stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let stdout_fd: std::os::fd::OwnedFd = write_stream.into();
    let stderr_path = repo_path.join(".git/push-disconnect-stderr");
    let mut command = Command::new(env!("CARGO_BIN_EXE_gg"));
    command
        .args(["sync", "--until", "1", "--no-rebase-check", "--jsonl"])
        .current_dir(&repo_path)
        .env("PATH", path)
        .env("GG_TEST_REAL_GIT", real_git.trim())
        .stdout(Stdio::from(stdout_fd))
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
    let mut child = ProcessGroupGuard::spawn(&mut command).expect("start sync");
    let mut reader = BufReader::new(read_stream);
    let mut first = String::new();
    reader.read_line(&mut first).expect("read start event");
    assert_eq!(
        serde_json::from_str::<Value>(&first).unwrap()["event"],
        "start"
    );

    let readiness_deadline = Instant::now() + Duration::from_secs(10);
    while !fake_bin.join("push-complete").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "sync exited before blocked push completed ({status}): {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        if Instant::now() >= readiness_deadline {
            let _ = fs::write(fake_bin.join("release-push"), "release");
            panic!("sync did not reach blocked push completion");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let operation_id = fs::read_to_string(fake_bin.join("operation-id"))
        .unwrap()
        .trim()
        .to_string();
    let pending = operation_record(&repo_path, &operation_id);
    assert_eq!(pending["status"], "pending", "{pending}");
    assert_eq!(pending["touched_remote"], true, "{pending}");

    drop(reader);
    fs::write(fake_bin.join("release-push"), "release").unwrap();
    let process_deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= process_deadline {
            panic!("sync did not stop after push checkpoint disconnect");
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(!status.success(), "closed JSONL consumer must abort sync");
    assert!(
        !fake_bin.join("create-attempted").exists(),
        "review creation followed disconnected push checkpoint"
    );
    let record = operation_record(&repo_path, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
    assert_eq!(
        record["remote_effects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|effect| effect["kind"] == "pushed")
            .count(),
        1,
        "{record}"
    );
}

#[cfg(unix)]
#[test]
fn push_success_event_waits_for_durable_remote_effect() {
    let (_temp, repo_path, _remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["github"]["stacks_integration"] = serde_json::json!("off");
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "durable-push"]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");
    checked_git(
        &repo_path,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7000001"],
    );

    let fake_bin = repo_path.join("fake-durable-push-bin");
    fs::create_dir(&fake_bin).unwrap();
    write_executable(
        &fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr create') touch "$dir/create-attempted"; echo 'https://github.com/test/repo/pull/101';;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#,
    );
    let real_git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("locate git");
    assert!(real_git.status.success());
    let real_git = String::from_utf8(real_git.stdout).unwrap();
    write_executable(
        &fake_bin.join("git"),
        r#"#!/bin/sh
set -eu
"$GG_TEST_REAL_GIT" "$@"
case " $* " in
  *" push "*)
    operations="$($GG_TEST_REAL_GIT rev-parse --absolute-git-dir)/gg/operations"
    pending=
    for record in "$operations"/*.json; do
      grep -q '"status": "pending"' "$record" || continue
      [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
      pending="$record"
    done
    [ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
    basename "$pending" .json > "$(dirname "$0")/operation-id"
    mkdir "$pending.tmp"
    ;;
esac
"#,
    );
    let mut paths = vec![fake_bin.clone()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let path = std::env::join_paths(paths).unwrap();

    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check", "--jsonl"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.trim().as_ref()),
        ],
    );
    let operation_id = fs::read_to_string(fake_bin.join("operation-id"))
        .expect("push shim must capture operation id")
        .trim()
        .to_string();
    let obstruction = repo_path
        .join(".git/gg/operations")
        .join(format!("{operation_id}.json.tmp"));
    fs::remove_dir(&obstruction).expect("restore operation store");

    assert!(
        !success,
        "persistence failure must stop sync: {stdout}\n{stderr}"
    );
    let events = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect::<Vec<_>>();
    assert!(
        events.iter().all(|event| event["event"] != "push_done"),
        "push success was emitted before durable recording: {events:?}"
    );
    assert!(
        !fake_bin.join("create-attempted").exists(),
        "review creation followed failed push-effect persistence"
    );
    let operation = operation_record(&repo_path, &operation_id);
    assert_eq!(operation["status"], "pending", "{operation}");
    assert_eq!(operation["touched_remote"], true, "{operation}");
    assert_eq!(
        operation["remote_effects"],
        serde_json::json!([]),
        "{operation}"
    );
}

#[cfg(unix)]
#[test]
fn test_sync_marks_ambiguous_push_before_attempt() {
    let (_temp, repo_path, remote_path) = create_test_repo_with_remote();
    write_remote_prune_config(&repo_path);
    let fake_path = install_fake_gh(&repo_path);
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "ambiguous-push"]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");
    checked_git(
        &repo_path,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-1111111"],
    );
    let entry_branch = "testuser/ambiguous-push--c-1111111";
    let real_git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("locate git");
    assert!(real_git.status.success());
    let real_git = String::from_utf8(real_git.stdout)
        .expect("git path is UTF-8")
        .trim()
        .to_string();
    write_executable(
        &repo_path.join("fake-bin/git"),
        r#"#!/bin/sh
set -eu
if [ "$1" = "push" ] && [ "${4:-}" = "$GG_TEST_PUSH_BRANCH" ]; then
  "$GG_TEST_REAL_GIT" "$@"
  echo "simulated lost push response" >&2
  exit 1
fi
exec "$GG_TEST_REAL_GIT" "$@"
"#,
    );
    let operations_before = operation_record_ids(&repo_path);

    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--no-rebase-check"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_PUSH_BRANCH", entry_branch.as_ref()),
        ],
    );

    assert!(!success, "lost response must fail sync: {stdout}\n{stderr}");
    assert_eq!(
        checked_git(
            &remote_path,
            &["rev-parse", &format!("refs/heads/{entry_branch}")]
        ),
        checked_git(
            &repo_path,
            &["rev-parse", &format!("refs/heads/{entry_branch}")]
        ),
        "the shim must push before reporting failure"
    );
    let new_ids = operation_record_ids(&repo_path)
        .difference(&operations_before)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(new_ids.len(), 1, "failed push must use one sync operation");
    let operation_id = new_ids.into_iter().next().unwrap();
    let operation = operation_record(&repo_path, &operation_id);
    assert_eq!(operation["kind"], "sync", "{operation}");
    assert_eq!(operation["status"], "committed", "{operation}");
    assert_eq!(operation["touched_remote"], true, "{operation}");

    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", &operation_id, "--json"]);
    assert!(
        !success,
        "undo must not claim local-only safety: {stdout}\n{stderr}"
    );
    let refusal: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(refusal["refusal"]["reason"], "remote");
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

    let operation_ids_before_prune = operation_record_ids(&repo_path);
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

    let operation_ids = operation_record_ids(&repo_path)
        .difference(&operation_ids_before_prune)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        operation_ids.len(),
        3,
        "review creation and each deletion attempt must own exact operation segments: {operation_ids:?}"
    );
    let operations = operation_ids
        .iter()
        .map(|id| (id.clone(), operation_record(&repo_path, id)))
        .collect::<Vec<_>>();
    for (_, operation) in &operations {
        assert_eq!(operation["kind"], "sync");
        assert_eq!(operation["status"], "committed", "{operation}");
        assert_eq!(operation["touched_remote"], true, "{operation}");
    }
    let (_, operation) = operations
        .iter()
        .find(|(_, operation)| {
            operation["remote_effects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|effect| effect["kind"] == "branch_deleted")
        })
        .expect("successful deletion must have its own durable effect");
    let uncertain = operations
        .iter()
        .find(|(_, operation)| operation["remote_effects"] == serde_json::json!([]))
        .expect("failed deletion must have its own uncertainty segment");
    assert_eq!(uncertain.1["remote_effects"], serde_json::json!([]));
    let deleted_effect = operation["remote_effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|effect| effect["kind"] == "branch_deleted")
        .unwrap();
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
fn branch_delete_config_save_and_finalization_failures_are_reported_together() {
    let (_temp, repo_path, remote_path, fake_path) =
        setup_published_prune_stack("delete-save-finalize-failure");
    let dropped_branch = "testuser/delete-save-finalize-failure--c-3333333";
    let dropped_oid = checked_git(
        &remote_path,
        &["rev-parse", &format!("refs/heads/{dropped_branch}")],
    );
    let later_branch = "testuser/delete-save-finalize-failure--c-2222222";
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "2", "--yes"]);
    assert!(success, "second drop failed: {stdout}\n{stderr}");
    clear_stack_mappings(&repo_path, "delete-save-finalize-failure");
    let real_git = install_git_delete_shim(&repo_path);
    let git_log = repo_path.join("fake-bin/git-requests");

    let blocker_repo = repo_path.clone();
    let blocker = std::thread::spawn(move || {
        let fake_bin = blocker_repo.join("fake-bin");
        wait_for_file(&fake_bin.join("delete-complete"));
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(blocker_repo.join(".git/gg/config.lock"))
            .unwrap();
        // SAFETY: the descriptor stays open for the lock's lifetime and refers
        // to the same config lock file used by Config::save.
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
        fs::write(fake_bin.join("config-lock-held"), "").unwrap();

        // The deletion shim cannot return before this lock is held. Give the
        // current implementation ample time to expose an early finalization;
        // the required implementation instead blocks in the config save.
        std::thread::sleep(Duration::from_secs(2));
        let operation_id = fs::read_to_string(fake_bin.join("delete-operation-id"))
            .unwrap()
            .trim()
            .to_string();
        fs::create_dir(blocker_repo.join(".git/gg/config.tmp")).unwrap();
        fs::create_dir(
            blocker_repo
                .join(".git/gg/operations")
                .join(format!("{operation_id}.json.tmp")),
        )
        .unwrap();
        // SAFETY: this releases the exclusive lock acquired above before the
        // file is dropped.
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_UN) }, 0);
        operation_id
    });

    let (success, stdout, stderr) = run_gg_bounded_with_env(
        &repo_path,
        &["sync", "--until", "1", "--no-rebase-check", "--jsonl"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_DELETE_BRANCH", dropped_branch.as_ref()),
            (
                "GG_TEST_DELETE_MODE",
                "block_post_delete_completion".as_ref(),
            ),
            ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ("GG_TEST_GIT_LOG", git_log.as_os_str()),
        ],
    );
    let operation_id = blocker.join().unwrap();
    fs::remove_dir(repo_path.join(".git/gg/config.tmp")).unwrap();
    fs::remove_dir(
        repo_path
            .join(".git/gg/operations")
            .join(format!("{operation_id}.json.tmp")),
    )
    .unwrap();

    assert!(!success, "double failure must fail: {stdout}\n{stderr}");
    let diagnostic = format!("{stdout}\n{stderr}");
    assert!(
        diagnostic.contains("additionally failed to finalize sync operation"),
        "finalization error was lost: {diagnostic}"
    );
    assert!(
        diagnostic.matches("Is a directory").count() >= 2,
        "config-save or finalization error was lost: {diagnostic}"
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
        "delete shim must remove the remote branch"
    );
    let record = operation_record(&repo_path, &operation_id);
    assert_eq!(record["kind"], "sync", "{record}");
    assert_eq!(record["status"], "pending", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
    let effects = record["remote_effects"].as_array().unwrap();
    assert_eq!(effects.len(), 1, "{record}");
    assert_eq!(effects[0]["kind"], "branch_deleted", "{record}");
    assert_eq!(effects[0]["remote"], "origin", "{record}");
    assert_eq!(effects[0]["branch"], dropped_branch, "{record}");
    assert_eq!(effects[0]["prior_oid"], dropped_oid, "{record}");
    assert_eq!(
        fs::read_to_string(repo_path.join("fake-bin/delete-count")).unwrap(),
        "1\n"
    );
    let pushes = fs::read_to_string(&git_log)
        .unwrap()
        .lines()
        .filter(|request| request.split_whitespace().any(|arg| arg == "push"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(
        pushes.len(),
        1,
        "a later remote mutation started after deletion completion failed: {pushes:?}"
    );
    assert!(pushes[0].contains(dropped_branch), "{pushes:?}");
    assert!(
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{later_branch}")
            ]
        )
        .0,
        "the next authorized deletion started after completion failed"
    );
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(events.iter().all(|event| event["event"] != "summary"));
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    let intent = config["pending_remote_branch_deletions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|intent| intent["branch"] == dropped_branch)
        .expect("disk must retain deleting intent after failed save");
    assert_eq!(intent["state"], "deleting");
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
    assert_eq!(
        operation["status"], "committed",
        "post-mutation prune errors must finalize their segment"
    );
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
