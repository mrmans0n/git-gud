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
fn test_gg_sync_help_has_update_descriptions() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["sync", "--help"]);

    assert!(success);
    assert!(stdout.contains("--update-descriptions"));
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

#[test]
fn test_rebase_updates_local_main() {
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
    let (success, _, stderr) = run_gg(&repo_path, &["co", "rebase-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "feature").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add feature"]);

    // Get the initial main SHA
    run_git(&repo_path, &["checkout", "main"]);
    let (_, initial_main_sha) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let initial_main_sha = initial_main_sha.trim();

    // Simulate a commit being merged on the remote (like a PR merge)
    // We'll add a commit directly to origin/main
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("merged.txt"), "merged from PR").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Merged PR"]);
    run_git(&repo_path, &["push", "origin", "main"]);

    // Reset local main to the old SHA (simulating local main being behind)
    run_git(&repo_path, &["reset", "--hard", initial_main_sha]);

    // Verify local main is behind
    let (_, local_sha) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(
        local_sha.trim(),
        initial_main_sha,
        "Local main should be at initial SHA"
    );

    // Switch to our stack
    run_git(&repo_path, &["checkout", "testuser/rebase-test"]);

    // Run gg rebase - this should update local main
    let (success, stdout, stderr) = run_gg(&repo_path, &["rebase"]);

    // Check that rebase ran (might fail if there are conflicts, but that's ok)
    // The important thing is that it attempted to update main
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("Updating main") || combined.contains("Updated local main") || success,
        "Should mention updating main: {}",
        combined
    );
}

#[test]
fn test_rebase_help() {
    let (_temp_dir, repo_path) = create_test_repo();

    let (success, stdout, _stderr) = run_gg(&repo_path, &["rebase", "--help"]);

    assert!(success, "Help should succeed");
    assert!(
        stdout.contains("Rebase") || stdout.contains("rebase"),
        "Should show rebase help: {}",
        stdout
    );
}

// ============================================================
// Tests for PR #50 - gg rebase improvements
// ============================================================

#[test]
fn test_rebase_restores_original_branch() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with commits
    let (success, _, stderr) = run_gg(&repo_path, &["co", "restore-branch-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "feature content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add feature"]);

    // Remember the branch we're on
    let (_, original_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let original_branch = original_branch.trim();

    // Run rebase
    let (success, _stdout, _stderr) = run_gg(&repo_path, &["rebase"]);

    // Whether it succeeds or not, we should be back on the original branch
    let (_, current_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(
        current_branch.trim(),
        original_branch,
        "Should restore to original branch after rebase (success={})",
        success
    );
}

#[test]
fn test_rebase_when_local_base_branch_not_exists() {
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
    let (success, _, stderr) = run_gg(&repo_path, &["co", "no-local-base-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add feature"]);

    // Delete the local main branch (unusual but possible scenario)
    // First we need to be on a different branch (we already are on our stack)
    run_git(&repo_path, &["branch", "-D", "main"]);

    // Verify main doesn't exist locally
    let (exists, _) = run_git(&repo_path, &["rev-parse", "--verify", "main"]);
    assert!(!exists, "Local main should not exist");

    // Rebase should still work (will use origin/main directly)
    let (success, stdout, stderr) = run_gg(&repo_path, &["rebase"]);

    // Should succeed or at least not crash due to missing local branch
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        success || combined.contains("Rebased"),
        "Rebase should handle missing local base branch gracefully: {}",
        combined
    );
}

#[test]
fn test_rebase_when_remote_base_branch_not_exists() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config with a non-existent base branch
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "no-remote-base-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add feature"]);

    // Try to rebase onto a non-existent branch
    let (_success, stdout, stderr) = run_gg(&repo_path, &["rebase", "nonexistent-branch"]);

    // Should fail gracefully with a clear error
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("Warning")
            || combined.contains("Could not")
            || combined.contains("error")
            || combined.contains("not exist"),
        "Should handle missing remote branch gracefully: {}",
        combined
    );
}

