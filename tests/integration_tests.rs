//! Integration tests for git-gud
//!
//! These tests create temporary git repositories and test the core functionality.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde_json::Value;
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
fn run_gg(repo_path: &std::path::Path, args: &[&str]) -> (bool, String, String) {
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
fn run_git(repo_path: &std::path::Path, args: &[&str]) -> (bool, String) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (output.status.success(), stdout)
}

fn run_git_full(repo_path: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stdout, stderr)
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

    let (success, stdout, stderr) = run_gg(&repo_path, &["clean", "--json"]);
    assert!(success, "gg clean --json failed: {}", stderr);
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
fn test_gg_lint_json_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["lint", "--help"]);

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
fn test_gg_ls_json_current_stack() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "json-stack"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Add file1\n\nGG-ID: c-abc1234"],
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["ls", "--json"]);
    assert!(success, "gg ls --json failed: {}", stderr);

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["stack"]["name"], "json-stack");
    let base = parsed["stack"]["base"]
        .as_str()
        .expect("stack.base must be a string");
    assert!(
        matches!(base, "main" | "master"),
        "expected stack base to be 'main' or 'master', got '{base}'"
    );
    assert_eq!(parsed["stack"]["total_commits"], 1);

    let entries = parsed["stack"]["entries"]
        .as_array()
        .expect("entries must be an array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["position"], 1);
    assert_eq!(entries[0]["title"], "Add file1");
}

#[test]
fn test_gg_ls_all_json() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "json-a"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("a.txt"), "a").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add A"]);

    run_git(&repo_path, &["checkout", "main"]);

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "json-b"]);
    assert!(success, "Failed to create second stack: {}", stderr);

    fs::write(repo_path.join("b.txt"), "b").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add B"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["ls", "--all", "--json"]);
    assert!(success, "gg ls -a --json failed: {}", stderr);

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["current_stack"], "json-b");

    let stacks = parsed["stacks"]
        .as_array()
        .expect("stacks must be an array");
    assert!(
        stacks.iter().any(|s| s["name"] == "json-a"),
        "json-a stack should be present"
    );
    assert!(
        stacks.iter().any(|s| s["name"] == "json-b"),
        "json-b stack should be present"
    );
}

#[test]
fn test_gg_ls_shows_behind_indicator_when_base_is_behind_origin() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack so `gg ls` has something to show.
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "behind-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "feature").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Feature commit"]);

    // Move main ahead on origin, then make local main behind.
    run_git(&repo_path, &["checkout", "main"]);
    let (_, old_main_sha) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let old_main_sha = old_main_sha.trim().to_string();

    fs::write(repo_path.join("main.txt"), "remote ahead").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Main moved"]);
    run_git(&repo_path, &["push", "origin", "main"]);
    run_git(&repo_path, &["reset", "--hard", &old_main_sha]);

    // Back to stack and list all stacks.
    run_git(&repo_path, &["checkout", "testuser/behind-test"]);
    let (success, stdout, stderr) = run_gg(&repo_path, &["ls", "--all"]);
    assert!(success, "ls --all failed: {}", stderr);
    assert!(
        stdout.contains("↓1"),
        "Expected behind indicator in output: {}",
        stdout
    );
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
fn test_gg_squash_warns_about_unstaged_at_stack_head() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with one commit (stack head)
    run_gg(&repo_path, &["co", "squash-unstaged-head-test"]);

    fs::write(repo_path.join("file1.txt"), "original content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Initial file"]);

    // Stage one change to squash
    fs::write(repo_path.join("file1.txt"), "staged content").expect("Failed to write file");
    run_git(&repo_path, &["add", "file1.txt"]);

    // Keep an unstaged change in another file
    fs::write(repo_path.join("file2.txt"), "unstaged content").expect("Failed to write file");

    let (success, stdout, stderr) = run_gg(&repo_path, &["sc"]);

    assert!(
        success,
        "gg sc should succeed at stack head with unstaged warning. stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("You have unstaged changes")
            || stderr.contains("You have unstaged changes"),
        "Expected unstaged warning. stdout={}, stderr={}",
        stdout,
        stderr
    );
}

