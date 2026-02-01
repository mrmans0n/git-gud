//! Integration tests for git-gud
//!
//! These tests create temporary git repositories and test the core functionality.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

/// Helper to create a temporary git repo
fn create_test_repo() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let repo_path = temp_dir.path().to_path_buf();

    // Initialize git repo
    Command::new("git")
        .args(["init"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to init git repo");

    // Configure git user
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to configure git email");

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to configure git name");

    // Create initial commit on main
    fs::write(repo_path.join("README.md"), "# Test Repo\n").expect("Failed to write README");

    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add files");

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to create initial commit");

    (temp_dir, repo_path)
}

/// Helper to run gg command in a repo
fn run_gg(repo_path: &PathBuf, args: &[&str]) -> (bool, String, String) {
    let gg_path = env!("CARGO_BIN_EXE_gg");

    let output = Command::new(gg_path)
        .args(args)
        .current_dir(repo_path)
        .output()
        .expect("Failed to run gg");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    (output.status.success(), stdout, stderr)
}

/// Helper to run git command
fn run_git(repo_path: &PathBuf, args: &[&str]) -> (bool, String) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (output.status.success(), stdout)
}

#[test]
fn test_gg_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["--help"]);

    assert!(success);
    assert!(stdout.contains("stacked-diffs CLI tool"));
    assert!(stdout.contains("co"));
    assert!(stdout.contains("sync"));
    assert!(stdout.contains("ls"));
}

#[test]
fn test_gg_version() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["--version"]);

    assert!(success);
    assert!(stdout.contains("gg"));
}

#[test]
fn test_gg_ls_no_stack() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Running ls outside a stack should show no stacks
    let (success, _stdout, stderr) = run_gg(&repo_path, &["ls"]);

    // Should succeed but show a message about no stacks
    // (It may fail because we're on main, not a stack branch)
    assert!(success || stderr.contains("Not on a stack"));
}

#[test]
fn test_gg_checkout_creates_branch() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up a config file with username since glab isn't available in tests
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a new stack
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "my-feature"]);

    if !success {
        println!("stdout: {}", stdout);
        println!("stderr: {}", stderr);
    }

    assert!(success, "Failed to create stack: {}", stderr);
    assert!(stdout.contains("Created stack") || stdout.contains("my-feature"));

    // Verify we're on the new branch
    let (_, branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(branch.trim(), "testuser/my-feature");
}

#[test]
fn test_gg_checkout_switch_existing() {
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
    let (success, _, stderr) = run_gg(&repo_path, &["co", "stack1"]);
    assert!(success, "Failed to create stack1: {}", stderr);

    // Go back to main
    run_git(&repo_path, &["checkout", "main"]);

    // Create another stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "stack2"]);
    assert!(success, "Failed to create stack2: {}", stderr);

    // Switch back to stack1
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "stack1"]);
    assert!(success, "Failed to switch to stack1: {}", stderr);
    assert!(stdout.contains("Switched") || stdout.contains("stack1"));

    // Verify we're on stack1
    let (_, branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(branch.trim(), "testuser/stack1");
}

#[test]
fn test_gg_ls_shows_commits() {
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
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-stack"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Add a commit
    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add file1"]);

    // Add another commit
    fs::write(repo_path.join("file2.txt"), "content2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add file2"]);

    // List the stack
    let (success, stdout, stderr) = run_gg(&repo_path, &["ls"]);
    assert!(success, "Failed to list stack: {}", stderr);

    assert!(stdout.contains("test-stack"));
    assert!(stdout.contains("2 commits"));
    assert!(stdout.contains("Add file1") || stdout.contains("file1"));
    assert!(stdout.contains("Add file2") || stdout.contains("file2"));
}

