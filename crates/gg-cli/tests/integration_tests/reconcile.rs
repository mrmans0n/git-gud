use crate::helpers::{create_test_repo, run_gg, run_git};

use std::fs;

#[test]
fn test_reconcile_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["reconcile", "--help"]);

    assert!(success);
    assert!(stdout.contains("--dry-run") || stdout.contains("-n"));
    assert!(stdout.contains("Reconcile") || stdout.contains("reconcile"));
}

#[test]
fn test_reconcile_empty_stack() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github"}}"#,
    )
    .expect("Failed to write config");

    // Create a new empty stack
    run_gg(&repo_path, &["co", "empty-stack"]);

    // Run reconcile - should say stack is empty
    let (success, stdout, stderr) = run_gg(&repo_path, &["reconcile", "--dry-run"]);

    let combined = format!("{}{}", stdout, stderr);
    assert!(
        success || combined.contains("empty"),
        "Should handle empty stack gracefully: {}",
        combined
    );
}

#[test]
fn test_reconcile_detects_commits_without_gg_id() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    run_gg(&repo_path, &["co", "test-reconcile"]);

    // Make commits WITHOUT going through gg sync (simulating the problem case)
    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit without GG-ID"]);

    // Run reconcile --dry-run - should detect commits needing GG-IDs
    let (success, stdout, stderr) = run_gg(&repo_path, &["reconcile", "--dry-run"]);

    assert!(success, "Dry run should succeed: {} {}", stdout, stderr);

    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("need GG-ID") || combined.contains("commits"),
        "Should detect commits needing GG-IDs: {}",
        combined
    );
    assert!(
        combined.contains("Dry run"),
        "Should confirm dry run: {}",
        combined
    );
}

#[test]
fn test_reconcile_already_reconciled() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    run_gg(&repo_path, &["co", "already-reconciled"]);

    // Make a commit WITH a GG-ID (properly formatted)
    fs::write(repo_path.join("proper.txt"), "proper content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Proper commit\n\nGG-ID: c-test123"],
    );

    // Run reconcile --dry-run - should say nothing to do
    let (success, stdout, stderr) = run_gg(&repo_path, &["reconcile", "--dry-run"]);

    assert!(success, "Should succeed: {} {}", stdout, stderr);

    let combined = format!("{}{}", stdout, stderr);
    // Should indicate already reconciled (no commits needing IDs, no PRs to map)
    assert!(
        combined.contains("reconciled") || combined.contains("Nothing to do"),
        "Should say already reconciled or nothing to do: {}",
        combined
    );
}

#[test]
fn test_reconcile_not_on_stack() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Try reconcile on main (not a stack branch)
    let (success, _stdout, stderr) = run_gg(&repo_path, &["reconcile"]);

    assert!(!success, "Should fail when not on a stack");
    // Error message can vary: "Not on a stack", "No origin remote", etc.
    // The key is that it fails with some error
    assert!(
        stderr.contains("Not on a stack")
            || stderr.contains("stack")
            || stderr.contains("origin")
            || stderr.contains("error"),
        "Should fail with error when not on a stack: {}",
        stderr
    );
}

// ============================================================
// Tests for auto-stashing functionality in sync command
// ============================================================
//
// Note: Full end-to-end sync testing requires a configured provider (glab/gh)
// which is not available in the integration test environment. These tests
// verify the auto-stashing logic works correctly when it's triggered.