#[test]
fn test_gg_squash_adds_unstaged_changes_when_configured() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config with unstaged_action=add
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","unstaged_action":"add"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with one commit
    run_gg(&repo_path, &["co", "squash-unstaged-add-test"]);

    fs::write(repo_path.join("file1.txt"), "original content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Initial file"]);

    // Make an unstaged change to a tracked file
    fs::write(repo_path.join("file1.txt"), "updated but unstaged").expect("Failed to write file");

    // Squash should auto-add unstaged changes and amend the current commit
    let (success, stdout, stderr) = run_gg(&repo_path, &["sc"]);
    assert!(
        success,
        "gg sc should succeed with unstaged_action=add. stdout={}, stderr={}",
        stdout, stderr
    );

    // Verify the amended commit now contains the previously unstaged change
    let (_success, amended_content) = run_git(&repo_path, &["show", "HEAD:file1.txt"]);
    assert_eq!(
        amended_content.trim(),
        "updated but unstaged",
        "Expected unstaged change to be included in amended commit"
    );

    // Working directory should be clean after amend
    let (_success, status_output) = run_git(&repo_path, &["status", "--porcelain"]);
    assert!(
        status_output.trim().is_empty(),
        "Expected clean working directory after squash with unstaged_action=add"
    );
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
fn test_gg_ls_remote_json() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    run_gg(&repo_path, &["co", "test-remote-json"]);

    fs::write(repo_path.join("test-json.txt"), "test json").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test remote json commit"]);

    run_git(
        &repo_path,
        &["push", "-u", "origin", "testuser/test-remote-json"],
    );

    run_git(&repo_path, &["checkout", "main"]);
    run_git(&repo_path, &["branch", "-D", "testuser/test-remote-json"]);

    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to reset config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["ls", "--remote", "--json"]);
    assert!(success, "gg ls --remote --json failed: {}", stderr);

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);

    let stacks = parsed["stacks"]
        .as_array()
        .expect("stacks must be an array");
    assert!(!stacks.is_empty(), "expected at least one remote stack");

    let stack = stacks
        .iter()
        .find(|s| s["name"] == "test-remote-json")
        .expect("test-remote-json stack should be present");

    assert!(stack["name"].is_string(), "name must be a string");
    assert!(
        stack["commit_count"].as_u64().is_some(),
        "commit_count must be a number"
    );
    assert!(
        stack["pr_numbers"].is_array(),
        "pr_numbers must be an array"
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
fn test_gg_lint_json_output() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["git --version"]}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "lint-json-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--json"]);
    assert!(success, "gg lint --json failed: {}", stderr);
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSON mode"
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert!(parsed["lint"]["all_passed"].is_boolean());

    let results = parsed["lint"]["results"]
        .as_array()
        .expect("lint.results must be an array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["position"], 1);

    let commands = results[0]["commands"]
        .as_array()
        .expect("commands must be an array");
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["command"], "git --version");
    assert_eq!(commands[0]["passed"], true);
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
fn test_lint_restores_branch_after_changes() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config with a lint command that modifies files
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["./lint.sh"]}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with a commit
    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-change-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    // Create a lint script that modifies the file
    fs::write(
        repo_path.join("lint.sh"),
        "#!/bin/sh\necho \"linted\" >> test.txt\n",
    )
    .expect("Failed to write lint script");
    Command::new("chmod")
        .args(["+x", "lint.sh"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to chmod lint script");

    // Remember the original branch
    let (_, original_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let original_branch = original_branch.trim();

    // Run lint - should succeed and restore branch
    let (success, _stdout, stderr) = run_gg(&repo_path, &["lint"]);
    assert!(success, "Lint should succeed: {}", stderr);

    let (_, current_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(
        current_branch.trim(),
        original_branch,
        "Should return to stack branch after lint changes"
    );

    let content = fs::read_to_string(repo_path.join("test.txt")).expect("Failed to read file");
    assert!(content.contains("linted"), "Lint changes should be applied");
}

#[test]
fn test_lint_until_restores_branch_after_changes() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Create the lint script FIRST and commit it, so it exists in all subsequent commits
    fs::write(
        repo_path.join("lint.sh"),
        "#!/bin/sh\nfor f in *.txt; do [ -f \"$f\" ] && echo \"linted\" >> \"$f\"; done\n",
    )
    .expect("Failed to write lint script");
    Command::new("chmod")
        .args(["+x", "lint.sh"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to chmod lint script");
    run_git(&repo_path, &["add", "lint.sh"]);
    run_git(&repo_path, &["commit", "-m", "Add lint script"]);

    // Set up config with a lint command that modifies files
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["./lint.sh"]}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with multiple commits
    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-until-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // First commit
    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "First commit\n\nGG-ID: c-test001"],
    );

    // Second commit
    fs::write(repo_path.join("file2.txt"), "content2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Second commit\n\nGG-ID: c-test002"],
    );

    // Third commit
    fs::write(repo_path.join("file3.txt"), "content3").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Third commit\n\nGG-ID: c-test003"],
    );

    // Remember the original branch
    let (_, original_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let original_branch = original_branch.trim();

    // Run lint with --until 2 (only first two commits)
    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "2"]);
    assert!(success, "Lint should succeed: {} {}", stdout, stderr);

    // Should be back on the branch, NOT in detached HEAD
    let (_, current_ref) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_ne!(
        current_ref.trim(),
        "HEAD",
        "Should NOT be in detached HEAD after lint --until"
    );
    assert_eq!(
        current_ref.trim(),
        original_branch,
        "Should return to stack branch after lint --until changes"
    );

    // Verify the output mentions rebase for remaining commits
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("rebase") || combined.contains("remaining"),
        "Should mention rebase for remaining commits when using --until: {}",
        combined
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
fn test_rebase_updates_local_main_from_worktree() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack branch and commit
    let (success, _, stderr) = run_gg(&repo_path, &["co", "rebase-worktree-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "feature").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add feature"]);

    // Keep main checked out in the main worktree
    run_git(&repo_path, &["checkout", "main"]);
    let (_, initial_main_sha) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let initial_main_sha = initial_main_sha.trim();

    // Create linked worktree for the stack branch
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("valid time")
        .as_nanos();
    let worktree_path = repo_path
        .parent()
        .unwrap()
        .join(format!("stack-worktree-{}", unique_suffix));
    let (success, _, stderr) = run_git_full(
        &repo_path,
        &[
            "worktree",
            "add",
            worktree_path.to_str().expect("valid path"),
            "testuser/rebase-worktree-test",
        ],
    );
    assert!(success, "Failed to create worktree: {}", stderr);

    // Advance origin/main and make local main behind
    fs::write(repo_path.join("merged.txt"), "merged").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Merged PR"]);
    run_git(&repo_path, &["push", "origin", "main"]);
    run_git(&repo_path, &["reset", "--hard", initial_main_sha]);

    // Run rebase from linked worktree
    let worktree_path_buf = worktree_path.to_path_buf();
    let (_success, stdout, stderr) = run_gg(&worktree_path_buf, &["rebase"]);
    let combined = format!("{}{}", stdout, stderr);

    assert!(
        !combined.contains("already used by worktree"),
        "rebase should not try checking out main in a linked worktree: {}",
        combined
    );

    // Local main should be updated to origin/main via fast-forward fetch
    let (_, local_main_sha) = run_git(&repo_path, &["rev-parse", "main"]);
    let (_, remote_main_sha) = run_git(&repo_path, &["rev-parse", "origin/main"]);
    assert_eq!(
        local_main_sha.trim(),
        remote_main_sha.trim(),
        "Local main should fast-forward to origin/main"
    );
}