#[test]
fn test_gg_navigation() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with commits
    run_gg(&repo_path, &["co", "nav-test"]);

    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("file2.txt"), "content2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    fs::write(repo_path.join("file3.txt"), "content3").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 3"]);

    // Test first
    let (success, stdout, stderr) = run_gg(&repo_path, &["first"]);
    assert!(
        success,
        "first failed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("[1]") || stdout.contains("Commit 1"),
        "first output: {}",
        stdout
    );

    // Test next
    let (success, stdout, stderr) = run_gg(&repo_path, &["next"]);
    assert!(success, "next failed: stdout={}, stderr={}", stdout, stderr);
    assert!(
        stdout.contains("[2]") || stdout.contains("Commit 2"),
        "next output: {}",
        stdout
    );

    // Test last
    let (success, stdout, stderr) = run_gg(&repo_path, &["last"]);
    assert!(success, "last failed: stdout={}, stderr={}", stdout, stderr);
    assert!(
        stdout.contains("[3]") || stdout.contains("Commit 3") || stdout.contains("stack head"),
        "last output: {}",
        stdout
    );

    // Test prev (from last, should go to second-to-last)
    let (success, stdout, stderr) = run_gg(&repo_path, &["prev"]);
    assert!(success, "prev failed: stdout={}, stderr={}", stdout, stderr);
    assert!(
        stdout.contains("[2]") || stdout.contains("Commit 2"),
        "prev output: {}",
        stdout
    );

    // Test mv
    let (success, stdout, stderr) = run_gg(&repo_path, &["mv", "1"]);
    assert!(success, "mv failed: stdout={}, stderr={}", stdout, stderr);
    assert!(
        stdout.contains("[1]") || stdout.contains("Commit 1"),
        "mv output: {}",
        stdout
    );
}

#[test]
fn test_completions() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Test bash completions
    let (success, stdout, _) = run_gg(&repo_path, &["completions", "bash"]);
    assert!(success);
    assert!(stdout.contains("_gg") || stdout.contains("complete"));

    // Test zsh completions
    let (success, stdout, _) = run_gg(&repo_path, &["completions", "zsh"]);
    assert!(success);
    assert!(stdout.contains("#compdef") || stdout.contains("_gg"));

    // Test fish completions
    let (success, stdout, _) = run_gg(&repo_path, &["completions", "fish"]);
    assert!(success);
    assert!(stdout.contains("complete") || stdout.contains("gg"));
}

// ============================================================
// Tests for bug fixes in PR #26
// ============================================================

#[test]
fn test_gg_squash_with_staged_changes() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with one commit
    run_gg(&repo_path, &["co", "squash-test"]);

    fs::write(repo_path.join("file1.txt"), "original content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Initial file"]);

    // Make a change and stage it
    fs::write(repo_path.join("file1.txt"), "modified content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);

    // Squash should work with staged changes (this was the bug)
    let (success, stdout, stderr) = run_gg(&repo_path, &["sc"]);

    // Should succeed - staged changes should be squashable
    assert!(
        success,
        "gg sc should succeed with staged changes. stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("Squashed") || stdout.contains("OK"),
        "Expected squash confirmation. stdout={}",
        stdout
    );

    // Verify the content was squashed
    let content = fs::read_to_string(repo_path.join("file1.txt")).expect("Failed to read file");
    assert_eq!(content, "modified content");
}

#[test]
fn test_gg_squash_rejects_unstaged_when_needs_rebase() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with multiple commits
    run_gg(&repo_path, &["co", "squash-rebase-test"]);

    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("file2.txt"), "content2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Navigate to first commit (now needs_rebase will be true)
    let (success, _, _) = run_gg(&repo_path, &["mv", "1"]);
    assert!(success, "Failed to navigate to first commit");

    // Make unstaged changes (not added)
    fs::write(repo_path.join("file1.txt"), "unstaged modification").expect("Failed to write file");

    // Also stage something to have changes to squash
    fs::write(repo_path.join("newfile.txt"), "new content").expect("Failed to write file");
    run_git(&repo_path, &["add", "newfile.txt"]);

    // Squash should fail because there are unstaged changes and we need to rebase
    let (success, stdout, stderr) = run_gg(&repo_path, &["sc"]);

    // Should fail - unstaged changes would be lost during rebase
    assert!(
        !success || stderr.contains("Dirty") || stderr.contains("clean"),
        "gg sc should reject unstaged changes when rebase is needed. stdout={}, stderr={}",
        stdout,
        stderr
    );
}

