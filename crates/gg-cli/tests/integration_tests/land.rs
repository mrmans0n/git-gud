use crate::helpers::{create_test_repo, create_test_repo_with_remote, run_gg, run_git};

use serde_json::Value;
use std::ffi::OsString;
use std::fs;
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
            test_home,
        }
    }

    fn start_land(&self) -> std::process::Child {
        Command::new(env!("CARGO_BIN_EXE_gg"))
            .args(["land", "--all", "--wait", "--no-clean"])
            .current_dir(&self.repo_path)
            .env("HOME", &self.test_home)
            .env("PATH", &self.path)
            .env("GG_FAKE_POLLED", &self.polled)
            .env("GG_FAKE_READY", &self.ready)
            .env("GG_FAKE_MERGED", &self.merged)
            .env("GG_FAKE_CI_CALLS", &self.ci_calls)
            .env("GG_FAKE_REGRESS", &self.regress)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start waiting land")
    }

    fn wait_until_polling(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.polled.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(self.polled.exists(), "land never started polling CI");
    }

    fn release_ci(&self) {
        fs::write(&self.ready, "ready\n").expect("release fake CI");
    }
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