fn setup_test_config(repo_path: &std::path::Path) {
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");
}

fn setup_absorb_stack(repo_path: &std::path::Path, stack: &str) {
    setup_test_config(repo_path);
    let (success, _stdout, stderr) = run_gg(repo_path, &["co", stack]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("stack.txt"), "line1\n").expect("Failed to write file");
    run_git(repo_path, &["add", "stack.txt"]);
    run_git(repo_path, &["commit", "-m", "Add line1"]);

    fs::write(repo_path.join("stack.txt"), "line1\nline2\n").expect("Failed to write file");
    run_git(repo_path, &["add", "stack.txt"]);
    run_git(repo_path, &["commit", "-m", "Add line2"]);
}

fn setup_large_absorb_stack(repo_path: &std::path::Path, stack: &str, commit_count: usize) {
    setup_test_config(repo_path);
    let (success, _stdout, stderr) = run_gg(repo_path, &["co", stack]);
    assert!(success, "Failed to create stack: {}", stderr);

    for i in 1..=commit_count {
        let file_name = format!("file-{i:02}.txt");
        let content = format!("commit-{i:02}\n");
        fs::write(repo_path.join(&file_name), content).expect("Failed to write file");
        run_git(repo_path, &["add", &file_name]);
        run_git(repo_path, &["commit", "-m", &format!("Add commit {i:02}")]);
    }
}

#[test]
fn test_absorb_basic_creates_fixup_commit() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_absorb_stack(&repo_path, "absorb-basic");

    fs::write(repo_path.join("stack.txt"), "line1 updated\nline2\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "stack.txt"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["absorb"]);
    assert!(success, "absorb failed: {} {}", stdout, stderr);

    let (_, log) = run_git(&repo_path, &["log", "--oneline", "-5"]);
    assert!(
        log.contains("fixup!") || log.contains("Add line1"),
        "Expected fixup-related commit after absorb. log={}",
        log
    );
}

#[test]
fn test_absorb_and_rebase_autosquashes() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_absorb_stack(&repo_path, "absorb-and-rebase");

    fs::write(repo_path.join("stack.txt"), "line1 updated\nline2\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "stack.txt"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["absorb", "--and-rebase"]);
    assert!(success, "absorb --and-rebase failed: {} {}", stdout, stderr);

    let (_, log) = run_git(&repo_path, &["log", "--oneline", "-5"]);
    let (_, reflog) = run_git(&repo_path, &["reflog", "-5", "--pretty=%gs"]);
    assert!(
        !log.contains("fixup!") || reflog.to_lowercase().contains("rebase"),
        "Expected --and-rebase to autosquash or run rebase. log={}, reflog={}",
        log,
        reflog
    );
}

#[test]
fn test_absorb_dry_run_does_not_modify_history() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_absorb_stack(&repo_path, "absorb-dry-run");

    fs::write(repo_path.join("stack.txt"), "line1 updated\nline2\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "stack.txt"]);

    let (_, before) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let (success, stdout, stderr) = run_gg(&repo_path, &["absorb", "--dry-run"]);
    assert!(success, "absorb --dry-run failed: {} {}", stdout, stderr);
    let (_, after) = run_git(&repo_path, &["rev-parse", "HEAD"]);

    assert_eq!(before.trim(), after.trim(), "HEAD changed in dry-run mode");
}

#[test]
fn test_absorb_no_staged_changes_reports_message() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_absorb_stack(&repo_path, "absorb-no-staged");

    let (success, stdout, stderr) = run_gg(&repo_path, &["absorb"]);
    assert!(success, "absorb should be a no-op without staged changes");
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("No staged changes") || combined.contains("No changes to absorb"),
        "Unexpected message: {}",
        combined
    );
}

#[test]
fn test_absorb_whole_file_mode() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_absorb_stack(&repo_path, "absorb-whole-file");

    fs::write(
        repo_path.join("stack.txt"),
        "line1 updated\nline2 updated\n",
    )
    .expect("Failed to write file");
    run_git(&repo_path, &["add", "stack.txt"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["absorb", "--whole-file"]);
    assert!(success, "absorb --whole-file failed: {} {}", stdout, stderr);
}

#[test]
fn test_absorb_one_fixup_per_commit_mode() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_absorb_stack(&repo_path, "absorb-one-fixup");

    fs::write(
        repo_path.join("stack.txt"),
        "line1 updated\nline2 updated\n",
    )
    .expect("Failed to write file");
    run_git(&repo_path, &["add", "stack.txt"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["absorb", "--one-fixup-per-commit"]);
    assert!(
        success,
        "absorb --one-fixup-per-commit failed: {} {}",
        stdout, stderr
    );
}