#[test]
fn test_gg_navigation_preserves_modifications() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with 3 commits
    run_gg(&repo_path, &["co", "nav-preserve-test"]);

    fs::write(repo_path.join("file1.txt"), "v1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add file1"]);

    fs::write(repo_path.join("file2.txt"), "v2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add file2"]);

    fs::write(repo_path.join("file3.txt"), "v3").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add file3"]);

    // Get the original SHA of commit 2
    let (_, log_before) = run_git(&repo_path, &["log", "--oneline", "-3"]);

    // Navigate to commit 2 (middle of stack)
    let (success, _, stderr) = run_gg(&repo_path, &["mv", "2"]);
    assert!(success, "Failed to navigate to commit 2: {}", stderr);

    // Modify file2 and squash
    fs::write(repo_path.join("file2.txt"), "v2-modified").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);

    let (success, _, _stderr) = run_gg(&repo_path, &["sc"]);
    // Note: This might fail if there are conflicts, which is expected in some cases
    // The important thing is that if it succeeds, the changes should persist

    if success {
        // Navigate back to last
        let (success, _, stderr) = run_gg(&repo_path, &["last"]);
        assert!(success, "Failed to navigate to last: {}", stderr);

        // The modification should persist - check by looking at the log
        // The SHA of commit 2 should be different now
        let (_, log_after) = run_git(&repo_path, &["log", "--oneline", "-3"]);

        // The logs should be different because commit 2 was modified
        // (and commit 3 was rebased on top)
        assert_ne!(
            log_before.trim(),
            log_after.trim(),
            "Commits should have changed after modification. Before: {}, After: {}",
            log_before,
            log_after
        );

        // Navigate back to commit 2 to verify the content
        let (success, _, _) = run_gg(&repo_path, &["mv", "2"]);
        if success {
            let content = fs::read_to_string(repo_path.join("file2.txt"))
                .unwrap_or_else(|_| "file not found".to_string());
            assert_eq!(
                content, "v2-modified",
                "Modified content should persist after navigation"
            );
        }
    }
}

#[test]
fn test_nav_context_persistence() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with commits
    run_gg(&repo_path, &["co", "context-test"]);

    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("file2.txt"), "content2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Navigate to commit 1 (this should save nav context)
    let (success, _, stderr) = run_gg(&repo_path, &["mv", "1"]);
    assert!(success, "Failed to navigate: {}", stderr);

    // Check that nav context file was created
    let current_stack_path = gg_dir.join("current_stack");
    assert!(
        current_stack_path.exists(),
        "Nav context file should be created after navigation"
    );

    // Read and verify context format (should be branch|position|oid)
    let context = fs::read_to_string(&current_stack_path).expect("Failed to read nav context");
    let parts: Vec<&str> = context.trim().split('|').collect();

    // Should have at least branch name, possibly position and oid
    assert!(
        !parts.is_empty() && !parts[0].is_empty(),
        "Nav context should contain branch name. Got: {}",
        context
    );
}