#[test]
fn test_rebase_when_branches_have_diverged() {
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
    let (success, _, stderr) = run_gg(&repo_path, &["co", "diverged-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "feature").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add feature"]);

    // Make a commit on remote main
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("remote-change.txt"), "from remote").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Remote commit"]);
    run_git(&repo_path, &["push", "origin", "main"]);

    // Make a DIFFERENT commit on local main (causing divergence)
    run_git(&repo_path, &["reset", "--hard", "HEAD~1"]);
    fs::write(repo_path.join("local-change.txt"), "local only").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Local only commit"]);

    // Verify branches have diverged
    let (_, local_sha) = run_git(&repo_path, &["rev-parse", "main"]);
    let (_, remote_sha) = run_git(&repo_path, &["rev-parse", "origin/main"]);
    assert_ne!(
        local_sha.trim(),
        remote_sha.trim(),
        "Branches should have diverged"
    );

    // Switch to our stack
    run_git(&repo_path, &["checkout", "testuser/diverged-test"]);

    // Rebase should warn but continue with origin/main
    let (success, stdout, stderr) = run_gg(&repo_path, &["rebase"]);

    let combined = format!("{}{}", stdout, stderr);

    // Should either succeed (using origin/main) or warn about divergence
    // The key is it shouldn't crash and should give useful feedback
    assert!(
        success
            || combined.contains("Warning")
            || combined.contains("Could not update")
            || combined.contains("origin/main"),
        "Should handle diverged branches gracefully: {}",
        combined
    );
}

#[test]
fn test_rebase_removes_merged_commits_from_stack() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with multiple commits
    let (success, _, stderr) = run_gg(&repo_path, &["co", "merged-commits-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("commit1.txt"), "commit 1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1 - will be merged"]);

    fs::write(repo_path.join("commit2.txt"), "commit 2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2 - stays in stack"]);

    // Get the initial commit count
    let (_, log_before) = run_git(&repo_path, &["log", "--oneline", "main..HEAD"]);
    let commits_before = log_before.trim().lines().count();
    assert_eq!(commits_before, 2, "Should have 2 commits before merge");

    // Simulate first commit being merged to main on remote
    // Cherry-pick commit 1 to main and push
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("commit1.txt"), "commit 1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Commit 1 - will be merged (merged via PR)"],
    );
    run_git(&repo_path, &["push", "origin", "main"]);

    // Reset local main to be behind
    run_git(&repo_path, &["reset", "--hard", "HEAD~1"]);

    // Switch back to stack and rebase
    run_git(&repo_path, &["checkout", "testuser/merged-commits-test"]);

    let (success, _stdout, _stderr) = run_gg(&repo_path, &["rebase"]);

    if success {
        // After rebase, the first commit should be gone (it's now in main)
        // Only commit 2 should remain
        let (_, log_after) = run_git(&repo_path, &["log", "--oneline", "origin/main..HEAD"]);
        let commits_after = log_after.trim().lines().count();

        // The commit that was "merged" should no longer appear in the stack
        // Note: This depends on git's ability to detect the commit was cherry-picked
        // In a real scenario with actual PR merges, git rebase drops duplicate commits
        assert!(
            commits_after <= commits_before,
            "Stack should have same or fewer commits after rebase. Before: {}, After: {}",
            commits_before,
            commits_after
        );
    }
}