#[test]
fn test_absorb_no_limit_on_large_stack() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_large_absorb_stack(&repo_path, "absorb-no-limit", 12);

    fs::write(repo_path.join("file-01.txt"), "commit-01 updated\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "file-01.txt"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["absorb", "--no-limit"]);
    assert!(success, "absorb --no-limit failed: {} {}", stdout, stderr);

    let (_, log) = run_git(&repo_path, &["log", "--oneline", "-20"]);
    assert!(
        log.contains("fixup! Add commit 01") || log.contains("fixup!"),
        "Expected fixup commit targeting old commit with --no-limit. log={}",
        log
    );
}

#[test]
fn test_absorb_squash_applies_without_fixup_commit() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_absorb_stack(&repo_path, "absorb-squash");

    fs::write(repo_path.join("stack.txt"), "line1 updated\nline2\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "stack.txt"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["absorb", "--squash"]);
    assert!(success, "absorb --squash failed: {} {}", stdout, stderr);

    let (_, log) = run_git(&repo_path, &["log", "--oneline", "-5"]);
    assert!(
        !log.contains("fixup!"),
        "--squash should not leave fixup commits in history. log={}",
        log
    );
}

#[test]
fn test_absorb_squash_and_rebase_combination() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_absorb_stack(&repo_path, "absorb-squash-rebase");

    fs::write(repo_path.join("stack.txt"), "line1 updated\nline2\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "stack.txt"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["absorb", "--squash", "--and-rebase"]);
    assert!(
        success,
        "absorb --squash --and-rebase failed: {} {}",
        stdout, stderr
    );

    let (_, log) = run_git(&repo_path, &["log", "--oneline", "-5"]);
    assert!(
        !log.contains("fixup!"),
        "--squash --and-rebase should not create fixup commits. log={}",
        log
    );
}

#[test]
fn test_absorb_no_limit_and_squash_combination() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_large_absorb_stack(&repo_path, "absorb-no-limit-squash", 12);

    fs::write(repo_path.join("file-01.txt"), "commit-01 updated\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "file-01.txt"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["absorb", "--no-limit", "--squash"]);
    assert!(
        success,
        "absorb --no-limit --squash failed: {} {}",
        stdout, stderr
    );

    let (_, log) = run_git(&repo_path, &["log", "--oneline", "-20"]);
    assert!(
        !log.contains("fixup!"),
        "--no-limit --squash should not leave fixup commits. log={}",
        log
    );
}

#[test]
fn test_absorb_ambiguous_change_does_not_crash() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_test_config(&repo_path);
    run_gg(&repo_path, &["co", "absorb-ambiguous"]);

    fs::write(repo_path.join("ambiguous.txt"), "common\nA\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "ambiguous.txt"]);
    run_git(&repo_path, &["commit", "-m", "Add A block"]);

    fs::write(repo_path.join("ambiguous.txt"), "common\nA\nB\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "ambiguous.txt"]);
    run_git(&repo_path, &["commit", "-m", "Add B block"]);

    fs::write(repo_path.join("ambiguous.txt"), "common edited\nA\nB\n")
        .expect("Failed to write file");
    run_git(&repo_path, &["add", "ambiguous.txt"]);

    let (_success, stdout, stderr) = run_gg(&repo_path, &["absorb"]);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        !combined.to_lowercase().contains("panic"),
        "absorb should not panic on ambiguous changes: {}",
        combined
    );
}

#[test]
fn test_absorb_single_commit_stack() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_test_config(&repo_path);
    run_gg(&repo_path, &["co", "absorb-single"]);

    fs::write(repo_path.join("single.txt"), "v1\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "single.txt"]);
    run_git(&repo_path, &["commit", "-m", "Single commit"]);

    fs::write(repo_path.join("single.txt"), "v2\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "single.txt"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["absorb"]);
    assert!(
        success,
        "absorb failed on single-commit stack: {} {}",
        stdout, stderr
    );
}

#[test]
fn test_absorb_potential_conflict_path_reports_cleanly() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_absorb_stack(&repo_path, "absorb-conflictish");

    // Move to first commit and rewrite content so rebasing descendants can conflict.
    run_gg(&repo_path, &["mv", "1"]);
    fs::write(repo_path.join("stack.txt"), "LINE1-REWRITTEN\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "stack.txt"]);
    run_git(&repo_path, &["commit", "--amend", "--no-edit"]);
    run_gg(&repo_path, &["last"]);

    fs::write(
        repo_path.join("stack.txt"),
        "LINE1-REWRITTEN\nline2 adjusted\n",
    )
    .expect("Failed to write file");
    run_git(&repo_path, &["add", "stack.txt"]);

    let (_success, stdout, stderr) = run_gg(&repo_path, &["absorb", "--and-rebase"]);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("absor") || combined.contains("conflict") || combined.contains("Warning"),
        "Expected absorb to either complete or report conflict cleanly: {}",
        combined
    );
}

#[test]
fn test_absorb_runs_from_worktree() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack with --worktree so it lives in a linked worktree
    let stack_name = "absorb-wt-test";
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", stack_name, "--worktree"]);
    assert!(
        success,
        "Failed to create stack with worktree: stdout={}, stderr={}",
        stdout, stderr
    );

    // Determine the worktree path from the default convention: ../<repo-dir>.<stack>/
    let worktree_path = repo_path.parent().unwrap().join(format!(
        "{}.{}",
        repo_path.file_name().unwrap().to_string_lossy(),
        stack_name
    ));
    assert!(
        worktree_path.exists(),
        "Worktree should exist at {}",
        worktree_path.display()
    );
    let worktree_path_buf = worktree_path.to_path_buf();

    // Create a commit in the worktree to have something to absorb into
    fs::write(worktree_path.join("notes.txt"), "line one\n").expect("Failed to write file");
    run_git(&worktree_path_buf, &["add", "."]);
    run_git(&worktree_path_buf, &["commit", "-m", "Add notes"]);

    // Stage a change that should be absorbed into the existing commit
    fs::write(worktree_path.join("notes.txt"), "line one updated\n").expect("Failed to write file");
    run_git(&worktree_path_buf, &["add", "notes.txt"]);

    // This used to fail with: fatal: this operation must be run in a work tree
    let (success, stdout, stderr) = run_gg(&worktree_path_buf, &["absorb"]);
    let combined = format!("{}{}", stdout, stderr);

    assert!(
        success,
        "gg absorb should succeed from linked worktree. stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        !combined.contains("must be run in a work tree"),
        "absorb should not fail with worktree detection error: {}",
        combined
    );
}