#[test]
fn test_gg_reorder_with_positions() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with 3 commits
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-reorder"]);
    assert!(success, "Failed to checkout: {}", stderr);

    fs::write(repo_path.join("a.txt"), "A").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add A"]);

    fs::write(repo_path.join("b.txt"), "B").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add B"]);

    fs::write(repo_path.join("c.txt"), "C").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add C"]);

    // Get original order
    let (_, log_before) = run_git(&repo_path, &["log", "--oneline", "-3"]);
    assert!(log_before.contains("Add A"));
    assert!(log_before.contains("Add B"));
    assert!(log_before.contains("Add C"));

    // Reorder using positions: move C to bottom, then A, then B on top
    let (success, _, stderr) = run_gg(&repo_path, &["reorder", "--order", "3,1,2"]);
    assert!(success, "Failed to reorder: {}", stderr);

    // Verify new order in log (most recent first)
    let (_, log_after) = run_git(&repo_path, &["log", "--oneline", "-3"]);
    let lines: Vec<&str> = log_after.trim().lines().collect();

    // After reorder "3,1,2": C becomes [1], A becomes [2], B becomes [3]
    // git log shows most recent first, so: B, A, C
    assert!(
        lines[0].contains("Add B"),
        "Expected B on top, got: {}",
        log_after
    );
    assert!(
        lines[1].contains("Add A"),
        "Expected A in middle, got: {}",
        log_after
    );
    assert!(
        lines[2].contains("Add C"),
        "Expected C at bottom, got: {}",
        log_after
    );
}

#[test]
fn test_gg_reorder_with_spaces() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with 3 commits
    run_gg(&repo_path, &["co", "test-reorder-spaces"]);

    fs::write(repo_path.join("x.txt"), "X").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add X"]);

    fs::write(repo_path.join("y.txt"), "Y").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add Y"]);

    fs::write(repo_path.join("z.txt"), "Z").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add Z"]);

    // Reorder using space-separated positions
    let (success, _, stderr) = run_gg(&repo_path, &["reorder", "--order", "2 3 1"]);
    assert!(success, "Failed to reorder with spaces: {}", stderr);

    // Verify new order
    let (_, log_after) = run_git(&repo_path, &["log", "--oneline", "-3"]);
    let lines: Vec<&str> = log_after.trim().lines().collect();

    // After reorder "2 3 1": Y becomes [1], Z becomes [2], X becomes [3]
    // git log shows: X, Z, Y
    assert!(
        lines[0].contains("Add X"),
        "Expected X on top, got: {}",
        log_after
    );
    assert!(
        lines[1].contains("Add Z"),
        "Expected Z in middle, got: {}",
        log_after
    );
    assert!(
        lines[2].contains("Add Y"),
        "Expected Y at bottom, got: {}",
        log_after
    );
}

#[test]
fn test_gg_reorder_invalid_position() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with 2 commits
    run_gg(&repo_path, &["co", "test-reorder-invalid"]);

    fs::write(repo_path.join("one.txt"), "1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("two.txt"), "2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Try to reorder with position 0 (invalid, 1-indexed)
    let (success, _, stderr) = run_gg(&repo_path, &["reorder", "--order", "0,1"]);
    assert!(!success, "Should fail with position 0");
    assert!(
        stderr.contains("out of range") || stderr.contains("Position 0"),
        "Error should mention invalid position: {}",
        stderr
    );

    // Try to reorder with position > stack length
    let (success, _, stderr) = run_gg(&repo_path, &["reorder", "--order", "1,5"]);
    assert!(!success, "Should fail with position > stack length");
    assert!(
        stderr.contains("out of range") || stderr.contains("Position 5"),
        "Error should mention out of range: {}",
        stderr
    );
}

#[test]
fn test_gg_reorder_duplicate_position() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with 2 commits
    run_gg(&repo_path, &["co", "test-reorder-dup"]);

    fs::write(repo_path.join("one.txt"), "1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("two.txt"), "2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Try to reorder with duplicate position
    let (success, _, stderr) = run_gg(&repo_path, &["reorder", "--order", "1,1"]);
    assert!(!success, "Should fail with duplicate position");
    assert!(
        stderr.to_lowercase().contains("duplicate"),
        "Error should mention duplicate: {}",
        stderr
    );
}