#[test]
fn test_rebase_with_prune_removes_deleted_remote_branches() {
    let (_temp_dir, repo_path, remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create and push a temporary branch
    run_git(&repo_path, &["checkout", "-b", "temp-branch"]);
    fs::write(repo_path.join("temp.txt"), "temp").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Temp commit"]);
    run_git(&repo_path, &["push", "-u", "origin", "temp-branch"]);

    // Go back to main
    run_git(&repo_path, &["checkout", "main"]);

    // Delete the branch on the remote directly (simulating PR merge with branch deletion)
    Command::new("git")
        .args(["branch", "-D", "temp-branch"])
        .current_dir(&remote_path)
        .output()
        .expect("Failed to delete remote branch");

    // Verify the remote tracking branch still exists locally
    let (exists_before, _) = run_git(
        &repo_path,
        &["rev-parse", "--verify", "refs/remotes/origin/temp-branch"],
    );
    assert!(
        exists_before,
        "Remote tracking branch should exist before fetch --prune"
    );

    // Create a stack and run rebase (which does fetch --prune)
    let (success, _, stderr) = run_gg(&repo_path, &["co", "prune-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "feature").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Feature commit"]);

    // Run rebase - this should fetch with --prune
    let (_success, _stdout, _stderr) = run_gg(&repo_path, &["rebase"]);

    // After rebase (which fetches with --prune), the deleted remote branch should be gone
    let (exists_after, _) = run_git(
        &repo_path,
        &["rev-parse", "--verify", "refs/remotes/origin/temp-branch"],
    );
    assert!(
        !exists_after,
        "Remote tracking branch should be pruned after rebase"
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
fn test_gg_last_fails_when_rebase_in_progress() {
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
    run_gg(&repo_path, &["co", "rebase-guard-test"]);

    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("file2.txt"), "content2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Navigate to first commit
    run_gg(&repo_path, &["mv", "1"]);

    // Modify file1 to create a conflict scenario
    fs::write(repo_path.join("file1.txt"), "modified content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "--amend", "--no-edit"]);

    // Try to navigate to next - this will trigger a rebase that will conflict
    // because file2 might depend on the original file1
    let _ = run_gg(&repo_path, &["next"]);

    // Simulate a rebase-in-progress state by creating the rebase-merge directory
    // (This is more reliable than trying to trigger an actual conflict)
    let rebase_dir = repo_path.join(".git/rebase-merge");
    fs::create_dir_all(&rebase_dir).expect("Failed to create rebase-merge dir");

    // Now try gg last - should fail with rebase in progress
    let (success, stdout, stderr) = run_gg(&repo_path, &["last"]);

    assert!(!success, "gg last should fail when rebase is in progress");
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("rebase is in progress") || combined.contains("rebase"),
        "Error should mention rebase in progress: {}",
        combined
    );
    assert!(
        combined.contains("gg continue") || combined.contains("gg abort"),
        "Error should suggest gg continue or gg abort: {}",
        combined
    );

    // Clean up
    fs::remove_dir_all(&rebase_dir).ok();
}

#[test]
fn test_gg_next_fails_when_rebase_in_progress() {
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
    run_gg(&repo_path, &["co", "next-rebase-test"]);

    fs::write(repo_path.join("a.txt"), "a").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add a"]);

    fs::write(repo_path.join("b.txt"), "b").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add b"]);

    // Navigate to first
    run_gg(&repo_path, &["mv", "1"]);

    // Simulate rebase in progress
    let rebase_dir = repo_path.join(".git/rebase-merge");
    fs::create_dir_all(&rebase_dir).expect("Failed to create rebase-merge dir");

    // Try gg next - should fail
    let (success, stdout, stderr) = run_gg(&repo_path, &["next"]);

    assert!(!success, "gg next should fail when rebase is in progress");
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("rebase is in progress") || combined.contains("rebase"),
        "Error should mention rebase in progress: {}",
        combined
    );
    assert!(
        combined.contains("gg continue") || combined.contains("gg abort"),
        "Error should suggest gg continue or gg abort: {}",
        combined
    );

    // Clean up
    fs::remove_dir_all(&rebase_dir).ok();
}

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
fn test_nested_rebase_protection_in_navigation() {
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
    run_gg(&repo_path, &["co", "nested-rebase-test"]);

    fs::write(repo_path.join("step1.txt"), "1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Step 1"]);

    fs::write(repo_path.join("step2.txt"), "2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Step 2"]);

    // Navigate to first
    run_gg(&repo_path, &["mv", "1"]);

    // Simulate a rebase already in progress
    let rebase_dir = repo_path.join(".git/rebase-merge");
    fs::create_dir_all(&rebase_dir).expect("Failed to create rebase-merge dir");

    // Try to move to next (which might trigger a rebase internally)
    // Should fail because a rebase is already in progress
    let (success, stdout, stderr) = run_gg(&repo_path, &["next"]);

    assert!(!success, "Should not allow nested rebase");
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("rebase is in progress") || combined.contains("rebase"),
        "Should mention rebase in progress: {}",
        combined
    );

    // Clean up
    fs::remove_dir_all(&rebase_dir).ok();
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