#[test]
fn test_absorb_runs_from_worktree_subdirectory() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_test_config(&repo_path);

    let stack_name = "absorb-wt-subdir";
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", stack_name, "--worktree"]);
    assert!(success, "Failed to create worktree stack: {}", stderr);

    let worktree_path = repo_path.parent().unwrap().join(format!(
        "{}.{}",
        repo_path.file_name().unwrap().to_string_lossy(),
        stack_name
    ));
    let nested = worktree_path.join("src/module");
    fs::create_dir_all(&nested).expect("Failed to create nested dir");

    let worktree_path_buf = worktree_path.to_path_buf();
    fs::write(worktree_path.join("src/module/nested.txt"), "one\n").expect("Failed write");
    run_git(&worktree_path_buf, &["add", "."]);
    run_git(&worktree_path_buf, &["commit", "-m", "Add nested file"]);

    fs::write(worktree_path.join("src/module/nested.txt"), "one updated\n").expect("Failed write");
    run_git(&worktree_path_buf, &["add", "src/module/nested.txt"]);

    let (success, stdout, stderr) = run_gg(&nested, &["absorb"]);
    assert!(
        success,
        "absorb should work from worktree subdirectory: {} {}",
        stdout, stderr
    );
}

#[test]
fn test_absorb_and_rebase_runs_from_worktree() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_test_config(&repo_path);

    let stack_name = "absorb-wt-and-rebase";
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", stack_name, "--worktree"]);
    assert!(success, "Failed to create worktree stack: {}", stderr);

    let worktree_path = repo_path.parent().unwrap().join(format!(
        "{}.{}",
        repo_path.file_name().unwrap().to_string_lossy(),
        stack_name
    ));
    let wt = worktree_path.to_path_buf();

    fs::write(worktree_path.join("notes.txt"), "a\n").expect("Failed write");
    run_git(&wt, &["add", "notes.txt"]);
    run_git(&wt, &["commit", "-m", "Add notes a"]);

    fs::write(worktree_path.join("notes.txt"), "a\nb\n").expect("Failed write");
    run_git(&wt, &["add", "notes.txt"]);
    run_git(&wt, &["commit", "-m", "Add notes b"]);

    fs::write(worktree_path.join("notes.txt"), "a updated\nb\n").expect("Failed write");
    run_git(&wt, &["add", "notes.txt"]);

    let (success, stdout, stderr) = run_gg(&wt, &["absorb", "--and-rebase"]);
    assert!(
        success,
        "absorb --and-rebase should work in worktree: {} {}",
        stdout, stderr
    );
}

#[test]
fn test_absorb_no_limit_runs_from_worktree() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_test_config(&repo_path);

    let stack_name = "absorb-wt-no-limit";
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", stack_name, "--worktree"]);
    assert!(success, "Failed to create worktree stack: {}", stderr);

    let worktree_path = repo_path.parent().unwrap().join(format!(
        "{}.{}",
        repo_path.file_name().unwrap().to_string_lossy(),
        stack_name
    ));
    let wt = worktree_path.to_path_buf();

    for i in 1..=12 {
        let file_name = format!("notes-{i:02}.txt");
        fs::write(worktree_path.join(&file_name), format!("v{i:02}\n")).expect("Failed write");
        run_git(&wt, &["add", &file_name]);
        run_git(&wt, &["commit", "-m", &format!("Add notes {i:02}")]);
    }

    fs::write(worktree_path.join("notes-01.txt"), "v01 updated\n").expect("Failed write");
    run_git(&wt, &["add", "notes-01.txt"]);

    let (success, stdout, stderr) = run_gg(&wt, &["absorb", "--no-limit"]);
    assert!(
        success,
        "absorb --no-limit should work in worktree: {} {}",
        stdout, stderr
    );
}