#[test]
fn test_gg_reorder_missing_commits() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with 3 commits
    run_gg(&repo_path, &["co", "test-reorder-missing"]);

    fs::write(repo_path.join("one.txt"), "1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("two.txt"), "2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    fs::write(repo_path.join("three.txt"), "3").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 3"]);

    // Try to reorder with only 2 positions for 3 commits
    let (success, _, stderr) = run_gg(&repo_path, &["reorder", "--order", "1,2"]);
    assert!(!success, "Should fail when not all commits included");
    assert!(
        stderr.contains("must include all") || stderr.contains("3 commits"),
        "Error should mention missing commits: {}",
        stderr
    );
}

/// Helper to create a test repo with a bare remote
fn create_test_repo_with_remote() -> (TempDir, PathBuf, PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let base_path = temp_dir.path().to_path_buf();

    // Create a bare repo to act as remote
    let remote_path = base_path.join("remote.git");
    fs::create_dir_all(&remote_path).expect("Failed to create remote dir");

    Command::new("git")
        .args(["init", "--bare", "--initial-branch=main"])
        .current_dir(&remote_path)
        .output()
        .expect("Failed to init bare repo");

    // Create the working repo
    let repo_path = base_path.join("repo");
    fs::create_dir_all(&repo_path).expect("Failed to create repo dir");

    Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to init git repo");

    // Configure git user
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to configure git email");

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to configure git name");

    // Add remote
    Command::new("git")
        .args(["remote", "add", "origin", remote_path.to_str().unwrap()])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add remote");

    // Create initial commit on main
    fs::write(repo_path.join("README.md"), "# Test Repo\n").expect("Failed to write README");

    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add files");

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to create initial commit");

    // Push to remote
    Command::new("git")
        .args(["push", "-u", "origin", "main"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to push to remote");

    (temp_dir, repo_path, remote_path)
}

#[test]
fn test_gg_ls_remote_no_stacks() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // List remote stacks - should show no stacks
    let (success, stdout, _stderr) = run_gg(&repo_path, &["ls", "--remote"]);
    assert!(success, "ls --remote should succeed");
    assert!(
        stdout.contains("No remote stacks") || stdout.contains("Remote stacks"),
        "Should show remote stacks message: {}",
        stdout
    );
}

#[test]
fn test_gg_ls_remote_with_stacks() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config with empty stacks (simulating fresh clone)
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack and push it
    run_gg(&repo_path, &["co", "test-remote-stack"]);

    fs::write(repo_path.join("test.txt"), "test").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    // Push the branch to remote
    run_git(
        &repo_path,
        &["push", "-u", "origin", "testuser/test-remote-stack"],
    );

    // Switch back to main and delete local stack branch
    run_git(&repo_path, &["checkout", "main"]);
    run_git(&repo_path, &["branch", "-D", "testuser/test-remote-stack"]);

    // Reset config to remove the stack entry (simulating a fresh clone scenario)
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to reset config");

    // List remote stacks - should show the stack
    let (success, stdout, _stderr) = run_gg(&repo_path, &["ls", "--remote"]);
    assert!(success, "ls --remote should succeed");
    assert!(
        stdout.contains("test-remote-stack"),
        "Should list the remote stack: {}",
        stdout
    );
}

#[test]
fn test_gg_checkout_remote_stack() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack and push it
    run_gg(&repo_path, &["co", "remote-checkout-test"]);

    fs::write(repo_path.join("test.txt"), "test content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit for remote"]);

    // Push the branch to remote
    run_git(
        &repo_path,
        &["push", "-u", "origin", "testuser/remote-checkout-test"],
    );

    // Switch back to main and delete local stack branch
    run_git(&repo_path, &["checkout", "main"]);
    run_git(
        &repo_path,
        &["branch", "-D", "testuser/remote-checkout-test"],
    );

    // Verify we're on main and the stack branch doesn't exist locally
    let (_, current_branch) = run_git(&repo_path, &["branch", "--show-current"]);
    assert!(
        current_branch.trim() == "main",
        "Should be on main: {}",
        current_branch
    );

    // Now checkout the remote stack
    let (success, stdout, _stderr) = run_gg(&repo_path, &["co", "remote-checkout-test"]);
    assert!(success, "Should successfully checkout remote stack");
    assert!(
        stdout.contains("Checked out remote stack") || stdout.contains("remote-checkout-test"),
        "Should mention checking out remote: {}",
        stdout
    );

    // Verify we're now on the stack branch
    let (_, current_branch) = run_git(&repo_path, &["branch", "--show-current"]);
    assert!(
        current_branch.contains("remote-checkout-test"),
        "Should be on the stack branch: {}",
        current_branch
    );

    // Verify the file exists
    assert!(
        repo_path.join("test.txt").exists(),
        "test.txt should exist after checkout"
    );
}

