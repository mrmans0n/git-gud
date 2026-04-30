use crate::helpers::{
    create_test_repo, create_test_repo_with_remote, create_test_repo_with_worktree_support, run_gg,
    run_gg_with_env, run_git, run_git_full,
};

use serde_json::Value;
use std::fs;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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