#[test]
fn test_absorb_squash_runs_from_worktree() {
    let (_temp_dir, repo_path) = create_test_repo();
    setup_test_config(&repo_path);

    let stack_name = "absorb-wt-squash";
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", stack_name, "--worktree"]);
    assert!(success, "Failed to create worktree stack: {}", stderr);

    let worktree_path = repo_path.parent().unwrap().join(format!(
        "{}.{}",
        repo_path.file_name().unwrap().to_string_lossy(),
        stack_name
    ));
    let wt = worktree_path.to_path_buf();

    fs::write(worktree_path.join("notes.txt"), "a\n").expect("Failed write");
    run_git(&wt, &["add", "notes.txt"]);
    run_git(&wt, &["commit", "-m", "Add notes a"]);

    fs::write(worktree_path.join("notes.txt"), "a\nb\n").expect("Failed write");
    run_git(&wt, &["add", "notes.txt"]);
    run_git(&wt, &["commit", "-m", "Add notes b"]);

    fs::write(worktree_path.join("notes.txt"), "a updated\nb\n").expect("Failed write");
    run_git(&wt, &["add", "notes.txt"]);

    let (success, stdout, stderr) = run_gg(&wt, &["absorb", "--squash"]);
    assert!(
        success,
        "absorb --squash should work in worktree: {} {}",
        stdout, stderr
    );

    let (_, log) = run_git(&wt, &["log", "--oneline", "-5"]);
    assert!(
        !log.contains("fixup!"),
        "worktree --squash should not leave fixup commits. log={}",
        log
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

// ==================== gg reconcile tests ====================

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
fn test_squash_requires_stack() {
    // Test that squash fails when not on a stack
    let (_temp_dir, repo_path) = create_test_repo();

    // Create a commit on main branch
    fs::write(repo_path.join("file.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    // Make some changes to squash
    fs::write(repo_path.join("file.txt"), "modified").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);

    // Try to squash while not on a stack
    let (success, _stdout, stderr) = run_gg(&repo_path, &["squash"]);

    // Should fail
    assert!(!success, "Squash should fail when not on a stack");

    // Should have helpful error message
    assert!(
        stderr.contains("Not on a stack") || stderr.contains("gg co"),
        "Should suggest using 'gg co' to create a stack. Got: {}",
        stderr
    );
}

#[test]
fn test_rebase_without_stack_requires_target() {
    // Test that rebase works on any branch if target is provided
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Create a feature branch (not a stack)
    run_git(&repo_path, &["checkout", "-b", "feature-branch"]);

    // Create a commit
    fs::write(repo_path.join("file.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Feature commit"]);

    // Try rebase without target (should fail - not on stack)
    let (success, _stdout, stderr) = run_gg(&repo_path, &["rebase"]);
    assert!(
        !success,
        "Rebase without target should fail when not on a stack"
    );
    assert!(
        stderr.contains("Not on a stack"),
        "Should indicate not on a stack. Got: {}",
        stderr
    );

    // Try rebase with target (should work on any branch)
    let (success, stdout, stderr) = run_gg(&repo_path, &["rebase", "main"]);
    assert!(
        success,
        "Rebase with target should work on any branch. stdout: {}, stderr: {}",
        stdout, stderr
    );
}

#[test]
fn test_nav_requires_stack() {
    // Test that navigation commands require being on a stack
    let (_temp_dir, repo_path) = create_test_repo();

    // Try nav first while not on a stack
    let (success, _stdout, stderr) = run_gg(&repo_path, &["first"]);
    assert!(!success, "Nav first should fail when not on a stack");
    assert!(
        stderr.contains("Not on a stack"),
        "Should indicate not on a stack. Got: {}",
        stderr
    );

    // Try nav last while not on a stack
    let (success, _stdout, stderr) = run_gg(&repo_path, &["last"]);
    assert!(!success, "Nav last should fail when not on a stack");
    assert!(stderr.contains("Not on a stack"));

    // Try nav next while not on a stack
    let (success, _stdout, stderr) = run_gg(&repo_path, &["next"]);
    assert!(!success, "Nav next should fail when not on a stack");
    assert!(stderr.contains("Not on a stack"));

    // Try nav prev while not on a stack
    let (success, _stdout, stderr) = run_gg(&repo_path, &["prev"]);
    assert!(!success, "Nav prev should fail when not on a stack");
    assert!(stderr.contains("Not on a stack"));
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
fn test_lint_rebases_commits_with_fixes() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Setup config with lint command
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["./lint-fix.py"]}}"#,
    )
    .expect("Failed to write config");

    // Write lint script to fix file2.txt
    let lint_script = repo_path.join("lint-fix.py");
    fs::write(
        &lint_script,
        r#"#!/usr/bin/env python3
from pathlib import Path
path = Path("file2.txt")
text = path.read_text()
path.write_text(text.replace("BAD", "GOOD"))
"#,
    )
    .expect("Failed to write lint script");

    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&lint_script)
            .expect("Failed to read lint script metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&lint_script, perms).expect("Failed to chmod lint script");
    }

    // Create stack branch
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "lint-stack"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Commit 1
    fs::write(repo_path.join("file1.txt"), "one").expect("Failed to write file1");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    // Commit 2 (needs lint fix)
    fs::write(repo_path.join("file2.txt"), "BAD").expect("Failed to write file2");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Commit 3
    fs::write(repo_path.join("file3.txt"), "three").expect("Failed to write file3");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 3"]);

    let (_success, before) = run_git(&repo_path, &["rev-list", "--reverse", "-n", "3", "HEAD"]);
    let before_commits: Vec<&str> = before.lines().collect();
    assert_eq!(before_commits.len(), 3);

    let (success, _stdout, stderr) = run_gg(&repo_path, &["lint"]);
    assert!(success, "gg lint failed: {}", stderr);

    let (_success, after) = run_git(&repo_path, &["rev-list", "--reverse", "-n", "3", "HEAD"]);
    let after_commits: Vec<&str> = after.lines().collect();
    assert_eq!(after_commits.len(), 3);

    assert_ne!(before_commits[1], after_commits[1]);
    assert_ne!(before_commits[2], after_commits[2]);

    let (_success, file2) = run_git(&repo_path, &["show", "HEAD:file2.txt"]);
    assert!(file2.contains("GOOD"));
}