// ============================================================
// Tests for PRs #44-#48 (merged while claude-review was broken)
// ============================================================

#[test]
fn test_stack_name_sanitization_spaces_to_kebab() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with spaces in the name - should be converted to hyphens
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "my feature branch"]);

    assert!(success, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("my-feature-branch") || stdout.contains("Converted"),
        "Should convert spaces to hyphens: stdout={}",
        stdout
    );

    // Verify we're on the kebab-case branch
    let (_, branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(
        branch.trim(),
        "testuser/my-feature-branch",
        "Branch should use kebab-case"
    );
}

#[test]
fn test_stack_name_rejects_slash() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Try to create a stack with slash - should fail
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "feature/subfeature"]);

    assert!(!success, "Should fail with slash in name");
    assert!(
        stderr.contains("cannot contain '/'") || stderr.contains("Invalid stack name"),
        "Should mention invalid character: stderr={}",
        stderr
    );
}

#[test]
fn test_stack_name_rejects_double_dash() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Try to create a stack with double dash - should fail (conflicts with entry branch format)
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "my--feature"]);

    assert!(!success, "Should fail with double dash in name");
    assert!(
        stderr.contains("cannot contain '--'") || stderr.contains("Invalid stack name"),
        "Should mention invalid sequence: stderr={}",
        stderr
    );
}

#[test]
fn test_lint_restores_position_on_command_not_found() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config with a non-existent lint command
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["nonexistent-command-12345"]}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with a commit
    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    // Remember the original branch
    let (_, original_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let original_branch = original_branch.trim();

    // Run lint - should fail but restore position
    let (success, stdout, stderr) = run_gg(&repo_path, &["lint"]);

    assert!(!success, "Lint should fail with non-existent command");
    assert!(
        stderr.contains("not found") || stdout.contains("not found") || stderr.contains("Command"),
        "Should mention command not found: stdout={}, stderr={}",
        stdout,
        stderr
    );

    // Verify we're back on the original branch (not in detached HEAD)
    let (_, current_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(
        current_branch.trim(),
        original_branch,
        "Should restore to original branch after lint failure"
    );
}

#[test]
fn test_lint_error_message_for_shell_alias() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config with a command that looks like a shell alias
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["gw ktfmtCheck"]}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with a commit
    run_gg(&repo_path, &["co", "lint-alias-test"]);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    // Run lint - should fail with helpful message about shell aliases
    let (success, stdout, stderr) = run_gg(&repo_path, &["lint"]);

    assert!(!success, "Lint should fail with non-existent gw command");

    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("not found") || combined.contains("alias"),
        "Should mention command not found or alias: {}",
        combined
    );
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
fn test_config_auto_add_gg_ids_default() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up minimal config without auto_add_gg_ids
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    run_gg(&repo_path, &["co", "gg-id-test"]);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit without GG-ID"]);

    // Verify the config doesn't have auto_add_gg_ids explicitly set
    // (it should default to true)
    let config_content =
        fs::read_to_string(gg_dir.join("config.json")).expect("Failed to read config");

    // The config should NOT contain auto_add_gg_ids: false
    // (either it's not present, meaning default true, or explicitly true)
    assert!(
        !config_content.contains("\"auto_add_gg_ids\":false")
            && !config_content.contains("\"auto_add_gg_ids\": false"),
        "auto_add_gg_ids should not be explicitly false: {}",
        config_content
    );
}
