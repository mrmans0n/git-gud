use crate::helpers::{create_test_repo, create_test_repo_with_remote, run_gg, run_git};

use std::fs;

#[test]
fn test_gg_continue_fails_with_unstaged_changes() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    run_gg(&repo_path, &["co", "continue-unstaged-test"]);

    fs::write(repo_path.join("test.txt"), "test").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    // Simulate rebase in progress
    let rebase_dir = repo_path.join(".git/rebase-merge");
    fs::create_dir_all(&rebase_dir).expect("Failed to create rebase-merge dir");
    fs::write(
        rebase_dir.join("head-name"),
        "refs/heads/testuser/continue-unstaged-test",
    )
    .expect("Failed to write head-name");

    // Create unstaged changes
    fs::write(repo_path.join("test.txt"), "modified but not staged").expect("Failed to write file");

    // Try gg continue - should fail
    let (success, stdout, stderr) = run_gg(&repo_path, &["continue"]);

    assert!(!success, "gg continue should fail with unstaged changes");
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("unstaged changes") || combined.contains("unstaged"),
        "Error should mention unstaged changes: {}",
        combined
    );
    assert!(
        combined.contains("git add") || combined.contains("stage"),
        "Error should mention git add: {}",
        combined
    );

    // Clean up
    fs::remove_dir_all(&rebase_dir).ok();
}

#[test]
fn test_gg_continue_fails_with_unresolved_conflicts() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    run_gg(&repo_path, &["co", "continue-conflict-test"]);

    fs::write(repo_path.join("conflict.txt"), "original").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Initial"]);

    // Simulate rebase in progress with conflicts
    let rebase_dir = repo_path.join(".git/rebase-merge");
    fs::create_dir_all(&rebase_dir).expect("Failed to create rebase-merge dir");
    fs::write(
        rebase_dir.join("head-name"),
        "refs/heads/testuser/continue-conflict-test",
    )
    .expect("Failed to write head-name");

    // Create a conflicted file (git marks conflicts in the index)
    // We'll simulate this by creating the conflict.txt with conflict markers
    fs::write(
        repo_path.join("conflict.txt"),
        "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> commit\n",
    )
    .expect("Failed to write conflict file");

    // Mark file as conflicted by NOT staging it (unstaged modification)
    // In a real conflict, git status would show "both modified" or similar
    // For this test, we simulate by having the file modified but not staged

    // Try gg continue - should fail
    let (success, stdout, stderr) = run_gg(&repo_path, &["continue"]);

    assert!(
        !success,
        "gg continue should fail with conflicts or unstaged changes"
    );
    let combined = format!("{}{}", stdout, stderr);
    // The error could mention either conflicts or unstaged changes
    assert!(
        combined.contains("conflict")
            || combined.contains("unstaged")
            || combined.contains("Resolve"),
        "Error should mention conflicts or need to resolve: {}",
        combined
    );

    // Clean up
    fs::remove_dir_all(&rebase_dir).ok();
}

#[test]
fn test_conflict_detection_from_stderr() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with a commit
    run_gg(&repo_path, &["co", "stderr-conflict-test"]);

    fs::write(repo_path.join("data.txt"), "version 1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add data"]);

    // Make a conflicting change on main
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("data.txt"), "version main").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Update on main"]);
    run_git(&repo_path, &["push", "origin", "main"]);

    // Go back to stack and try to rebase (will conflict)
    run_git(&repo_path, &["checkout", "testuser/stderr-conflict-test"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["rebase"]);

    // Should detect conflict
    assert!(!success, "Rebase should fail with conflict");
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("conflict") || combined.contains("CONFLICT"),
        "Should detect conflict from stderr: {}",
        combined
    );
    assert!(
        combined.contains("gg continue") || combined.contains("gg abort"),
        "Should suggest gg continue/abort: {}",
        combined
    );

    // Abort the rebase to clean up
    let _ = run_gg(&repo_path, &["abort"]);
}

#[test]
fn test_conflict_detection_from_stdout() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    run_gg(&repo_path, &["co", "stdout-conflict-test"]);

    fs::write(repo_path.join("shared.txt"), "original").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Initial shared"]);

    // Create conflicting commit on main
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("shared.txt"), "main version").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Main update"]);
    run_git(&repo_path, &["push", "origin", "main"]);

    // Return to stack
    run_git(&repo_path, &["checkout", "testuser/stdout-conflict-test"]);

    // Rebase will conflict
    let (success, stdout, stderr) = run_gg(&repo_path, &["rebase"]);

    assert!(!success, "Rebase should fail with conflict");

    // Conflict might be reported in stdout or stderr
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("CONFLICT") || combined.contains("conflict"),
        "Should detect conflict from output: {}",
        combined
    );

    // Abort to clean up
    let _ = run_gg(&repo_path, &["abort"]);
}

#[test]
fn test_gg_continue_provides_actionable_error_messages() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Try gg continue when no rebase is in progress
    let (success, stdout, stderr) = run_gg(&repo_path, &["continue"]);

    assert!(!success, "Should fail when no rebase in progress");
    let combined = format!("{}{}", stdout, stderr);
    // Should provide clear message (the specific error type depends on implementation)
    assert!(
        combined.contains("rebase") || combined.contains("No rebase"),
        "Should mention no rebase in progress: {}",
        combined
    );
}

// ==================== gg reconcile tests ====================