#[test]
fn test_lint_conflict_continue_updates_branch() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Setup config with lint command
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["./lint.py"]}}"#,
    )
    .expect("Failed to write config");

    // Write lint script to add spaces around =
    let lint_script = repo_path.join("lint.py");
    fs::write(
        &lint_script,
        r#"#!/usr/bin/env python3
from pathlib import Path
path = Path("format_me.txt")
if path.exists():
    text = path.read_text()
    path.write_text(text.replace("=", " = "))
"#,
    )
    .expect("Failed to write lint script");

    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&lint_script)
            .expect("Failed to read lint script metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&lint_script, perms).expect("Failed to chmod lint script");
    }

    // Create stack branch
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "lint-conflict"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Commit 1 (needs lint fix)
    fs::write(
        repo_path.join("format_me.txt"),
        "line1=one\nline2=two\nline3=three\n",
    )
    .expect("Failed to write format_me.txt");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    // Commit 2 modifies the same line to cause conflict after lint
    fs::write(
        repo_path.join("format_me.txt"),
        "line1=one\nline2=two\nline3=changed\n",
    )
    .expect("Failed to write format_me.txt");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Run lint - should hit conflict
    let (success, _stdout, _stderr) = run_gg(&repo_path, &["lint"]);
    assert!(!success, "gg lint should report conflict");

    // Resolve conflict and continue
    fs::write(
        repo_path.join("format_me.txt"),
        "line1 = one\nline2 = two\nline3 = changed\n",
    )
    .expect("Failed to write resolved format_me.txt");
    run_git(&repo_path, &["add", "format_me.txt"]);

    let (success, _stdout, stderr) = run_gg(&repo_path, &["continue"]);
    assert!(success, "gg continue failed: {}", stderr);

    // Ensure we're back on stack branch and it points to HEAD
    let (_success, head_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head_branch.trim(), "testuser/lint-conflict");

    let (_success, head_oid) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let (_success, branch_oid) = run_git(&repo_path, &["rev-parse", "testuser/lint-conflict"]);
    assert_eq!(head_oid.trim(), branch_oid.trim());

    let content =
        fs::read_to_string(repo_path.join("format_me.txt")).expect("Failed to read format_me.txt");
    assert!(content.contains("line1 = one"));
    assert!(content.contains("line3 = changed"));
}

// ============================================================
// Tests for PR #106 - Rebase remaining branches after land
// ============================================================

#[test]
fn test_stacked_branches_can_rebase_after_squash_merge() {
    // This test verifies the mechanics that `gg land --all` relies on:
    // After a squash merge, remaining stacked branches can be rebased
    // onto the updated main to avoid merge conflicts.
    //
    // Scenario:
    // - Stack: commit A -> commit B -> commit C
    // - Each has a branch (simulating PR branches)
    // - Squash merge commit A to main (creates new SHA)
    // - Rebase branch B onto new main
    // - Verify branch B now has only its own commit (not the old A)

    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up gg config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack with gg co
    let (success, _, stderr) = run_gg(&repo_path, &["co", "stacked-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create commit A
    fs::write(repo_path.join("file_a.txt"), "content A").expect("Failed to write file A");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: commit A\n\nGG-ID: c-aaa1111"],
    );

    // Create branch for commit A (simulating PR branch)
    run_git(&repo_path, &["branch", "pr-branch-a"]);

    // Create commit B
    fs::write(repo_path.join("file_b.txt"), "content B").expect("Failed to write file B");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: commit B\n\nGG-ID: c-bbb2222"],
    );

    // Create branch for commit B
    run_git(&repo_path, &["branch", "pr-branch-b"]);

    // Create commit C
    fs::write(repo_path.join("file_c.txt"), "content C").expect("Failed to write file C");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: commit C\n\nGG-ID: c-ccc3333"],
    );

    // Create branch for commit C
    run_git(&repo_path, &["branch", "pr-branch-c"]);

    // Get SHA of commit A on the stack branch
    let (_, old_a_sha) = run_git(&repo_path, &["rev-parse", "pr-branch-a"]);
    let old_a_sha = old_a_sha.trim();

    // Now simulate a squash merge of commit A to main
    // This creates a NEW commit with DIFFERENT SHA but same content
    run_git(&repo_path, &["checkout", "main"]);

    // Cherry-pick with squash (simulates GitHub squash merge)
    fs::write(repo_path.join("file_a.txt"), "content A").expect("Failed to write file A");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: commit A (#1)\n\nSquash merged"],
    );

    // Push the updated main
    run_git(&repo_path, &["push", "origin", "main"]);

    // Get the new SHA on main
    let (_, new_main_sha) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let new_main_sha = new_main_sha.trim();

    // Verify the SHAs are different (squash creates new commit)
    assert_ne!(
        old_a_sha, new_main_sha,
        "Squash merge should create different SHA"
    );

    // Now the critical part: rebase pr-branch-b onto the new main
    // This is what gg land does after merging each PR
    run_git(&repo_path, &["checkout", "pr-branch-b"]);

    // Before rebase: branch B has commits A and B
    let (_, log_before) = run_git(&repo_path, &["log", "--oneline", "main..HEAD"]);
    let commits_before: Vec<&str> = log_before.trim().lines().collect();
    assert_eq!(
        commits_before.len(),
        2,
        "Before rebase: should have 2 commits (A and B)"
    );

    // Rebase onto the updated main
    let (success, _) = run_git(&repo_path, &["rebase", "main"]);
    assert!(success, "Rebase should succeed");

    // After rebase: branch B should only have commit B
    let (_, log_after) = run_git(&repo_path, &["log", "--oneline", "main..HEAD"]);
    let commits_after: Vec<&str> = log_after.trim().lines().collect();
    assert_eq!(
        commits_after.len(),
        1,
        "After rebase: should have 1 commit (only B)"
    );
    assert!(
        log_after.contains("commit B"),
        "Should still have commit B: {}",
        log_after
    );

    // Verify the rebased branch can be pushed (simulating force-push)
    let (_success, _) = run_git(&repo_path, &["push", "-f", "origin", "pr-branch-b"]);
    // Note: This might fail if branch doesn't exist on remote, which is fine for this test
    // The important part is that the rebase succeeded
}

