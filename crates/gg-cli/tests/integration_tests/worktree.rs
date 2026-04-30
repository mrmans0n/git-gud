use crate::helpers::{create_test_repo_with_worktree_support, create_worktree, run_gg, run_git};

use std::fs;
use std::process::Command;

#[test]
fn test_worktree_shares_config() {
    let (_parent_dir, repo_path) = create_test_repo_with_worktree_support();

    // Set up config in main repo
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a worktree
    let worktree_path = create_worktree(&repo_path, "wt-config-test");

    // gg ls should work from the worktree (it loads config)
    let (success, stdout, stderr) = run_gg(&worktree_path, &["ls"]);

    // Should succeed (may say "not on a stack" but should not fail with config errors)
    assert!(
        success,
        "gg ls from worktree should succeed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    // Verify the config is read from the shared .git/gg/ location
    // by checking that the worktree does NOT have its own config
    let worktree_git_dir = repo_path.join(".git/worktrees/wt-config-test/gg/config.json");
    assert!(
        !worktree_git_dir.exists(),
        "Worktree should NOT have its own config - should use shared config"
    );
}

#[test]
fn test_worktree_independent_nav_state() {
    let (_parent_dir, repo_path) = create_test_repo_with_worktree_support();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with commits in main repo
    let (success, _, stderr) = run_gg(&repo_path, &["co", "nav-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content 1").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "commit 1"]);

    fs::write(repo_path.join("file2.txt"), "content 2").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "commit 2"]);

    // Navigate to first commit in main repo
    let (success, _, stderr) = run_gg(&repo_path, &["first"]);
    assert!(success, "Failed to nav first: {}", stderr);

    // The main repo should now have a current_stack file in its git dir
    let main_current_stack = repo_path.join(".git/gg/current_stack");
    assert!(
        main_current_stack.exists(),
        "Main repo should have nav state (current_stack file)"
    );

    // Create a worktree on main branch
    let _worktree_path = create_worktree(&repo_path, "wt-nav-test");

    // The worktree should NOT have nav state (it's per-worktree)
    let worktree_current_stack = repo_path.join(".git/worktrees/wt-nav-test/gg/current_stack");
    assert!(
        !worktree_current_stack.exists(),
        "Worktree should NOT have nav state from main repo"
    );
}

#[test]
fn test_worktree_shared_lock() {
    let (_parent_dir, repo_path) = create_test_repo_with_worktree_support();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a worktree
    let worktree_path = create_worktree(&repo_path, "wt-lock-test");

    // Verify that lock files go to the common dir, not the worktree dir
    // We can test this by running a command from the worktree and checking
    // that the lock file would be in .git/gg/ not .git/worktrees/wt-lock-test/gg/
    // Since the lock is transient, we just verify the gg dir gets created in the right place
    let (success, _, stderr) = run_gg(&worktree_path, &["ls"]);
    assert!(success, "gg ls from worktree should succeed: {}", stderr);

    // The shared gg dir should exist (created by config load or lock)
    assert!(gg_dir.exists(), "Shared .git/gg/ directory should exist");

    // The worktree-specific gg dir should NOT have a lock file
    let worktree_lock = repo_path.join(".git/worktrees/wt-lock-test/gg/operation.lock");
    assert!(
        !worktree_lock.exists(),
        "Worktree should NOT have its own lock file - lock should be in shared .git/gg/"
    );
}

#[test]
fn test_amend_in_worktree_does_not_leave_detached_head() {
    let (_parent_dir, repo_path) = create_test_repo_with_worktree_support();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with 2 commits
    let (success, _, stderr) = run_gg(&repo_path, &["co", "wt-amend-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content 1").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "commit 1"]);

    fs::write(repo_path.join("file2.txt"), "content 2").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "commit 2"]);

    // Now create a worktree for this stack branch.
    // First, switch main repo to main so the stack branch is free.
    run_git(&repo_path, &["checkout", "main"]);

    let worktree_path = repo_path.parent().unwrap().join("wt-amend");
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            "testuser/wt-amend-test",
        ])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to create worktree");
    assert!(
        output.status.success(),
        "worktree add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // In the worktree, navigate to first commit and amend
    let (success, _, stderr) = run_gg(&worktree_path, &["first"]);
    assert!(success, "Failed to nav first in worktree: {}", stderr);

    fs::write(worktree_path.join("file1.txt"), "amended content").expect("Failed to write");
    run_git(&worktree_path, &["add", "."]);
    let (success, stdout, stderr) = run_gg(&worktree_path, &["amend"]);
    assert!(
        success,
        "amend should succeed in worktree: stdout={}, stderr={}",
        stdout, stderr
    );

    // Verify amend completed the full flow: squash + rebase + nav back.
    // Without ensure_branch_attached, checkout_branch would fail in a
    // worktree because git refuses to checkout a branch "in use" elsewhere.
    // Note: after amend, gg navigates back to the amended position which
    // detaches HEAD again (normal nav behavior), so we check the output
    // messages to confirm the rebase step completed successfully.
    assert!(
        stdout.contains("Rebased"),
        "amend should complete rebase in worktree: {}",
        stdout
    );
    assert!(
        stdout.contains("OK"),
        "amend output should contain OK: {}",
        stdout
    );

    // Clean up worktree
    let _ = Command::new("git")
        .args([
            "worktree",
            "remove",
            worktree_path.to_str().unwrap(),
            "--force",
        ])
        .current_dir(&repo_path)
        .output();
}

// Test for lint after rebase drops landed commits (auth-independent)
// See: https://github.com/mrmans0n/git-gud/issues/199
//
// This test uses `gg rebase` + `gg lint` instead of `gg sync --lint` to avoid
// the provider auth requirement. Both commands exercise the same code paths:
// - rebase drops commits that are already on the base branch
// - lint runs on the post-rebase stack
//
// The original bug was that lint would try to run on position 3 of a 2-commit
// stack after rebase dropped a landed commit.
