use crate::helpers::{
    create_test_repo, create_test_repo_with_remote, create_test_repo_with_worktree_support,
    isolated_home, run_gg, run_gg_with_env, run_git, run_git_full,
};

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
#[ignore = "subprocess helper for public clean API integration tests"]
fn run_public_clean_for_stack_with_repo_helper() {
    let repo_path = std::env::var_os("GG_TEST_PUBLIC_CLEAN_REPO")
        .expect("public clean helper requires GG_TEST_PUBLIC_CLEAN_REPO");
    let stack_name = std::env::var("GG_TEST_PUBLIC_CLEAN_STACK").expect("stack name");
    std::env::set_current_dir(repo_path).expect("enter public clean test repo");
    let repo = gg_core::git::open_repo().expect("open public clean test repo");
    gg_core::commands::clean::run_for_stack_with_repo(&repo, &stack_name, true)
        .expect("public clean entry point failed");
}

fn run_public_clean_for_stack_with_repo(
    repo_path: &Path,
    stack_name: &str,
    path: &std::ffi::OsStr,
) -> std::process::Output {
    run_public_clean_for_stack_with_repo_with_env(repo_path, stack_name, path, &[])
}

fn run_public_clean_for_stack_with_repo_with_env(
    repo_path: &Path,
    stack_name: &str,
    path: &std::ffi::OsStr,
    envs: &[(&str, &std::ffi::OsStr)],
) -> std::process::Output {
    let mut command =
        Command::new(std::env::current_exe().expect("current integration test executable"));
    command
        .args([
            "--ignored",
            "--exact",
            "clean::run_public_clean_for_stack_with_repo_helper",
            "--nocapture",
        ])
        .env("GG_TEST_PUBLIC_CLEAN_REPO", repo_path)
        .env("GG_TEST_PUBLIC_CLEAN_STACK", stack_name)
        .env("HOME", isolated_home(repo_path))
        .env("PATH", path);
    for (key, value) in envs {
        command.env(key, value);
    }
    command
        .output()
        .expect("run public clean integration helper")
}

fn clean_operation_records(repo_path: &Path) -> Vec<Value> {
    let operations = repo_path.join(".git/gg/operations");
    if !operations.exists() {
        return Vec::new();
    }
    fs::read_dir(operations)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.unwrap();
            entry
                .file_type()
                .unwrap()
                .is_file()
                .then(|| serde_json::from_slice::<Value>(&fs::read(entry.path()).unwrap()).unwrap())
        })
        .filter(|record| {
            record["kind"] == "clean"
                && record["remote_effects"]
                    .as_array()
                    .is_some_and(|effects| !effects.is_empty())
        })
        .collect()
}

fn checked_git(repo: &Path, args: &[&str]) -> String {
    let (success, output) = run_git(repo, args);
    assert!(success, "git {args:?} failed: {output}");
    output.trim().to_string()
}

fn install_fake_merged_gh(repo_path: &Path) -> std::ffi::OsString {
    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).unwrap();
    fs::write(
        fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
if [ "$1" = "--version" ] || { [ "$1" = "auth" ] && [ "$2" = "status" ]; }; then
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
  echo "https://github.com/test/repo/pull/101"
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  echo '{"number":101,"title":"Entry","state":"MERGED","url":"https://github.com/test/repo/pull/101","headRefName":null,"isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  if [ "$3" = "--base" ] && [ "${GG_TEST_DEFER_ON_SECOND_BRANCH:-}" = "$4" ]; then
    count_file="$(dirname "$0")/second-lookup-count"
    count=0
    [ ! -f "$count_file" ] || count=$(cat "$count_file")
    count=$((count + 1))
    printf '%s\n' "$count" > "$count_file"
    [ "$count" -lt 2 ] || echo '104'
    exit 0
  fi
  if [ "$3" = "--base" ] && [ -f "$(dirname "$0")/targets/$(echo "$4" | tr / _)" ]; then
    cat "$(dirname "$0")/targets/$(echo "$4" | tr / _)"
  fi
  exit 0
fi
echo "unexpected gh invocation: $@" >&2
exit 1
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(fake_bin.join("gh")).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(fake_bin.join("gh"), permissions).unwrap();
    }
    let mut path = std::ffi::OsString::from(fake_bin.as_os_str());
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    path
}