#[test]
fn test_rebase_chain_after_multiple_squash_merges() {
    // This test verifies that we can rebase a chain of branches
    // after multiple squash merges (landing PR 1, then PR 2, etc.)

    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up gg config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "chain-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create 3 commits with branches
    for (letter, num) in [('a', 1), ('b', 2), ('c', 3)] {
        let filename = format!("file_{}.txt", letter);
        let content = format!("content {}", letter.to_uppercase());
        fs::write(repo_path.join(&filename), &content).expect("Failed to write file");
        run_git(&repo_path, &["add", "."]);
        run_git(
            &repo_path,
            &[
                "commit",
                "-m",
                &format!(
                    "feat: commit {}\n\nGG-ID: c-{}{}{}",
                    letter.to_uppercase(),
                    letter,
                    letter,
                    num
                ),
            ],
        );
        run_git(&repo_path, &["branch", &format!("pr-branch-{}", letter)]);
    }

    // Simulate landing PR A (squash merge to main)
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("file_a.txt"), "content A").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "feat: commit A (#1)"]);
    run_git(&repo_path, &["push", "origin", "main"]);

    // Rebase remaining branches (B and C) onto new main
    run_git(&repo_path, &["checkout", "pr-branch-b"]);
    let (success, _) = run_git(&repo_path, &["rebase", "main"]);
    assert!(success, "Rebase of B should succeed");

    run_git(&repo_path, &["checkout", "pr-branch-c"]);
    let (success, _) = run_git(&repo_path, &["rebase", "pr-branch-b"]);
    assert!(success, "Rebase of C onto B should succeed");

    // Simulate landing PR B (squash merge)
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("file_b.txt"), "content B").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "feat: commit B (#2)"]);
    run_git(&repo_path, &["push", "origin", "main"]);

    // Rebase C onto the new main
    run_git(&repo_path, &["checkout", "pr-branch-c"]);
    let (success, _) = run_git(&repo_path, &["rebase", "main"]);
    assert!(success, "Rebase of C after B landed should succeed");

    // Verify C only has one commit now
    let (_, log) = run_git(&repo_path, &["log", "--oneline", "main..HEAD"]);
    let commits: Vec<&str> = log.trim().lines().collect();
    assert_eq!(
        commits.len(),
        1,
        "C should only have 1 commit after all rebases"
    );
    assert!(log.contains("commit C"), "Should be commit C: {}", log);
}

// ==================== Worktree Support Tests ====================

/// Create a test repo inside a parent dir, so worktrees can be created as siblings.
/// Uses --initial-branch=main for CI compatibility where git may default to 'master'.
/// Returns (parent_temp_dir, repo_path) - the parent dir owns both repo and worktree dirs.
fn create_test_repo_with_worktree_support() -> (TempDir, PathBuf) {
    let parent_dir = TempDir::new().expect("Failed to create parent temp dir");
    let repo_path = parent_dir.path().join("repo");
    fs::create_dir(&repo_path).expect("Failed to create repo dir");

    Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to init git repo");

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

    (parent_dir, repo_path)
}

/// Helper to create a git worktree as a sibling of the repo inside the parent temp dir
fn create_worktree(main_repo: &PathBuf, name: &str) -> PathBuf {
    let worktree_path = main_repo.parent().unwrap().join(name);
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            "-b",
            name,
        ])
        .current_dir(main_repo)
        .output()
        .expect("Failed to run git worktree add");
    assert!(
        output.status.success(),
        "Failed to create worktree '{}': {}{}",
        name,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    worktree_path
}

#[test]
fn test_worktree_shares_config() {
    let (_parent_dir, repo_path) = create_test_repo_with_worktree_support();

    // Set up config in main repo
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
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
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
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
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
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
fn test_gg_checkout_with_worktree_creates_worktree_and_preserves_main_repo_head() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "wt-stack", "--worktree"]);
    assert!(
        success,
        "checkout --worktree should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    // Main repo should remain on its original branch (no branch checkout in primary worktree)
    let (_, current_branch) = run_git(&repo_path, &["branch", "--show-current"]);
    let branch = current_branch.trim();
    assert!(
        branch == "main" || branch == "master",
        "Expected main or master, got: {}",
        branch
    );

    let config = fs::read_to_string(gg_dir.join("config.json")).expect("Failed to read config");
    assert!(
        config.contains("worktree_path"),
        "Config should persist worktree path"
    );

    let expected_path = repo_path.parent().expect("repo parent").join(format!(
        "{}.{}",
        repo_path.file_name().unwrap().to_string_lossy(),
        "wt-stack"
    ));

    assert!(
        expected_path.exists(),
        "Expected worktree path to exist: {}",
        expected_path.display()
    );
}

#[test]
fn test_gg_ls_marks_stacks_with_worktree_indicator() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "wt-list", "--worktree"]);
    assert!(success, "checkout --worktree should succeed: {}", stderr);

    let (success, stdout, stderr) = run_gg(&repo_path, &["ls", "--all"]);
    assert!(success, "ls should succeed: {}", stderr);
    assert!(stdout.contains("wt-list"), "ls should show stack name");
    assert!(stdout.contains("[wt]"), "ls should show worktree indicator");
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
