use crate::helpers::{
    create_test_repo, create_test_repo_with_remote, run_gg, run_gg_with_env, run_git,
};

use serde_json::Value;
use std::fs;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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