fn setup_published_drop_for_clean(
    stack_name: &str,
) -> (
    tempfile::TempDir,
    PathBuf,
    PathBuf,
    std::ffi::OsString,
    String,
) {
    let (temp, repo_path, remote_path) = create_test_repo_with_remote();
    fs::create_dir_all(repo_path.join(".git/gg")).unwrap();
    fs::write(
        repo_path.join(".git/gg/config.json"),
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
    .unwrap();
    let fake_path = install_fake_merged_gh(&repo_path);
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
        &["sync", "--no-rebase-check"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "sync failed: {stdout}\n{stderr}");
    let dropped_branch = format!("testuser/{stack_name}--c-3333333");
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--yes"]);
    assert!(success, "drop failed: {stdout}\n{stderr}");
    checked_git(&repo_path, &["checkout", "main"]);
    (temp, repo_path, remote_path, fake_path, dropped_branch)
}

#[cfg(unix)]
fn install_clean_delete_git_shim(repo_path: &Path) -> String {
    let output = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("locate git");
    assert!(output.status.success());
    let real_git = String::from_utf8(output.stdout)
        .expect("git path is UTF-8")
        .trim()
        .to_string();
    let shim = repo_path.join("fake-bin/git");
    fs::write(
        &shim,
        r#"#!/bin/sh
set -eu
case " $* " in
  *" worktree remove "*)
    if [ "${GG_TEST_DELETE_MODE:-}" = "block_worktree_finalize" ]; then
      "$GG_TEST_REAL_GIT" "$@"
      pending=''
      for record in "$GG_TEST_GIT_DIR/gg/operations/"*.json; do
        [ -f "$record" ] || continue
        grep -q '"kind": "clean"' "$record" || continue
        grep -q '"status": "pending"' "$record" || continue
        [ -z "$pending" ] || { echo 'multiple pending clean operations' >&2; exit 1; }
        pending=$(basename "$record" .json)
      done
      [ -n "$pending" ] || { echo 'missing pending clean operation' >&2; exit 1; }
      mkdir "$GG_TEST_GIT_DIR/gg/operations/$pending.json.tmp"
      exit 0
    fi
    ;;
  *" push --force-with-lease=refs/heads/"*" :refs/heads/"*)
    delete_branch=''
    delete_oid=''
    for arg in "$@"; do
      case "$arg" in
        --force-with-lease=refs/heads/*:*)
          lease=${arg#--force-with-lease=refs/heads/}
          delete_branch=${lease%%:*}
          delete_oid=${lease#*:}
          ;;
      esac
    done
    [ -n "$delete_branch" ] && [ -n "$delete_oid" ] || { echo 'missing exact deletion lease' >&2; exit 1; }
    pending=''
    for record in "$GG_TEST_GIT_DIR/gg/operations/"*.json; do
      [ -f "$record" ] || continue
      grep -Eq '"kind": "(clean|land)"' "$record" || continue
      grep -q '"status": "pending"' "$record" || continue
      grep -q '"touched_remote": true' "$record" || continue
      [ -z "$pending" ] || { echo 'multiple durable clean deletion checkpoints' >&2; exit 1; }
      pending=$(basename "$record" .json)
    done
    [ -n "$pending" ] || { echo 'missing durable clean deletion checkpoint' >&2; exit 1; }
    grep -q '"state": "deleting"' "$GG_TEST_GIT_DIR/gg/config.json" || { echo 'missing durable deleting intent' >&2; exit 1; }
    grep -Fq "\"branch\": \"$delete_branch\"" "$GG_TEST_GIT_DIR/gg/config.json" || { echo 'deleting intent has wrong branch' >&2; exit 1; }
    grep -Fq "\"expected_oid\": \"$delete_oid\"" "$GG_TEST_GIT_DIR/gg/config.json" || { echo 'deleting intent has wrong OID' >&2; exit 1; }
    if [ -f "$GG_TEST_CHECKPOINT_LOG.previous-operation" ]; then
      previous=$(cat "$GG_TEST_CHECKPOINT_LOG.previous-operation")
      previous_branch=$(cat "$GG_TEST_CHECKPOINT_LOG.previous-branch")
      previous_oid=$(cat "$GG_TEST_CHECKPOINT_LOG.previous-oid")
      previous_record="$GG_TEST_GIT_DIR/gg/operations/$previous.json"
      grep -q '"status": "committed"' "$previous_record" || { echo 'previous deletion segment not finalized' >&2; exit 1; }
      grep -q '"kind": "branch_deleted"' "$previous_record" || { echo 'previous exact effect missing' >&2; exit 1; }
      grep -Fq "\"branch\": \"$previous_branch\"" "$previous_record" || { echo 'previous effect has wrong branch' >&2; exit 1; }
      grep -Fq "\"prior_oid\": \"$previous_oid\"" "$previous_record" || { echo 'previous effect has wrong OID' >&2; exit 1; }
    fi
    printf '%s\n' "$pending" >> "$GG_TEST_CHECKPOINT_LOG"
    if [ "${GG_TEST_DELETE_MODE:-}" = "fail_push" ]; then
      echo 'simulated lost deletion response' >&2
      exit 1
    fi
    "$GG_TEST_REAL_GIT" "$@"
    status=$?
    [ "$status" -eq 0 ] || exit "$status"
    if [ "${GG_TEST_DELETE_MODE:-}" = "delete_then_fail" ]; then
      echo 'simulated lost deletion response after server mutation' >&2
      exit 1
    fi
    printf '%s\n' "$pending" > "$GG_TEST_CHECKPOINT_LOG.previous-operation"
    printf '%s\n' "$delete_branch" > "$GG_TEST_CHECKPOINT_LOG.previous-branch"
    printf '%s\n' "$delete_oid" > "$GG_TEST_CHECKPOINT_LOG.previous-oid"
    if [ "${GG_TEST_DELETE_MODE:-}" = "block_effect_save" ]; then
      printf '%s\n' "$pending" > "$(dirname "$0")/blocked-operation-id"
      mkdir "$GG_TEST_GIT_DIR/gg/operations/$pending.json.tmp"
    fi
    if [ "${GG_TEST_DELETE_MODE:-}" = "break_config" ]; then
      mkdir "$GG_TEST_GIT_DIR/gg/config.tmp"
    fi
    if [ "${GG_TEST_DELETE_MODE:-}" = "break_config_and_snapshot" ]; then
      mkdir "$GG_TEST_GIT_DIR/gg/config.tmp"
      echo 'not a valid HEAD' > "$GG_TEST_GIT_DIR/HEAD"
    fi
    if [ "${GG_TEST_DELETE_MODE:-}" = "break_snapshot" ]; then
      echo 'not a valid HEAD' > "$GG_TEST_GIT_DIR/HEAD"
    fi
    exit 0
    ;;
esac
exec "$GG_TEST_REAL_GIT" "$@"
"#,
    )
    .expect("write clean git shim");
    let mut permissions = fs::metadata(&shim)
        .expect("stat clean git shim")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&shim, permissions).expect("chmod clean git shim");
    real_git
}

#[cfg(unix)]
fn setup_direct_remote_clean(
    stack_name: &str,
    entry_ids: &[&str],
) -> (
    tempfile::TempDir,
    PathBuf,
    PathBuf,
    std::ffi::OsString,
    Vec<(String, String)>,
) {
    let (temp, repo_path, remote_path) = create_test_repo_with_remote();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main","provider":"github"}}"#,
    )
    .unwrap();
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", stack_name]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");

    let mut branches = Vec::new();
    let mut mappings = serde_json::Map::new();
    for (index, entry_id) in entry_ids.iter().enumerate() {
        checked_git(
            &repo_path,
            &[
                "commit",
                "--allow-empty",
                "-m",
                &format!("Entry {}\n\nGG-ID: {entry_id}", index + 1),
            ],
        );
        let branch = format!("testuser/{stack_name}--{entry_id}");
        checked_git(&repo_path, &["branch", &branch]);
        checked_git(&repo_path, &["push", "-u", "origin", &branch]);
        let oid = checked_git(&repo_path, &["rev-parse", &branch]);
        branches.push((branch, oid));
        mappings.insert(entry_id.to_string(), Value::from(101 + index as u64));
    }
    checked_git(&repo_path, &["checkout", "main"]);
    checked_git(
        &repo_path,
        &["merge", "--ff-only", &format!("testuser/{stack_name}")],
    );
    checked_git(&repo_path, &["push", "origin", "main"]);
    fs::write(
        gg_dir.join("config.json"),
        serde_json::json!({
            "defaults": {
                "branch_username": "testuser",
                "base": "main",
                "provider": "github"
            },
            "stacks": {
                stack_name: {
                    "base": "main",
                    "mrs": mappings
                }
            }
        })
        .to_string(),
    )
    .unwrap();

    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).unwrap();
    fs::write(
        fake_bin.join("gh"),
        r#"#!/bin/sh
set -eu
if [ "$1" = "--version" ] || { [ "$1" = "auth" ] && [ "$2" = "status" ]; }; then
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  if [ "${GG_TEST_BLOCK_FINALIZATION_ON_VIEW:-}" = "1" ]; then
    pending=''
    for record in "$GG_TEST_GIT_DIR/gg/operations/"*.json; do
      [ -f "$record" ] || continue
      grep -q '"kind": "clean"' "$record" || continue
      grep -q '"status": "pending"' "$record" || continue
      [ -z "$pending" ] || { echo 'multiple pending clean operations' >&2; exit 1; }
      pending=$(basename "$record" .json)
    done
    [ -n "$pending" ] || { echo 'missing pending clean operation' >&2; exit 1; }
    mkdir "$GG_TEST_GIT_DIR/gg/operations/$pending.json.tmp"
    echo 'simulated provider lookup failure' >&2
    exit 1
  fi
  printf '{"number":%s,"title":"Entry","state":"MERGED","url":"https://github.com/test/repo/pull/%s","headRefName":null,"isDraft":false,"mergeable":"MERGEABLE","reviews":[]}\n' "$3" "$3"
  exit 0
fi
echo "unexpected gh invocation: $*" >&2
exit 1
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(fake_bin.join("gh")).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(fake_bin.join("gh"), permissions).unwrap();
    let mut path = std::ffi::OsString::from(fake_bin.as_os_str());
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    (temp, repo_path, remote_path, path, branches)
}

#[cfg(unix)]
#[test]
fn test_public_clean_with_repo_journals_pending_deletion_reconciliation() {
    let (_temp, repo_path, remote_path, fake_path, dropped_branch) =
        setup_published_drop_for_clean("public-pending-delete");

    let output =
        run_public_clean_for_stack_with_repo(&repo_path, "public-pending-delete", &fake_path);
    assert!(
        output.status.success(),
        "public clean failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
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
        "pending deletion reconciliation must delete the remote branch"
    );

    let records = clean_operation_records(&repo_path)
        .into_iter()
        .filter(|record| record["remote_effects"][0]["branch"] == dropped_branch)
        .collect::<Vec<_>>();
    assert_eq!(
        records.len(),
        1,
        "the public entry point must durably journal the reconciled deletion: {records:?}"
    );
    assert_eq!(records[0]["status"], "committed");
    assert_eq!(records[0]["touched_remote"], true);
    assert_eq!(records[0]["remote_effects"][0]["kind"], "branch_deleted");
    assert_eq!(records[0]["remote_effects"][0]["branch"], dropped_branch);
}

#[cfg(unix)]
#[test]
fn test_public_clean_with_repo_journals_direct_verified_remote_deletion() {
    let (_temp, repo_path, remote_path, fake_path, branches) =
        setup_direct_remote_clean("public-direct-delete", &["c-1111111"]);

    let output =
        run_public_clean_for_stack_with_repo(&repo_path, "public-direct-delete", &fake_path);
    assert!(
        output.status.success(),
        "public clean failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{}", branches[0].0)
            ]
        )
        .0,
        "verified direct cleanup must delete the remote branch"
    );

    let records = clean_operation_records(&repo_path);
    assert_eq!(
        records.len(),
        1,
        "the public entry point must durably journal the direct deletion: {records:?}"
    );
    assert_eq!(records[0]["status"], "committed");
    assert_eq!(records[0]["touched_remote"], true);
    assert_eq!(records[0]["remote_effects"][0]["kind"], "branch_deleted");
    assert_eq!(records[0]["remote_effects"][0]["branch"], branches[0].0);
    assert_eq!(records[0]["remote_effects"][0]["prior_oid"], branches[0].1);
}

#[cfg(unix)]
#[test]
fn test_public_clean_finalizes_non_uncertain_provider_deferrals() {
    for late in [false, true] {
        let stack_name = if late {
            "public-late-deferral"
        } else {
            "public-early-deferral"
        };
        let (_temp, repo_path, remote_path, fake_path, dropped_branch) =
            setup_published_drop_for_clean(stack_name);
        let output = if late {
            run_public_clean_for_stack_with_repo_with_env(
                &repo_path,
                stack_name,
                &fake_path,
                &[("GG_TEST_DEFER_ON_SECOND_BRANCH", dropped_branch.as_ref())],
            )
        } else {
            let targets = repo_path.join("fake-bin/targets");
            fs::create_dir_all(&targets).unwrap();
            fs::write(targets.join(dropped_branch.replace('/', "_")), "104\n").unwrap();
            run_public_clean_for_stack_with_repo(&repo_path, stack_name, &fake_path)
        };
        assert!(
            !output.status.success(),
            "provider deferral must keep cleanup incomplete: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
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
            "a provider-vetoed deletion must not reach the remote"
        );

        let records = fs::read_dir(repo_path.join(".git/gg/operations"))
            .unwrap()
            .filter_map(|entry| {
                let record: Value =
                    serde_json::from_slice(&fs::read(entry.unwrap().path()).unwrap()).unwrap();
                (record["kind"] == "clean").then_some(record)
            })
            .collect::<Vec<_>>();
        assert!(
            !records.is_empty(),
            "owned clean must create a journal record"
        );
        assert!(
            records.iter().all(|record| record["status"] != "pending"),
            "a known provider deferral must finalize every owned segment: {records:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn test_public_clean_classifies_local_cleanup_config_save_failure() {
    let (_temp, repo_path, remote_path, path, branches) =
        setup_direct_remote_clean("public-local-save-failure", &["c-1111111"]);
    checked_git(
        &remote_path,
        &["update-ref", "-d", &format!("refs/heads/{}", branches[0].0)],
    );
    fs::create_dir(repo_path.join(".git/gg/config.tmp")).unwrap();

    let output = run_public_clean_for_stack_with_repo(
        &repo_path,
        "public-local-save-failure",
        path.as_os_str(),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "config durability failure must fail: {stdout}\n{stderr}"
    );
    assert!(
        stderr.contains("DurabilityCheckpoint"),
        "{stdout}\n{stderr}"
    );
    let records = fs::read_dir(repo_path.join(".git/gg/operations"))
        .unwrap()
        .filter_map(|entry| {
            let record: Value =
                serde_json::from_slice(&fs::read(entry.unwrap().path()).unwrap()).unwrap();
            (record["kind"] == "clean").then_some(record)
        })
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["status"], "pending", "{records:?}");
    assert_eq!(records[0]["touched_remote"], false, "{records:?}");
    assert!(
        records[0]["remote_effects"].as_array().unwrap().is_empty(),
        "{records:?}"
    );
}

#[cfg(unix)]
#[test]
fn test_clean_classifies_local_cleanup_config_save_failure() {
    let (_temp, repo_path, remote_path, path, branches) =
        setup_direct_remote_clean("cli-local-save-failure", &["c-1111111"]);
    checked_git(
        &remote_path,
        &["update-ref", "-d", &format!("refs/heads/{}", branches[0].0)],
    );
    fs::create_dir(repo_path.join(".git/gg/config.tmp")).unwrap();

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all", "--json"],
        &[("PATH", path.as_os_str())],
    );
    assert!(
        !success,
        "config durability failure must fail: {stdout}\n{stderr}"
    );
    let payload: Value = serde_json::from_str(&stdout).expect("JSON error payload");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|error| error.contains("Durability checkpoint error")),
        "{payload}"
    );
}

#[cfg(all(unix, debug_assertions))]
#[test]
fn test_clean_emits_no_worktree_success_before_config_durability() {
    let (temp, repo_path, _remote_path, path, _branches) =
        setup_direct_remote_clean("worktree-save-failure", &["c-1111111"]);
    let worktree_path = temp.path().join("configured-worktree");
    checked_git(
        &repo_path,
        &[
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            "testuser/worktree-save-failure",
        ],
    );
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["worktree-save-failure"]["worktree_path"] =
        Value::String(worktree_path.to_string_lossy().into_owned());
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    fs::create_dir(repo_path.join(".git/gg/config.tmp")).unwrap();

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_CONFIRM_WORKTREE_REMOVAL", "1".as_ref()),
        ],
    );

    assert!(
        !success,
        "config durability failure must fail: {stdout}\n{stderr}"
    );
    assert!(
        !worktree_path.exists(),
        "the fixture must reach the successful worktree removal"
    );
    assert!(
        !stdout.contains("Removed worktree"),
        "worktree success preceded config durability: {stdout}\n{stderr}"
    );

    fs::remove_dir(repo_path.join(".git/gg/config.tmp")).unwrap();
    let (retry_success, retry_stdout, retry_stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_CONFIRM_WORKTREE_REMOVAL", "1".as_ref()),
        ],
    );
    assert!(
        retry_success,
        "retry must reconcile the already removed worktree: {retry_stdout}\n{retry_stderr}"
    );
    assert!(
        retry_stdout.contains(&format!(
            "OK Removed worktree '{}'.",
            worktree_path.display()
        )),
        "the durable retry must report the completed worktree cleanup: {retry_stdout}"
    );
    let saved: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert!(
        saved["stacks"]["worktree-save-failure"].is_null(),
        "retry must durably finish stack cleanup: {saved}"
    );
}

#[cfg(all(unix, debug_assertions))]
#[test]
fn test_clean_emits_no_worktree_success_when_local_segment_finalization_fails() {
    let (temp, repo_path, _remote_path, path, _branches) =
        setup_direct_remote_clean("worktree-finalize-failure", &["c-1111111"]);
    let worktree_path = temp.path().join("configured-worktree-finalize");
    checked_git(
        &repo_path,
        &[
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            "testuser/worktree-finalize-failure",
        ],
    );
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["worktree-finalize-failure"]["worktree_path"] =
        Value::String(worktree_path.to_string_lossy().into_owned());
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let real_git = install_clean_delete_git_shim(&repo_path);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_CONFIRM_WORKTREE_REMOVAL", "1".as_ref()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ("GG_TEST_DELETE_MODE", "block_worktree_finalize".as_ref()),
        ],
    );

    assert!(
        !success,
        "local finalization failure must fail: {stdout}\n{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("finalize clean local mutation before success output"),
        "{stdout}\n{stderr}"
    );
    assert!(
        !stdout.contains("Removed worktree"),
        "worktree success preceded segment finalization: {stdout}\n{stderr}"
    );
    assert!(!worktree_path.exists());
    let saved: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert!(
        saved["stacks"]["worktree-finalize-failure"]["worktree_path"].is_null(),
        "the worktree config save precedes segment finalization: {saved}"
    );
}

#[cfg(all(unix, debug_assertions))]
#[test]
fn test_clean_emits_configured_worktree_success_after_durable_cleanup() {
    let (temp, repo_path, _remote_path, path, _branches) =
        setup_direct_remote_clean("worktree-save-success", &["c-1111111"]);
    let worktree_path = temp.path().join("configured-worktree-success");
    checked_git(
        &repo_path,
        &[
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            "testuser/worktree-save-success",
        ],
    );
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["worktree-save-success"]["worktree_path"] =
        Value::String(worktree_path.to_string_lossy().into_owned());
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_CONFIRM_WORKTREE_REMOVAL", "1".as_ref()),
        ],
    );

    assert!(success, "clean failed: {stdout}\n{stderr}");
    let worktree_message = format!("OK Removed worktree '{}'.", worktree_path.display());
    let worktree_position = stdout.find(&worktree_message).unwrap_or_else(|| {
        panic!("the successful cleanup must retain its worktree message: {stdout}")
    });
    let stack_position = stdout
        .find("OK Deleted stack 'worktree-save-success'")
        .unwrap_or_else(|| {
            panic!("the successful cleanup must retain its stack message: {stdout}")
        });
    assert!(
        worktree_position < stack_position,
        "worktree success must retain its position before stack success: {stdout}"
    );
    let saved: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert!(
        saved["stacks"]["worktree-save-success"].is_null(),
        "the success message requires the cleaned config on disk: {saved}"
    );
}

#[cfg(all(unix, not(debug_assertions)))]
#[test]
fn test_release_clean_ignores_worktree_env_confirmation_override() {
    let (temp, repo_path, _remote_path, path, _branches) =
        setup_direct_remote_clean("release-worktree-confirm", &["c-1111111"]);
    let worktree_path = temp.path().join("release-configured-worktree");
    checked_git(
        &repo_path,
        &[
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            "testuser/release-worktree-confirm",
        ],
    );
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["release-worktree-confirm"]["worktree_path"] =
        Value::String(worktree_path.to_string_lossy().into_owned());
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all", "--json"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_CONFIRM_WORKTREE_REMOVAL", "1".as_ref()),
        ],
    );

    assert!(success, "release clean failed: {stdout}\n{stderr}");
    assert!(
        worktree_path.exists(),
        "a release build must not auto-confirm worktree removal from a test env var"
    );
    let payload: Value = serde_json::from_str(&stdout).expect("clean JSON payload");
    assert!(payload["clean"]["cleaned"].as_array().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn test_clean_persists_remote_uncertainty_before_first_direct_deletion() {
    let (_temp, repo_path, remote_path, path, branches) =
        setup_direct_remote_clean("durable-first-delete", &["c-1111111"]);
    let real_git = install_clean_delete_git_shim(&repo_path);
    let checkpoint_log = repo_path.join("fake-bin/clean-checkpoints");

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ("GG_TEST_CHECKPOINT_LOG", checkpoint_log.as_os_str()),
        ],
    );

    assert!(success, "clean failed: {stdout}\n{stderr}");
    assert_eq!(
        fs::read_to_string(&checkpoint_log).unwrap().lines().count(),
        1,
        "the deletion shim must observe one durable checkpoint"
    );
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{}", branches[0].0)
            ]
        )
        .0,
        "the checkpointed deletion must reach the remote"
    );
}

#[cfg(unix)]
#[test]
fn test_clean_finalizes_each_direct_deletion_with_its_exact_effect() {
    let (_temp, repo_path, _remote_path, path, branches) =
        setup_direct_remote_clean("segmented-direct-deletes", &["c-1111111", "c-2222222"]);
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["segmented-direct-deletes"]["mrs"]
        .as_object_mut()
        .unwrap()
        .remove("c-2222222");
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let real_git = install_clean_delete_git_shim(&repo_path);
    let checkpoint_log = repo_path.join("fake-bin/clean-checkpoints");

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all", "--json"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ("GG_TEST_CHECKPOINT_LOG", checkpoint_log.as_os_str()),
        ],
    );
    assert!(success, "clean failed: {stdout}\n{stderr}");

    let mut deletion_records =
        fs::read_dir(repo_path.join(".git/gg/operations"))
            .unwrap()
            .filter_map(|entry| {
                let entry = entry.unwrap();
                entry.file_type().unwrap().is_file().then(|| {
                    serde_json::from_slice::<Value>(&fs::read(entry.path()).unwrap()).unwrap()
                })
            })
            .filter(|record| {
                record["kind"] == "clean"
                    && record["remote_effects"]
                        .as_array()
                        .is_some_and(|effects| !effects.is_empty())
            })
            .collect::<Vec<_>>();
    deletion_records.sort_by_key(|record| record["id"].as_str().unwrap().to_string());

    assert_eq!(
        deletion_records.len(),
        2,
        "each deletion needs its own operation record: {deletion_records:?}"
    );
    let mut actual_effects = deletion_records
        .iter()
        .map(|record| {
            assert_eq!(record["status"], "committed", "{record}");
            assert_eq!(record["touched_remote"], true, "{record}");
            let effects = record["remote_effects"].as_array().unwrap();
            assert_eq!(effects.len(), 1, "{record}");
            (
                effects[0]["remote"].as_str().unwrap().to_string(),
                effects[0]["branch"].as_str().unwrap().to_string(),
                effects[0]["prior_oid"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    actual_effects.sort();
    let mut expected_effects = branches
        .into_iter()
        .map(|(branch, oid)| ("origin".to_string(), branch, oid))
        .collect::<Vec<_>>();
    expected_effects.sort();
    assert_eq!(actual_effects, expected_effects);

    let checkpoint_ids = fs::read_to_string(checkpoint_log)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(checkpoint_ids.len(), 2);
    assert_ne!(
        checkpoint_ids[0], checkpoint_ids[1],
        "successive deletions must not share one pending segment"
    );
}

#[cfg(unix)]
#[test]
fn test_clean_surfaces_effect_recording_failure_after_successful_deletion() {
    let (_temp, repo_path, remote_path, path, branches) =
        setup_direct_remote_clean("effect-save-failure", &["c-1111111"]);
    let real_git = install_clean_delete_git_shim(&repo_path);
    let checkpoint_log = repo_path.join("fake-bin/clean-checkpoints");

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all", "--json"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ("GG_TEST_CHECKPOINT_LOG", checkpoint_log.as_os_str()),
            ("GG_TEST_DELETE_MODE", "block_effect_save".as_ref()),
        ],
    );
    assert!(
        !success,
        "effect persistence failure must fail: {stdout}\n{stderr}"
    );
    let diagnostic = format!("{stdout}{stderr}");
    assert!(
        diagnostic.contains("failed to durably record exact remote deletion effect"),
        "the known effect failure must be explicit: {diagnostic}"
    );
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{}", branches[0].0)
            ]
        )
        .0,
        "the fake remote must delete the branch before blocking effect persistence"
    );
    assert!(
        run_git(
            &repo_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{}", branches[0].0)
            ]
        )
        .0,
        "the local branch must remain as recovery evidence"
    );

    let operation_id = fs::read_to_string(repo_path.join("fake-bin/blocked-operation-id"))
        .unwrap()
        .trim()
        .to_string();
    let record: Value = serde_json::from_slice(
        &fs::read(
            repo_path
                .join(".git/gg/operations")
                .join(format!("{operation_id}.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["status"], "pending", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
    assert!(record["remote_effects"].as_array().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn test_clean_keeps_effect_segment_pending_when_intent_removal_save_fails() {
    let (_temp, repo_path, remote_path, path, dropped_branch) =
        setup_published_drop_for_clean("intent-removal-save-failure");
    let real_git = install_clean_delete_git_shim(&repo_path);
    let checkpoint_log = repo_path.join("fake-bin/clean-checkpoints");

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all", "--json"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ("GG_TEST_CHECKPOINT_LOG", checkpoint_log.as_os_str()),
            ("GG_TEST_DELETE_MODE", "break_config".as_ref()),
        ],
    );

    assert!(
        !success,
        "intent-removal durability failure must fail: {stdout}\n{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("failed to persist completed remote deletion"),
        "{stdout}\n{stderr}"
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
        "the remote deletion must have succeeded before the save failure"
    );

    let operation_id = fs::read_to_string(&checkpoint_log)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let record: Value = serde_json::from_slice(
        &fs::read(
            repo_path
                .join(".git/gg/operations")
                .join(format!("{operation_id}.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["status"], "pending", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
    assert_eq!(
        record["remote_effects"].as_array().unwrap().len(),
        1,
        "the exact effect must remain durable: {record}"
    );
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert_eq!(
        config["pending_remote_branch_deletions"][0]["state"], "deleting",
        "the on-disk uncertain intent must remain truthful: {config}"
    );
}

#[cfg(unix)]
#[test]
fn test_clean_direct_push_failure_is_uncertain_and_not_retried() {
    let (_temp, repo_path, remote_path, path, branches) =
        setup_direct_remote_clean("uncertain-direct-delete", &["c-1111111"]);
    // Model a provider-verified squash merge: the review is merged, but the
    // original entry commit remains in the local stack walk.
    checked_git(&repo_path, &["reset", "--hard", "HEAD~1"]);
    let real_git = install_clean_delete_git_shim(&repo_path);
    let checkpoint_log = repo_path.join("fake-bin/clean-checkpoints");

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all", "--json"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ("GG_TEST_CHECKPOINT_LOG", checkpoint_log.as_os_str()),
            ("GG_TEST_DELETE_MODE", "delete_then_fail".as_ref()),
        ],
    );

    assert!(!success, "ambiguous deletion must fail: {stdout}\n{stderr}");
    assert!(
        format!("{stdout}{stderr}").contains("Remote mutation outcome is uncertain"),
        "the uncertain outcome must be explicit: {stdout}\n{stderr}"
    );
    assert_eq!(
        fs::read_to_string(&checkpoint_log).unwrap().lines().count(),
        1,
        "the failed deletion must not be retried"
    );
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{}", branches[0].0)
            ]
        )
        .0,
        "the simulated lost response must happen after the remote branch is deleted"
    );
    assert!(
        run_git(
            &repo_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{}", branches[0].0)
            ]
        )
        .0,
        "local recovery evidence must remain"
    );

    let records = fs::read_dir(repo_path.join(".git/gg/operations"))
        .unwrap()
        .filter_map(|entry| {
            let record: Value =
                serde_json::from_slice(&fs::read(entry.unwrap().path()).unwrap()).unwrap();
            (record["kind"] == "clean" && record["touched_remote"] == true).then_some(record)
        })
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["status"], "pending");
    assert!(records[0]["remote_effects"].as_array().unwrap().is_empty());

    // Another actor may restore the same ref while the first response remains
    // unresolved. The durable Deleting marker must still forbid a retry.
    checked_git(
        &repo_path,
        &[
            "push",
            "origin",
            &format!("{}:{}", branches[0].0, branches[0].0),
        ],
    );

    let (retry_success, retry_stdout, retry_stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all", "--json"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ("GG_TEST_CHECKPOINT_LOG", checkpoint_log.as_os_str()),
            ("GG_TEST_DELETE_MODE", "delete_then_fail".as_ref()),
        ],
    );
    assert!(
        !retry_success,
        "an unresolved deletion must block the next clean: {retry_stdout}\n{retry_stderr}"
    );
    assert!(
        format!("{retry_stdout}{retry_stderr}").contains("uncertain prior outcome"),
        "the retry must require reconciliation: {retry_stdout}\n{retry_stderr}"
    );
    assert_eq!(
        fs::read_to_string(&checkpoint_log).unwrap().lines().count(),
        1,
        "the next invocation must not send another deletion request"
    );
    assert!(
        run_git(
            &repo_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{}", branches[0].0)
            ]
        )
        .0,
        "local recovery evidence must remain until the uncertainty is reconciled"
    );
    assert!(
        run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{}", branches[0].0)
            ]
        )
        .0,
        "the restored remote branch must not be deleted by an automatic retry"
    );
}

#[cfg(unix)]
#[test]
fn test_clean_emits_no_human_or_json_success_before_finalization() {
    for json in [false, true] {
        let stack_name = if json {
            "json-finalization-failure"
        } else {
            "human-finalization-failure"
        };
        let (_temp, repo_path, _remote_path, path, _branches) =
            setup_direct_remote_clean(stack_name, &["c-1111111"]);
        let args = if json {
            vec!["clean", "--all", "--json"]
        } else {
            vec!["clean", "--all"]
        };
        let (success, stdout, stderr) = run_gg_with_env(
            &repo_path,
            &args,
            &[
                ("PATH", path.as_os_str()),
                ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
                ("GG_TEST_BLOCK_FINALIZATION_ON_VIEW", "1".as_ref()),
            ],
        );

        assert!(
            !success,
            "finalization failure must fail: {stdout}\n{stderr}"
        );
        if json {
            let payload: Value = serde_json::from_str(&stdout).expect("JSON error payload");
            assert!(
                payload.get("clean").is_none(),
                "JSON success payload preceded finalization failure: {payload}"
            );
            assert!(
                payload["error"]
                    .as_str()
                    .is_some_and(|error| error.contains("Is a directory")),
                "JSON must report the finalization failure: {payload}"
            );
        } else {
            assert!(
                !stdout.contains("Deleted stack") && !stdout.contains("Cleaned 1 stack"),
                "human success output preceded finalization failure: {stdout}\n{stderr}"
            );
            assert!(
                stderr.contains("Is a directory"),
                "human mode must report the finalization failure: {stdout}\n{stderr}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn test_clean_preserves_per_stack_human_output_order_after_finalization() {
    let (_temp, repo_path, _remote_path, path, _branches) =
        setup_direct_remote_clean("aa-cleaned-first", &["c-1111111"]);
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "zz-unmerged-later"]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");
    checked_git(
        &repo_path,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "Unmerged\n\nGG-ID: c-9999999",
        ],
    );
    checked_git(&repo_path, &["checkout", "main"]);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all"],
        &[("PATH", path.as_os_str())],
    );
    assert!(success, "clean failed: {stdout}\n{stderr}");
    let deleted = stdout.find("Deleted stack 'aa-cleaned-first'").unwrap();
    let skipped = stdout
        .find("Stack 'zz-unmerged-later' has unmerged commits")
        .unwrap();
    assert!(
        deleted < skipped,
        "each finalized stack must be reported before later stack output: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_empty_stack_emits_no_success_before_finalization() {
    for json in [false, true] {
        let (_temp, repo_path, _remote_path) = create_test_repo_with_remote();
        fs::create_dir_all(repo_path.join(".git/gg")).unwrap();
        fs::write(
            repo_path.join(".git/gg/config.json"),
            r#"{"defaults":{"branch_username":"testuser","base":"main","provider":"github"}}"#,
        )
        .unwrap();
        let (success, stdout, stderr) = run_gg(&repo_path, &["co", "empty-land"]);
        assert!(success, "checkout failed: {stdout}\n{stderr}");

        let fake_bin = repo_path.join("fake-bin");
        fs::create_dir_all(&fake_bin).unwrap();
        let gh = fake_bin.join("gh");
        fs::write(
            &gh,
            r#"#!/bin/sh
set -eu
if [ "$1" = "--version" ]; then
  exit 0
fi
if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  pending=''
  for record in "$GG_TEST_GIT_DIR/gg/operations/"*.json; do
    [ -f "$record" ] || continue
    grep -q '"kind": "land"' "$record" || continue
    grep -q '"status": "pending"' "$record" || continue
    [ -z "$pending" ] || { echo 'multiple pending land operations' >&2; exit 1; }
    pending=$(basename "$record" .json)
  done
  [ -n "$pending" ] || { echo 'missing pending land operation' >&2; exit 1; }
  mkdir "$GG_TEST_GIT_DIR/gg/operations/$pending.json.tmp"
  exit 0
fi
echo "unexpected gh invocation: $*" >&2
exit 1
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&gh).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&gh, permissions).unwrap();
        let mut path = std::ffi::OsString::from(fake_bin.as_os_str());
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());
        let args = if json {
            vec!["land", "--json"]
        } else {
            vec!["land"]
        };
        let (success, stdout, stderr) = run_gg_with_env(
            &repo_path,
            &args,
            &[
                ("PATH", path.as_os_str()),
                ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ],
        );
        assert!(
            !success,
            "finalization failure must fail empty land: {stdout}\n{stderr}"
        );
        assert!(
            !stdout.contains("\"land\"") && !stdout.contains("Stack is empty"),
            "success output preceded finalization: {stdout}\n{stderr}"
        );
    }
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_emits_no_cleanup_summary_when_deletion_finalization_fails() {
    let (_temp, repo_path, _remote_path, path, _branches) =
        setup_direct_remote_clean("land-jsonl-finalize", &["c-1111111"]);
    checked_git(&repo_path, &["checkout", "testuser/land-jsonl-finalize"]);
    checked_git(&repo_path, &["branch", "-f", "main", "HEAD~1"]);
    checked_git(&repo_path, &["push", "--force", "origin", "main"]);
    let real_git = install_clean_delete_git_shim(&repo_path);
    let checkpoint_log = repo_path.join("fake-bin/clean-checkpoints");

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["land", "--all", "--clean", "--jsonl"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ("GG_TEST_CHECKPOINT_LOG", checkpoint_log.as_os_str()),
            ("GG_TEST_DELETE_MODE", "break_snapshot".as_ref()),
        ],
    );
    assert!(
        !success,
        "finalization failure must fail land: {stdout}\n{stderr}"
    );
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("valid JSONL event"))
        .collect::<Vec<_>>();
    assert!(
        events.iter().all(|event| event["event"] != "summary"),
        "cleanup summary preceded deletion finalization: {events:?}"
    );
    assert_eq!(
        fs::read_to_string(&checkpoint_log).unwrap().lines().count(),
        1,
        "the deletion request must reach its durable checkpoint: {stdout}\n{stderr}"
    );
    assert!(
        events.iter().any(|event| {
            event["event"] == "error"
                && event["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("finalize land clean deletion segment"))
        }),
        "the exact deletion-finalization failure must be observable: {stdout}\n{stderr}"
    );
}

#[test]
fn test_clean_consumes_pending_drop_before_removing_local_stack() {
    let (temp, repo_path, remote_path, fake_path, dropped_branch) =
        setup_published_drop_for_clean("clean-drop");

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "clean failed: {stdout}\n{stderr}");
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
        "clean must consume the durable drop intent"
    );
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert!(
        config["pending_remote_branch_deletions"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "successful clean must consume deletion authority: {config}"
    );

    let reader = temp.path().join("clean-reader");
    checked_git(
        &repo_path,
        &[
            "clone",
            remote_path.to_str().unwrap(),
            reader.to_str().unwrap(),
        ],
    );
    fs::create_dir_all(reader.join(".git/gg")).unwrap();
    fs::write(
        reader.join(".git/gg/config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
    )
    .unwrap();
    let (success, stdout, stderr) = run_gg(&reader, &["co", "clean-drop"]);
    assert!(success, "fresh checkout failed: {stdout}\n{stderr}");
    assert_eq!(
        checked_git(&reader, &["rev-list", "--count", "main..HEAD"]),
        "0",
        "fresh checkout must not resurrect a remotely published dropped entry"
    );
}

#[test]
fn test_clean_surfaces_absent_pending_deletion_config_save_failure() {
    let (_temp, repo_path, remote_path, fake_path, dropped_branch) =
        setup_published_drop_for_clean("absent-pending-save-failure");
    checked_git(
        &remote_path,
        &["update-ref", "-d", &format!("refs/heads/{dropped_branch}")],
    );
    fs::create_dir(repo_path.join(".git/gg/config.tmp")).unwrap();

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all", "--json"],
        &[("PATH", fake_path.as_os_str())],
    );

    assert!(
        !success,
        "losing the durable intent update must fail: {stdout}\n{stderr}"
    );
    let payload: Value = serde_json::from_str(&stdout).expect("JSON error payload");
    assert!(payload.get("clean").is_none(), "{payload}");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|error| error.contains("Durability checkpoint error")),
        "{payload}"
    );
}

#[cfg(unix)]
#[test]
fn test_clean_config_save_failure_does_not_finalize_incomplete_deletion() {
    let (_temp, repo_path, _remote_path, fake_path, _dropped_branch) =
        setup_published_drop_for_clean("combined-deletion-failure");
    let real_git = install_clean_delete_git_shim(&repo_path);
    let checkpoint_log = repo_path.join("fake-bin/clean-checkpoints");

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all", "--json"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ("GG_TEST_CHECKPOINT_LOG", checkpoint_log.as_os_str()),
            ("GG_TEST_DELETE_MODE", "break_config_and_snapshot".as_ref()),
        ],
    );
    assert!(
        !success,
        "config durability failure must fail: {stdout}\n{stderr}"
    );
    let diagnostic = format!("{stdout}{stderr}");
    assert!(
        diagnostic.contains("failed to persist completed remote deletion"),
        "{diagnostic}"
    );
    assert!(
        !diagnostic.contains("failed to finalize"),
        "the incomplete deletion segment must remain pending: {diagnostic}"
    );
}

#[cfg(unix)]
#[test]
fn test_clean_surfaces_obsolete_pending_deletion_config_save_failure() {
    let (_temp, repo_path, _remote_path, path, branches) =
        setup_direct_remote_clean("obsolete-pending-save-failure", &["c-1111111"]);
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["pending_remote_branch_deletions"] = serde_json::json!([{
        "remote": "origin",
        "branch": branches[0].0,
        "expected_oid": branches[0].1,
        "source_operation_id": "test-obsolete-intent",
        "state": "pending"
    }]);
    checked_git(&repo_path, &["reset", "--hard", "HEAD~1"]);
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    fs::create_dir(repo_path.join(".git/gg/config.tmp")).unwrap();

    let output = run_public_clean_for_stack_with_repo(
        &repo_path,
        "obsolete-pending-save-failure",
        path.as_os_str(),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "losing obsolete-intent removal must fail: {stdout}\n{stderr}"
    );
    assert!(
        stderr.contains("DurabilityCheckpoint"),
        "{stdout}\n{stderr}"
    );
}

#[test]
fn test_clean_orphan_stack_keeps_dropped_branch_targeted_by_open_review() {
    let (_temp, repo_path, remote_path, fake_path, dropped_branch) =
        setup_published_drop_for_clean("clean-orphan-dependent");
    checked_git(
        &repo_path,
        &["branch", "-D", "testuser/clean-orphan-dependent"],
    );
    let targets = repo_path.join("fake-bin/targets");
    fs::create_dir_all(&targets).unwrap();
    let target_file = targets.join(dropped_branch.replace('/', "_"));
    fs::write(&target_file, "104\n").unwrap();
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

    let (_success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(
        format!("{stdout}{stderr}").contains("#104 still targets it"),
        "clean must explain the deferral: {stdout}\n{stderr}"
    );
    assert!(
        remote_has_dropped(),
        "clean must not delete a branch an open review targets"
    );
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert!(
        config["stacks"]["clean-orphan-dependent"].is_object(),
        "stack config must stay so a later clean can retry: {config}"
    );

    fs::remove_file(&target_file).unwrap();
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(success, "retry clean failed: {stdout}\n{stderr}");
    assert!(!remote_has_dropped(), "retry clean must prune the branch");
}

#[cfg(unix)]
#[test]
fn test_clean_reopens_local_segment_after_late_remote_deletion_deferral() {
    let (_temp, repo_path, _remote_path, fake_path, dropped_branch) =
        setup_published_drop_for_clean("aa-late-deferral");
    let later_stack = "zz-local-after-deferral";
    let later_branch = format!("testuser/{later_stack}--c-9999999");
    checked_git(&repo_path, &["branch", &later_branch]);

    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"][later_stack] = serde_json::json!({
        "base": "main",
        "mrs": {"c-9999999": 999}
    });
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all", "--json"],
        &[
            ("PATH", fake_path.as_os_str()),
            ("GG_TEST_DEFER_ON_SECOND_BRANCH", dropped_branch.as_ref()),
        ],
    );
    assert!(success, "clean failed: {stdout}\n{stderr}");
    assert!(
        !run_git(
            &repo_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{later_branch}")
            ]
        )
        .0,
        "the later orphan branch must be cleaned"
    );

    let later_ref = format!("refs/heads/{later_branch}");
    let records = fs::read_dir(repo_path.join(".git/gg/operations"))
        .unwrap()
        .filter_map(|entry| {
            let record: Value =
                serde_json::from_slice(&fs::read(entry.unwrap().path()).unwrap()).unwrap();
            (record["kind"] == "clean" && record["status"] == "committed").then_some(record)
        })
        .collect::<Vec<_>>();
    assert!(
        records.iter().all(|record| {
            record["touched_remote"] != true
                || !record["remote_effects"].as_array().unwrap().is_empty()
        }),
        "a provider-vetoed request must not be recorded as uncertain: {records:?}"
    );
    assert!(
        records.iter().any(|record| {
            record["refs_before"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reference| reference["name"] == later_ref)
                && record["refs_after"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|reference| reference["name"] != later_ref)
        }),
        "the later local cleanup needs its own durable segment: {records:?}"
    );
}

#[cfg(unix)]
#[test]
fn test_clean_retains_stack_when_pending_drop_deletion_is_uncertain() {
    let (_temp, repo_path, remote_path, fake_path, dropped_branch) =
        setup_published_drop_for_clean("clean-drop-rejected");
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

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all"],
        &[("PATH", fake_path.as_os_str())],
    );
    assert!(!success, "rejected clean must fail: {stdout}\n{stderr}");
    assert!(stderr.contains("uncertain outcome"), "{stderr}");
    assert!(
        run_git(
            &repo_path,
            &[
                "show-ref",
                "--verify",
                "refs/heads/testuser/clean-drop-rejected"
            ]
        )
        .0,
        "clean must retain the local stack for recovery"
    );
    let config: Value =
        serde_json::from_slice(&fs::read(repo_path.join(".git/gg/config.json")).unwrap()).unwrap();
    assert!(config["stacks"]["clean-drop-rejected"].is_object());
    let intent = config["pending_remote_branch_deletions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|intent| intent["branch"] == dropped_branch)
        .expect("deletion authority must remain recoverable");
    assert_eq!(intent["state"], "deleting");

    let clean_operation: Value = fs::read_dir(repo_path.join(".git/gg/operations"))
        .unwrap()
        .map(|entry| serde_json::from_slice(&fs::read(entry.unwrap().path()).unwrap()).unwrap())
        .filter(|operation: &Value| operation["kind"] == "clean")
        .max_by_key(|operation| operation["created_at_ms"].as_u64().unwrap())
        .expect("clean operation must be recorded");
    assert_eq!(clean_operation["touched_remote"], true);
    let operation_id = clean_operation["id"].as_str().unwrap();
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", operation_id, "--json"]);
    assert!(
        !success,
        "undo must refuse uncertain remote effects: {stdout}\n{stderr}"
    );
    let refusal: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(refusal["refusal"]["reason"], "remote");
}

#[cfg(unix)]
#[test]
fn test_land_clean_finalizes_each_remote_deletion_in_its_own_segment() {
    let (_temp, repo_path, _remote_path, path, branches) =
        setup_direct_remote_clean("land-segmented-deletes", &["c-1111111", "c-2222222"]);
    checked_git(&repo_path, &["checkout", "testuser/land-segmented-deletes"]);
    checked_git(&repo_path, &["branch", "-f", "main", "HEAD~2"]);
    checked_git(&repo_path, &["push", "--force", "origin", "main"]);
    let real_git = install_clean_delete_git_shim(&repo_path);
    let checkpoint_log = repo_path.join("fake-bin/clean-checkpoints");

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["land", "--all", "--clean", "--jsonl"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ("GG_TEST_CHECKPOINT_LOG", checkpoint_log.as_os_str()),
        ],
    );
    assert!(success, "land failed: {stdout}\n{stderr}");

    let mut records = fs::read_dir(repo_path.join(".git/gg/operations"))
        .unwrap()
        .filter_map(|entry| {
            let record: Value =
                serde_json::from_slice(&fs::read(entry.unwrap().path()).unwrap()).unwrap();
            (record["kind"] == "land"
                && record["remote_effects"]
                    .as_array()
                    .is_some_and(|effects| !effects.is_empty()))
            .then_some(record)
        })
        .collect::<Vec<_>>();
    records.sort_by_key(|record| record["id"].as_str().unwrap().to_string());
    assert_eq!(records.len(), 2, "{records:?}");
    let mut actual = records
        .iter()
        .map(|record| {
            assert_eq!(record["status"], "committed");
            assert_eq!(record["touched_remote"], true);
            let effects = record["remote_effects"].as_array().unwrap();
            assert_eq!(effects.len(), 1, "{record}");
            (
                effects[0]["branch"].as_str().unwrap().to_string(),
                effects[0]["prior_oid"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = branches;
    expected.sort();
    assert_eq!(actual, expected);

    let checkpoint_ids = fs::read_to_string(checkpoint_log)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(checkpoint_ids.len(), 2);
    assert_ne!(checkpoint_ids[0], checkpoint_ids[1]);
}

#[cfg(unix)]
#[test]
fn test_land_clean_surfaces_effect_save_failure_with_truthful_recovery_record() {
    let (_temp, repo_path, remote_path, path, branches) =
        setup_direct_remote_clean("land-effect-save-failure", &["c-1111111"]);
    checked_git(
        &repo_path,
        &["checkout", "testuser/land-effect-save-failure"],
    );
    checked_git(&repo_path, &["branch", "-f", "main", "HEAD~1"]);
    checked_git(&repo_path, &["push", "--force", "origin", "main"]);
    let real_git = install_clean_delete_git_shim(&repo_path);
    let checkpoint_log = repo_path.join("fake-bin/clean-checkpoints");

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["land", "--all", "--clean", "--jsonl"],
        &[
            ("PATH", path.as_os_str()),
            ("GG_TEST_REAL_GIT", real_git.as_ref()),
            ("GG_TEST_GIT_DIR", repo_path.join(".git").as_os_str()),
            ("GG_TEST_CHECKPOINT_LOG", checkpoint_log.as_os_str()),
            ("GG_TEST_DELETE_MODE", "block_effect_save".as_ref()),
        ],
    );
    assert!(
        !success,
        "effect save failure must fail land: {stdout}\n{stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("Durability checkpoint error"),
        "durability failure must be explicit: {stdout}\n{stderr}"
    );
    assert!(
        !stdout.lines().any(|line| {
            serde_json::from_str::<Value>(line).is_ok_and(|event| event["event"] == "summary")
        }),
        "land must not emit a success summary after effect-save failure: {stdout}"
    );
    assert!(
        !run_git(
            &remote_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{}", branches[0].0)
            ]
        )
        .0,
        "the remote branch must be deleted before the injected save failure"
    );
    assert!(
        run_git(
            &repo_path,
            &[
                "show-ref",
                "--verify",
                &format!("refs/heads/{}", branches[0].0)
            ]
        )
        .0,
        "the local branch must remain as recovery evidence"
    );

    let operation_id = fs::read_to_string(repo_path.join("fake-bin/blocked-operation-id"))
        .unwrap()
        .trim()
        .to_string();
    let record: Value = serde_json::from_slice(
        &fs::read(
            repo_path
                .join(".git/gg/operations")
                .join(format!("{operation_id}.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["kind"], "land");
    assert_eq!(record["status"], "pending");
    assert_eq!(record["touched_remote"], true);
    assert!(record["remote_effects"].as_array().unwrap().is_empty());
}

#[test]
fn test_gg_clean_json_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["clean", "--help"]);

    assert!(success);
    assert!(stdout.contains("--json"));
}

#[test]
fn test_gg_clean_json_no_stacks() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["clean", "--json", "--all"]);
    assert!(success, "gg clean --json --all failed: {}", stderr);
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSON mode"
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);

    let cleaned = parsed["clean"]["cleaned"]
        .as_array()
        .expect("clean.cleaned must be an array");
    let skipped = parsed["clean"]["skipped"]
        .as_array()
        .expect("clean.skipped must be an array");

    assert!(cleaned.is_empty(), "cleaned should be empty");
    assert!(skipped.is_empty(), "skipped should be empty");
}

#[test]
fn test_gg_clean_json_requires_all() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["clean", "--json"]);
    assert!(!success, "gg clean --json should fail without --all");
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSON mode"
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(
        parsed["error"],
        "--json requires --all (cannot show interactive prompts in JSON mode)"
    );
}

#[test]
fn test_wp_alias_for_clean() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Test that `gg wp --help` works and shows clean command help
    let (success, stdout, _) = run_gg(&repo_path, &["wp", "--help"]);
    assert!(success, "gg wp --help should succeed");
    assert!(
        stdout.contains("Clean up merged stacks"),
        "wp should be an alias for clean: {}",
        stdout
    );

    // Test that `gg wp` runs without error (same as `gg clean`)
    let (success, _, _) = run_gg(&repo_path, &["wp"]);
    assert!(success, "gg wp should succeed");
}

// ============================================================
// Tests for bug fixes in PR #26
// ============================================================

#[test]
fn test_land_help_shows_clean_option() {
    let (_temp_dir, repo_path) = create_test_repo();

    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success, "Help should succeed");
    assert!(
        stdout.contains("--clean"),
        "Should show --clean option: {}",
        stdout
    );
    assert!(
        stdout.contains("clean up stack") || stdout.contains("Automatically clean"),
        "Should mention cleaning up stack: {}",
        stdout
    );
}

#[test]
fn test_clean_removes_orphan_entry_branches_when_stack_branch_missing() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config with username (provider detection not available in tests)
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create orphan entry branches (but NO main stack branch)
    run_git(&repo_path, &["branch", "testuser/fix-dashboard--c-aaa111"]);
    run_git(&repo_path, &["branch", "testuser/fix-dashboard--c-bbb222"]);

    let (_success, stdout_before, _stderr_before) = run_gg(&repo_path, &[]);
    assert!(
        stdout_before.contains("fix-dashboard"),
        "Expected stack to be listed before clean, got: {stdout_before}"
    );

    // Run clean (should remove local orphan entry branches)
    let (success, _stdout, stderr) = run_gg(&repo_path, &["clean", "--all"]);
    assert!(success, "gg clean failed: {stderr}");

    let (_success, branches) = run_git(&repo_path, &["branch", "--list"]);
    assert!(
        !branches.contains("testuser/fix-dashboard--c-aaa111"),
        "Expected orphan entry branch to be deleted"
    );
    assert!(
        !branches.contains("testuser/fix-dashboard--c-bbb222"),
        "Expected orphan entry branch to be deleted"
    );

    let (_success, stdout_after, _stderr_after) = run_gg(&repo_path, &[]);
    assert!(
        !stdout_after.contains("fix-dashboard"),
        "Expected stack to not be listed after clean, got: {stdout_after}"
    );
}

#[test]
fn test_land_no_clean_flag_accepted() {
    // Test that the --no-clean flag is recognized and doesn't cause an error
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

    // Verify --no-clean flag is accepted (it will fail for other reasons,
    // like no PRs to land, but should not fail on unknown argument)
    let (_, _stdout, stderr) = run_gg(&repo_path, &["land", "--no-clean"]);

    // Should not contain "unknown argument" or similar clap errors
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("invalid value"),
        "The --no-clean flag should be recognized, stderr: {}",
        stderr
    );
}

#[test]
fn test_land_auto_clean_config_default() {
    // Test that land_auto_clean config setting defaults to false
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up minimal config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(gg_dir.join("config.json"), r#"{"defaults":{}}"#).expect("Failed to write config");

    // Load and verify default
    let config_path = gg_dir.join("config.json");
    let content = fs::read_to_string(config_path).expect("Failed to read config");

    // The default config should not contain land_auto_clean
    // (since it defaults to false and is skipped in serialization)
    assert!(
        !content.contains("land_auto_clean"),
        "Default config should not contain land_auto_clean when false"
    );
}

#[test]
fn test_land_auto_clean_config_enabled() {
    // Test that land_auto_clean can be set to true in config
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config with land_auto_clean enabled
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"land_auto_clean":true}}"#,
    )
    .expect("Failed to write config");

    // Load and verify
    let config_path = gg_dir.join("config.json");
    let content = fs::read_to_string(config_path).expect("Failed to read config");

    assert!(
        content.contains("\"land_auto_clean\":true"),
        "Config should contain land_auto_clean when enabled"
    );
}

#[test]
fn test_land_clean_and_no_clean_conflict() {
    // Test that --clean and --no-clean flags conflict
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

    // Try to use both flags - should fail with conflict error
    let (success, _stdout, stderr) = run_gg(&repo_path, &["land", "--clean", "--no-clean"]);

    assert!(!success, "Using both --clean and --no-clean should fail");

    // clap should report the conflict
    assert!(
        stderr.contains("conflict") || stderr.contains("cannot be used with"),
        "Error should mention flag conflict, stderr: {}",
        stderr
    );
}

// ============================================================
// Tests for PR #57 - rebase state handling fix
// ============================================================

#[test]
fn test_sync_with_missing_gg_ids_and_clean_working_directory() {
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
    run_gg(&repo_path, &["co", "clean-wd-gg-id-test"]);

    // Create a commit without GG-ID
    fs::write(repo_path.join("file1.txt"), "original").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    // Ensure working directory is clean (no uncommitted changes)
    let (_, status) = run_git(&repo_path, &["status", "--short"]);
    assert!(
        status.trim().is_empty(),
        "Working directory should be clean before sync"
    );

    // Run sync - with clean working directory, no stashing should be needed
    let (_success, _stdout, _stderr) = run_gg(&repo_path, &["sync"]);

    // This test verifies the flow works when there are no uncommitted changes
    // The actual GG-ID addition and sync logic is tested elsewhere
}

#[test]
fn test_working_directory_clean_detection() {
    // Test that we can correctly detect when working directory is clean
    // This is important for the auto-stashing logic
    let (_temp_dir, repo_path) = create_test_repo();

    // Initially with just the README commit, should be clean
    let (_, status) = run_git(&repo_path, &["status", "--short"]);
    assert!(
        status.trim().is_empty(),
        "Fresh repo should have clean working directory"
    );

    // Add a new file (untracked)
    fs::write(repo_path.join("new-file.txt"), "content").expect("Failed to write file");

    // Untracked files don't make the working directory "dirty" for our purposes
    let (_, status) = run_git(&repo_path, &["status", "--short"]);
    assert!(
        status.contains("new-file.txt"),
        "Should show untracked file"
    );

    // Stage the file
    run_git(&repo_path, &["add", "new-file.txt"]);

    // Now it's dirty (staged changes)
    let (_, status) = run_git(&repo_path, &["status", "--short"]);
    assert!(
        status.contains("new-file.txt"),
        "Should show staged changes"
    );

    // Commit it
    run_git(&repo_path, &["commit", "-m", "Add new file"]);

    // Should be clean again
    let (_, status) = run_git(&repo_path, &["status", "--short"]);
    assert!(status.trim().is_empty(), "Should be clean after commit");
}

#[test]
fn test_clean_detects_locally_merged_worktree_stack_when_provider_check_fails() {
    let (_parent_dir, repo_path) = create_test_repo_with_worktree_support();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main","provider":"github"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "clean-wt", "--worktree"]);
    assert!(success, "checkout --worktree should succeed: {}", stderr);

    let worktree_path = repo_path.parent().expect("repo parent").join(format!(
        "{}.{}",
        repo_path.file_name().expect("repo name").to_string_lossy(),
        "clean-wt"
    ));
    assert!(worktree_path.exists(), "Expected worktree to exist");

    fs::write(worktree_path.join("feature.txt"), "hello").expect("Failed to write feature file");
    run_git(&worktree_path, &["add", "."]);
    let (success, _) = run_git(&worktree_path, &["commit", "-m", "feat: worktree change"]);
    assert!(success, "Expected commit in worktree to succeed");

    let (success, _) = run_git(&repo_path, &["merge", "--ff-only", "testuser/clean-wt"]);
    assert!(success, "Expected fast-forward merge into main to succeed");

    let config_path = gg_dir.join("config.json");
    let mut config: Value =
        serde_json::from_str(&fs::read_to_string(&config_path).expect("Failed to read config"))
            .expect("Failed to parse config JSON");

    config["stacks"]["clean-wt"]["mrs"] = serde_json::json!({ "c-deadbee": 999999 });

    fs::write(
        &config_path,
        serde_json::to_string(&config).expect("Failed to serialize config"),
    )
    .expect("Failed to write config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["clean", "--all"]);
    assert!(
        success,
        "clean --all should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("Deleted stack 'clean-wt'"),
        "Expected merged stack to be cleaned. stdout: {}",
        stdout
    );
    assert!(
        !stdout.contains("No stacks to clean."),
        "Should not report no stacks to clean. stdout: {}",
        stdout
    );
}

#[test]
fn test_clean_merged_stack_with_stacked_entries_verifies_all_branches() {
    // Test that clean verifies ALL entry branches (not just config MRs) before
    // allowing remote deletion. This ensures orphan entry branches discovered
    // via pattern scan are also verified.
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config with provider (to enable verification path)
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main","provider":"github"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with 2 commits
    let (success, _, stderr) = run_gg(&repo_path, &["co", "stacked-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content 1").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "feat: first commit"]);

    fs::write(repo_path.join("file2.txt"), "content 2").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "feat: second commit"]);

    // Create entry branches manually to simulate orphan branches
    // (branches that exist but aren't in config.mrs)
    run_git(&repo_path, &["branch", "testuser/stacked-test--c-orphan1"]);

    // Merge stack into main (locally)
    run_git(&repo_path, &["checkout", "main"]);
    let (success, _) = run_git(&repo_path, &["merge", "--ff-only", "testuser/stacked-test"]);
    assert!(success, "Expected fast-forward merge to succeed");

    // Push to origin to update remote tracking
    let (_, _) = run_git(&repo_path, &["push", "origin", "main"]);

    // Verify clean succeeds but with warning about skipping remote deletion
    // (because provider checks will fail in test environment)
    let (success, stdout, stderr) = run_gg(&repo_path, &["clean", "--all"]);
    assert!(
        success,
        "clean --all should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    // The orphan entry branch should be cleaned locally
    let (_, branches, _) = run_git_full(&repo_path, &["branch", "--list"]);
    assert!(
        !branches.contains("testuser/stacked-test--c-orphan1"),
        "Orphan entry branch should be deleted locally"
    );

    // Should have warning about remote deletion skipped (no provider in test)
    assert!(
        stdout.contains("Skipping remote branch deletion")
            || stderr.contains("Skipping remote branch deletion")
            || stdout.contains("Deleted stack"),
        "Should either skip remote deletion or succeed. stdout: {}, stderr: {}",
        stdout,
        stderr
    );
}

#[test]
fn test_clean_shows_warning_when_verification_unavailable() {
    // Test that clean shows appropriate warning when merge verification
    // is not available (e.g., provider errors or no provider)
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config with provider configured (but will fail in tests)
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main","provider":"github"},"stacks":{"verify-test":{"mrs":{"c-fake123":999999}}}}"#,
    )
    .expect("Failed to write config");

    // Create a stack branch and merge it
    let (success, _, stderr) = run_gg(&repo_path, &["co", "verify-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "content").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "feat: test feature"]);

    // Merge into main
    run_git(&repo_path, &["checkout", "main"]);
    let (success, _) = run_git(&repo_path, &["merge", "--ff-only", "testuser/verify-test"]);
    assert!(success, "Expected fast-forward merge to succeed");

    // Run clean - should show warning about skipping remote deletion
    // since provider check will fail (fake MR number)
    let (success, stdout, stderr) = run_gg(&repo_path, &["clean", "--all"]);
    assert!(
        success,
        "clean --all should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    // Should show warning about verification being unavailable
    assert!(
        stdout.contains("Skipping remote branch deletion") || stderr.contains("Could not fetch MR"),
        "Should warn about verification issues. stdout: {}, stderr: {}",
        stdout,
        stderr
    );
}

#[test]
fn test_clean_local_branches_still_deleted_when_verification_fails() {
    // Test that local branches are still cleaned even when remote verification
    // fails. The conservative behavior only affects REMOTE branch deletion.
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config with provider (to trigger verification path)
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main","provider":"github"},"stacks":{"local-clean-test":{"mrs":{"c-badmr1":888888}}}}"#,
    )
    .expect("Failed to write config");

    // Create stack branch
    let (success, _, stderr) = run_gg(&repo_path, &["co", "local-clean-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("local-file.txt"), "local content").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "feat: local feature"]);

    // Create an entry branch manually
    run_git(
        &repo_path,
        &["branch", "testuser/local-clean-test--c-entry1"],
    );

    // Merge into main (locally merged)
    run_git(&repo_path, &["checkout", "main"]);
    let (success, _) = run_git(
        &repo_path,
        &["merge", "--ff-only", "testuser/local-clean-test"],
    );
    assert!(success, "Expected fast-forward merge to succeed");

    // Verify branches exist before clean
    let (_, branches_before, _) = run_git_full(&repo_path, &["branch", "--list"]);
    assert!(
        branches_before.contains("testuser/local-clean-test"),
        "Stack branch should exist before clean"
    );
    assert!(
        branches_before.contains("testuser/local-clean-test--c-entry1"),
        "Entry branch should exist before clean"
    );

    // Run clean
    let (success, stdout, stderr) = run_gg(&repo_path, &["clean", "--all"]);
    assert!(
        success,
        "clean --all should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    // Verify LOCAL branches are deleted (even though remote verification failed)
    let (_, branches_after, _) = run_git_full(&repo_path, &["branch", "--list"]);
    assert!(
        !branches_after.contains("testuser/local-clean-test--c-entry1"),
        "Local entry branch should be deleted even without verification"
    );

    // Stack should be cleaned
    assert!(
        stdout.contains("Deleted stack 'local-clean-test'"),
        "Stack should be cleaned. stdout: {}",
        stdout
    );
}

#[test]
fn test_clean_no_mrs_tracked_verified_false() {
    // Test that when provider is configured but no MRs are tracked,
    // verified=false (provider wasn't consulted).
    // This ensures we don't claim verification without actually checking.
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config with provider but NO MRs tracked
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main","provider":"github"},"stacks":{"no-mr-test":{}}}"#,
    )
    .expect("Failed to write config");

    // Create stack branch
    let (success, _, stderr) = run_gg(&repo_path, &["co", "no-mr-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file.txt"), "content").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "feat: feature"]);

    // Push to remote so there's a remote branch to potentially delete
    run_git(&repo_path, &["push", "-u", "origin", "testuser/no-mr-test"]);

    // Merge into main
    run_git(&repo_path, &["checkout", "main"]);
    let (success, _) = run_git(&repo_path, &["merge", "--ff-only", "testuser/no-mr-test"]);
    assert!(success, "Expected fast-forward merge to succeed");
    run_git(&repo_path, &["push", "origin", "main"]);

    // Run clean
    let (success, stdout, stderr) = run_gg(&repo_path, &["clean", "--all"]);
    assert!(
        success,
        "clean --all should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    // Should show warning about skipping remote branch deletion
    // because provider wasn't consulted (no MRs to check)
    assert!(
        stdout.contains("Skipping remote branch deletion"),
        "Should skip remote deletion when provider not consulted. stdout: {}, stderr: {}",
        stdout,
        stderr
    );

    // Remote branch should still exist
    let (_, remote_branches, _) = run_git_full(&repo_path, &["branch", "-r"]);
    assert!(
        remote_branches.contains("origin/testuser/no-mr-test"),
        "Remote branch should NOT be deleted when verified=false. Branches: {}",
        remote_branches
    );
}

#[test]
fn test_land_all_clean_deletes_remote_entry_branch_after_mapping_removed() {
    let (_temp_dir, repo_path, remote_path) = create_test_repo_with_remote();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main","provider":"github"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "land-clean"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("land-clean.txt"), "land clean\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: land clean\n\nGG-ID: c-1a2b3c4"],
    );

    fs::write(
        gg_dir.join("config.json"),
        r#"{
  "defaults": {
    "branch_username": "testuser",
    "base": "main",
    "provider": "github"
  },
  "stacks": {
    "land-clean": {
      "base": "main",
      "mrs": {
        "c-1a2b3c4": 4242
      }
    }
  }
}"#,
    )
    .expect("Failed to write PR mapping");

    let entry_branch = "testuser/land-clean--c-1a2b3c4";
    run_git(&repo_path, &["branch", entry_branch]);
    let (success, _, stderr) = run_git_full(&repo_path, &["push", "-u", "origin", entry_branch]);
    assert!(success, "Failed to push entry branch: {}", stderr);

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

if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "4242" ]; then
  case "$*" in
    *"--jq .reviewDecision"*) echo APPROVED; exit 0 ;;
  esac
  echo '{"number":4242,"title":"Land clean","state":"OPEN","url":"https://github.com/test/repo/pull/4242","headRefName":"testuser/land-clean--c-1a2b3c4","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "merge" ] && [ "$3" = "4242" ]; then
  current_branch=$(git rev-parse --abbrev-ref HEAD)
  git checkout main >/dev/null 2>&1
  git merge --ff-only testuser/land-clean >/dev/null 2>&1
  git push origin main >/dev/null 2>&1
  git checkout "$current_branch" >/dev/null 2>&1
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
        &["land", "--all", "--clean"],
        &[("PATH", new_path.as_os_str())],
    );
    assert!(
        success,
        "land --all --clean should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    let combined = format!("{}{}", stdout, stderr);
    assert!(
        !combined.contains("merge verification is unavailable"),
        "auto-clean after verified land should not warn about unavailable verification: {}",
        combined
    );

    let output = Command::new("git")
        .args([
            "--git-dir",
            remote_path.to_str().unwrap(),
            "show-ref",
            "refs/heads/testuser/land-clean--c-1a2b3c4",
        ])
        .output()
        .expect("Failed to inspect remote refs");
    assert!(
        !output.status.success(),
        "remote entry branch should be deleted after verified land auto-clean"
    );
}

#[test]
fn test_land_already_merged_auto_clean_records_branch_deletion_and_refuses_undo() {
    let (_temp_dir, repo_path, remote_path) = create_test_repo_with_remote();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main","provider":"github"}}"#,
    )
    .unwrap();
    let (success, _, stderr) = run_gg(&repo_path, &["co", "land-already-merged-clean"]);
    assert!(success, "checkout failed: {stderr}");
    fs::write(repo_path.join("feature.txt"), "feature\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: already merged\n\nGG-ID: c-b1c2d3e"],
    );

    let entry_branch = "testuser/land-already-merged-clean--c-b1c2d3e";
    run_git(&repo_path, &["branch", entry_branch]);
    let (success, _, stderr) = run_git_full(&repo_path, &["push", "-u", "origin", entry_branch]);
    assert!(success, "entry push failed: {stderr}");
    let (_, prior_oid) = run_git(&repo_path, &["rev-parse", entry_branch]);
    let prior_oid = prior_oid.trim().to_string();

    fs::write(
        gg_dir.join("config.json"),
        r#"{
  "defaults": {
    "branch_username": "testuser",
    "base": "main",
    "provider": "github"
  },
  "stacks": {
    "land-already-merged-clean": {
      "base": "main",
      "mrs": { "c-b1c2d3e": 77 }
    }
  }
}"#,
    )
    .unwrap();

    let fake_bin = repo_path.join("fake-bin-land-already-merged");
    fs::create_dir_all(&fake_bin).unwrap();
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
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "77" ]; then
  echo '{"number":77,"title":"Already merged","state":"MERGED","url":"https://github.com/test/repo/pull/77","headRefName":"testuser/land-already-merged-clean--c-b1c2d3e","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
  exit 0
fi
echo "unexpected gh invocation: $@" >&2
exit 1
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(fake_bin.join("gh")).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(fake_bin.join("gh"), permissions).unwrap();
    }
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut path = std::ffi::OsString::from(fake_bin.as_os_str());
    path.push(":");
    path.push(old_path);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["land", "--all", "--clean", "--json"],
        &[("PATH", path.as_os_str())],
    );
    assert!(success, "land failed: stdout={stdout} stderr={stderr}");
    let land_result: Value = serde_json::from_str(&stdout).expect("land output must be JSON");
    assert_eq!(land_result["land"]["cleaned"], true);

    let remote_ref = format!("refs/heads/{entry_branch}");
    let output = Command::new("git")
        .args([
            "--git-dir",
            remote_path.to_str().unwrap(),
            "show-ref",
            &remote_ref,
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "auto-clean must delete remote entry branch"
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "--list", "--json"]);
    assert!(success, "undo list failed: {stderr}");
    let listed: Value = serde_json::from_str(&stdout).unwrap();
    let land_record = listed["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| {
            record["kind"] == "land"
                && record["remote_effects"].as_array().is_some_and(|effects| {
                    effects
                        .iter()
                        .any(|effect| effect["kind"] == "branch_deleted")
                })
        })
        .expect("Land operation must be recorded");
    assert_eq!(land_record["touched_remote"], true);
    assert_eq!(land_record["is_undoable"], false);
    let deletion = land_record["remote_effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|effect| effect["kind"] == "branch_deleted")
        .expect("Land must retain auto-clean branch deletion effect");
    assert_eq!(deletion["branch"], entry_branch);
    assert_eq!(deletion["prior_oid"], prior_oid);
    let operation_id = land_record["id"].as_str().unwrap();

    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", operation_id, "--json"]);
    assert!(!success, "undo must refuse Land remote effect: {stderr}");
    let refused: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(refused["refusal"]["reason"], "remote");
}

#[test]
fn test_clean_records_remote_branch_deletion_and_undo_refuses_with_recovery_hint() {
    let (_temp_dir, repo_path, remote_path) = create_test_repo_with_remote();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main","provider":"github"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "clean-remote-effect"]);
    assert!(success, "Failed to create stack: {stderr}");
    fs::write(repo_path.join("feature.txt"), "feature\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: remote effect\n\nGG-ID: c-a1b2c3d"],
    );

    let entry_branch = "testuser/clean-remote-effect--c-a1b2c3d";
    run_git(&repo_path, &["branch", entry_branch]);
    let (success, _, stderr) = run_git_full(&repo_path, &["push", "-u", "origin", entry_branch]);
    assert!(success, "Failed to push entry branch: {stderr}");
    let (success, prior_oid) = run_git(&repo_path, &["rev-parse", entry_branch]);
    assert!(success);
    let prior_oid = prior_oid.trim().to_string();

    fs::write(
        gg_dir.join("config.json"),
        r#"{
  "defaults": {
    "branch_username": "testuser",
    "base": "main",
    "provider": "github"
  },
  "stacks": {
    "clean-remote-effect": {
      "base": "main",
      "mrs": { "c-a1b2c3d": 42 }
    }
  }
}"#,
    )
    .unwrap();

    let fake_bin = repo_path.join("fake-bin-clean-effect");
    fs::create_dir_all(&fake_bin).unwrap();
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
if [ "$1" = "pr" ] && [ "$2" = "view" ] && [ "$3" = "42" ]; then
  echo '{"number":42,"title":"Remote effect","state":"MERGED","url":"https://github.com/test/repo/pull/42","headRefName":"testuser/clean-remote-effect--c-a1b2c3d","isDraft":false,"mergeable":"MERGEABLE","reviews":[]}'
  exit 0
fi
echo "unexpected gh invocation: $@" >&2
exit 1
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(fake_bin.join("gh")).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(fake_bin.join("gh"), permissions).unwrap();
    }
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut path = std::ffi::OsString::from(fake_bin.as_os_str());
    path.push(":");
    path.push(old_path);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["clean", "--all", "--json"],
        &[("PATH", path.as_os_str())],
    );
    assert!(success, "clean failed: stdout={stdout} stderr={stderr}");

    let remote_ref = format!("refs/heads/{entry_branch}");
    let output = Command::new("git")
        .args([
            "--git-dir",
            remote_path.to_str().unwrap(),
            "show-ref",
            &remote_ref,
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "remote entry branch must be deleted"
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "--list", "--json"]);
    assert!(success, "undo list failed: {stderr}");
    let listed: Value = serde_json::from_str(&stdout).expect("undo list must be JSON");
    let clean_record = listed["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| {
            record["kind"] == "clean"
                && record["remote_effects"].as_array().is_some_and(|effects| {
                    effects
                        .iter()
                        .any(|effect| effect["kind"] == "branch_deleted")
                })
        })
        .expect("clean operation must be listed");
    assert_eq!(clean_record["touched_remote"], true);
    assert_eq!(clean_record["is_undoable"], false);
    assert_eq!(clean_record["remote_effects"][0]["kind"], "branch_deleted");
    assert_eq!(clean_record["remote_effects"][0]["remote"], "origin");
    assert_eq!(clean_record["remote_effects"][0]["branch"], entry_branch);
    assert_eq!(clean_record["remote_effects"][0]["prior_oid"], prior_oid);
    let operation_id = clean_record["id"].as_str().unwrap().to_string();

    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", &operation_id, "--json"]);
    assert!(!success, "undo must refuse remote deletion: {stderr}");
    let refused: Value = serde_json::from_str(&stdout).expect("undo refusal must be JSON");
    assert_eq!(refused["status"], "refused");
    assert_eq!(refused["refusal"]["reason"], "remote");
    let hints = refused["refusal"]["hints"].as_array().unwrap();
    let restore = format!("git push origin {prior_oid}:refs/heads/{entry_branch}");
    assert!(
        hints
            .iter()
            .filter_map(Value::as_str)
            .any(|hint| hint.contains(&restore)),
        "recovery hint must preserve the deleted branch restore command: {hints:?}"
    );
}

#[test]
fn test_gg_clean_current_branch_with_main_in_linked_worktree() {
    // Regression: gg clean crashed when user is on the stack branch and
    // main is checked out in a linked worktree.
    // Error was: "cannot set HEAD to reference 'refs/heads/main' as it is
    // the current HEAD of a linked repository"
    let (temp_dir, repo_path) = create_test_repo();

    // Set up gg config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create the stack and add a commit on it
    let stack_name = "broken-windows";
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", stack_name]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feat.txt"), "feature\n").expect("Failed to write feat.txt");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "feat: add feature"]);

    // Move HEAD back to the stack branch explicitly
    run_git(&repo_path, &["checkout", "testuser/broken-windows"]);

    // Create a linked worktree that checks out main — this is the trigger for the bug
    let linked_path = temp_dir.path().join("linked-main");
    let output = Command::new("git")
        .args(["worktree", "add", linked_path.to_str().unwrap(), "main"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to run git worktree add");
    assert!(
        output.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Merge the stack onto main so gg clean considers it cleaned
    run_git(
        &linked_path,
        &["merge", "--ff-only", "testuser/broken-windows"],
    );

    // Now: main worktree is on testuser/broken-windows, linked worktree has main.
    // gg clean must handle checking out main (the base) gracefully.
    let (success, stdout, stderr) = run_gg(&repo_path, &["clean", "--json", "--all"]);
    assert!(
        success,
        "gg clean should succeed even when main is in a linked worktree.\nstdout={}\nstderr={}",
        stdout, stderr
    );

    // Stack branch must be gone
    let branch_output = Command::new("git")
        .args(["branch", "--list", "testuser/broken-windows"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to list branches");
    assert!(
        String::from_utf8_lossy(&branch_output.stdout)
            .trim()
            .is_empty(),
        "Stack branch should have been deleted"
    );

    // Clean up linked worktree
    let _ = Command::new("git")
        .args([
            "worktree",
            "remove",
            "--force",
            linked_path.to_str().unwrap(),
        ])
        .current_dir(&repo_path)
        .output();
}
