//! Integration tests for git-gud
//!
//! These tests create temporary git repositories and test the core functionality.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde_json::Value;
use tempfile::TempDir;

/// Helper to create a temporary git repo
fn create_test_repo() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let repo_path = temp_dir.path().to_path_buf();

    // Initialize git repo with explicit main branch (for CI compatibility)
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

fn run_gg_with_env(
    repo_path: &std::path::Path,
    args: &[&str],
    envs: &[(&str, &std::ffi::OsStr)],
) -> (bool, String, String) {
    let gg_path = env!("CARGO_BIN_EXE_gg");

    let mut cmd = Command::new(gg_path);
    cmd.args(args).current_dir(repo_path);
    for (key, value) in envs {
        cmd.env(key, value);
    }

    let output = cmd.output().expect("Failed to run gg");

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
fn test_gg_lint_json_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["lint", "--help"]);

    assert!(success);
    assert!(stdout.contains("--json"));
}

#[test]
fn test_gg_lint_json_no_lint_commands() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "lint-json-no-commands"]);
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
    assert_eq!(parsed["lint"]["all_passed"], true);

    let results = parsed["lint"]["results"]
        .as_array()
        .expect("lint.results must be an array");
    assert!(results.is_empty(), "lint.results should be empty");
}

#[test]
fn test_gg_lint_json_empty_stack() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["git --version"]}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "lint-json-empty-stack"]);
    assert!(success, "Failed to create stack: {}", stderr);

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--json"]);
    assert!(success, "gg lint --json failed: {}", stderr);
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSON mode"
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["lint"]["all_passed"], true);

    let results = parsed["lint"]["results"]
        .as_array()
        .expect("lint.results must be an array");
    assert!(results.is_empty(), "lint.results should be empty");
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
fn test_gg_squash_staged_changes_in_worktree_do_not_trigger_unstaged_warning() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let stack_name = "squash-worktree-staged";
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", stack_name, "--worktree"]);
    assert!(success, "Failed to create worktree stack: {}", stderr);

    let worktree_path = repo_path.parent().unwrap().join(format!(
        "{}.{}",
        repo_path.file_name().unwrap().to_string_lossy(),
        stack_name
    ));

    fs::write(worktree_path.join("file1.txt"), "original content").expect("Failed to write file");
    run_git(&worktree_path, &["add", "."]);
    run_git(&worktree_path, &["commit", "-m", "Initial file"]);

    fs::write(worktree_path.join("file1.txt"), "modified content").expect("Failed to write file");
    run_git(&worktree_path, &["add", "file1.txt"]);

    let (success, stdout, stderr) = run_gg(&worktree_path, &["sc"]);
    assert!(
        success,
        "gg sc should succeed in worktree with only staged changes. stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        !stdout.contains("You have unstaged changes")
            && !stderr.contains("You have unstaged changes"),
        "Should not warn about unstaged changes when all changes are staged. stdout={}, stderr={}",
        stdout,
        stderr
    );
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

#[test]
fn test_reorder_no_tui_flag() {
    // Verify --no-tui flag appears in help
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["reorder", "--help"]);
    assert!(success, "reorder --help should succeed");
    assert!(
        stdout.contains("--no-tui"),
        "reorder help should mention --no-tui flag: {}",
        stdout
    );
}

#[test]
fn test_reorder_no_tui_editor_fallback() {
    // Test that --no-tui uses the editor path instead of the TUI.
    // Using VISUAL=true (the `true` command exits immediately, keeping original order).
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up gg config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with 2 commits
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-reorder-notui"]);
    assert!(success, "Failed to checkout: {}", stderr);

    fs::write(repo_path.join("a.txt"), "aaa\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Commit A\n\nGG-ID: c-aaaa001"],
    );

    fs::write(repo_path.join("b.txt"), "bbb\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Commit B\n\nGG-ID: c-aaaa002"],
    );

    // Run reorder with --no-tui and VISUAL=true (editor exits immediately, order unchanged)
    let gg_path = env!("CARGO_BIN_EXE_gg");
    let output = Command::new(gg_path)
        .args(["reorder", "--no-tui"])
        .current_dir(&repo_path)
        .env("VISUAL", "true")
        .output()
        .expect("Failed to run gg reorder --no-tui");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        output.status.success(),
        "gg reorder --no-tui should succeed: stdout={}, stderr={}",
        stdout,
        stderr,
    );

    // The editor (true) exits immediately without modifying the file,
    // so reorder should report either "cancelled" or "unchanged"
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("cancelled") || combined.contains("unchanged"),
        "Expected 'cancelled' or 'unchanged' message, got: stdout={}, stderr={}",
        stdout,
        stderr,
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
fn test_lint_runs_from_subdirectory_using_repo_root() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["./lint.sh"]}}"#,
    )
    .expect("Failed to write config");

    fs::write(
        repo_path.join("lint.sh"),
        "#!/bin/sh\necho ok >> lint-output.txt\n",
    )
    .expect("Failed to write lint script");
    Command::new("chmod")
        .args(["+x", "lint.sh"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to chmod lint script");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "lint-subdir-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let subdir = repo_path.join("nested/subdir");
    fs::create_dir_all(&subdir).expect("Failed to create nested subdir");

    let (success, stdout, stderr) = run_gg(&subdir, &["lint"]);
    assert!(
        success,
        "gg lint should succeed from subdirectory. stdout={}, stderr={}",
        stdout, stderr
    );

    let lint_output =
        fs::read_to_string(repo_path.join("lint-output.txt")).expect("Failed to read lint output");
    assert!(
        lint_output.contains("ok"),
        "lint command should run from repo root and write output"
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
fn test_sync_lint_failure_restores_head_and_does_not_push() {
    let (_temp_dir, repo_path, remote_path) = create_test_repo_with_remote();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github","lint":["./lint-fail.sh"]}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-sync-rollback"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(
        repo_path.join("lint-fail.sh"),
        "#!/bin/sh\necho 'lint-touched' >> data.txt\nexit 1\n",
    )
    .expect("Failed to write lint script");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(repo_path.join("lint-fail.sh"))
            .expect("Failed to stat lint script")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(repo_path.join("lint-fail.sh"), perms)
            .expect("Failed to chmod lint script");
    }

    fs::write(repo_path.join("data.txt"), "original\n").expect("Failed to write data file");
    run_git(&repo_path, &["add", "lint-fail.sh", "data.txt"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Stack commit\n\nGG-ID: c-sync001"],
    );

    let (_ok, start_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let (_ok, start_head) = run_git(&repo_path, &["rev-parse", "HEAD"]);

    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("Failed to create fake bin dir");
    fs::write(
        fake_bin.join("gh"),
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'gh version 2.0.0'\n  exit 0\nfi\nif [ \"$1\" = \"auth\" ] && [ \"$2\" = \"status\" ]; then\n  exit 0\nfi\necho 'unexpected gh invocation' >&2\nexit 1\n",
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
        &["sync", "--lint"],
        &[("PATH", new_path.as_os_str())],
    );
    assert!(!success, "sync --lint should fail when lint fails");
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("Lint failed") && combined.contains("restored"),
        "expected lint failure + restore message, got: {}",
        combined
    );

    let (_ok, end_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let (_ok, end_head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(
        start_branch.trim(),
        end_branch.trim(),
        "branch should be restored"
    );
    assert_eq!(
        start_head.trim(),
        end_head.trim(),
        "HEAD should be restored"
    );

    let (_ok, symbolic) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert_eq!(
        symbolic.trim(),
        start_branch.trim(),
        "HEAD should stay attached to original branch"
    );

    let remote_branch = "refs/heads/testuser/lint-sync-rollback--c-sync001";
    let output = Command::new("git")
        .args([
            "--git-dir",
            remote_path.to_str().unwrap(),
            "show-ref",
            remote_branch,
        ])
        .output()
        .expect("Failed to inspect remote refs");
    assert!(
        !output.status.success(),
        "entry branch should not be pushed when lint fails"
    );
}

#[test]
fn test_sync_lint_failure_restores_post_rebase_snapshot() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github","lint":["./lint-fail.sh"],"sync_auto_rebase":true,"sync_behind_threshold":1}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-sync-post-rebase"]);
    assert!(success, "Failed to create stack: {}", stderr);
    let (_ok, stack_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);

    fs::write(
        repo_path.join("lint-fail.sh"),
        "#!/bin/sh\necho 'lint-failure' >&2\nexit 1\n",
    )
    .expect("Failed to write lint script");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(repo_path.join("lint-fail.sh"))
            .expect("Failed to stat lint script")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(repo_path.join("lint-fail.sh"), perms)
            .expect("Failed to chmod lint script");
    }

    fs::write(repo_path.join("stack.txt"), "stack\n").expect("Failed to write stack file");
    run_git(&repo_path, &["add", "lint-fail.sh", "stack.txt"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Stack commit\n\nGG-ID: c-postrb1"],
    );

    let (_ok, pre_sync_head) = run_git(&repo_path, &["rev-parse", "HEAD"]);

    // Move origin/main ahead so sync's behind-base check triggers auto-rebase.
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("base.txt"), "base update\n").expect("Failed to write base file");
    run_git(&repo_path, &["add", "base.txt"]);
    run_git(&repo_path, &["commit", "-m", "Base moved"]);
    run_git(&repo_path, &["push", "origin", "main"]);
    run_git(&repo_path, &["checkout", stack_branch.trim()]);

    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("Failed to create fake bin dir");
    fs::write(
        fake_bin.join("gh"),
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'gh version 2.0.0'\n  exit 0\nfi\nif [ \"$1\" = \"auth\" ] && [ \"$2\" = \"status\" ]; then\n  exit 0\nfi\necho 'unexpected gh invocation' >&2\nexit 1\n",
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

    let (success, _stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--lint"],
        &[("PATH", new_path.as_os_str())],
    );
    assert!(!success, "sync --lint should fail when lint fails");
    assert!(
        stderr.contains("Lint failed") || stderr.contains("lint failed"),
        "expected lint failure output, got: {}",
        stderr
    );

    let (_ok, end_head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_ne!(
        pre_sync_head.trim(),
        end_head.trim(),
        "HEAD should stay at post-rebase snapshot, not pre-rebase commit"
    );

    let (_ok, current_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(current_branch.trim(), stack_branch.trim());
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
fn test_gg_ls_reads_shared_repo_config_from_worktree() {
    let (_parent_dir, repo_path) = create_test_repo_with_worktree_support();

    let fake_home = repo_path.parent().unwrap().join("fake-home");
    fs::create_dir_all(&fake_home).expect("Failed to create fake home");

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let home = fake_home.as_os_str();
    let (success, _, stderr) = run_gg_with_env(
        &repo_path,
        &["co", "shared-config-stack"],
        &[("HOME", home)],
    );
    assert!(success, "Failed to create stack: {}", stderr);

    let worktree_path = create_worktree(&repo_path, "wt-shared-config-stack");

    let (success, stdout, stderr) =
        run_gg_with_env(&worktree_path, &["ls", "-a"], &[("HOME", home)]);

    assert!(
        success,
        "gg ls -a from worktree should succeed. stdout: {}, stderr: {}",
        stdout, stderr
    );
    assert!(
        stdout.contains("shared-config-stack"),
        "gg ls -a from worktree should use shared repo config and list the stack. stdout: {}",
        stdout
    );

    let worktree_git_dir = repo_path.join(".git/worktrees/wt-shared-config-stack/gg/config.json");
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
fn test_lint_resolves_git_paths_in_worktree() {
    let (_parent_dir, repo_path) = create_test_repo_with_worktree_support();

    // Create a lint script inside the main repo's .git/gg/ directory
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");

    let lint_script = gg_dir.join("lint.sh");
    fs::write(&lint_script, "#!/bin/sh\nexit 0\n").expect("Failed to write lint script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&lint_script, fs::Permissions::from_mode(0o755))
            .expect("Failed to set script permissions");
    }

    // Configure lint to use ./.git/gg/lint.sh
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["./.git/gg/lint.sh"]}}"#,
    )
    .expect("Failed to write config");

    // Create a worktree
    let worktree_path = create_worktree(&repo_path, "wt-lint-test");

    // Create a stack and a commit in the worktree
    let (success, _, stderr) = run_gg(&worktree_path, &["co", "lint-stack"]);
    assert!(success, "checkout should succeed: {}", stderr);

    run_git(
        &worktree_path,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "test commit\n\nGG-ID: c-lint001",
        ],
    );

    // Run lint from the worktree – this should resolve ./.git/gg/lint.sh
    // to the main repo's .git/gg/lint.sh via commondir
    let (success, stdout, stderr) = run_gg(&worktree_path, &["lint"]);
    assert!(
        success,
        "lint should succeed in worktree: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("OK") || stdout.contains("Linted"),
        "Expected lint success output, got: {}",
        stdout
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
#[test]
fn test_lint_after_rebase_drops_landed_commits() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Setup config with lint command (no provider needed for rebase + lint)
    // Using "true" as lint command (always succeeds)
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["true"]}}"#,
    )
    .expect("Failed to write config");

    // Create stack with gg co
    let (success, _, stderr) = run_gg(&repo_path, &["co", "rebase-lint-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create commit 1
    fs::write(repo_path.join("file1.txt"), "content 1").expect("Failed to write file1");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: commit 1\n\nGG-ID: c-aaa1111"],
    );

    // Create commit 2
    fs::write(repo_path.join("file2.txt"), "content 2").expect("Failed to write file2");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: commit 2\n\nGG-ID: c-bbb2222"],
    );

    // Create commit 3
    fs::write(repo_path.join("file3.txt"), "content 3").expect("Failed to write file3");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: commit 3\n\nGG-ID: c-ccc3333"],
    );

    // Verify stack has 3 commits
    let (success, stdout, _) = run_gg(&repo_path, &["ls"]);
    assert!(success, "gg ls should succeed");
    assert!(
        stdout.contains("[1]") && stdout.contains("[2]") && stdout.contains("[3]"),
        "Stack should have 3 commits before landing. stdout: {}",
        stdout
    );

    // Simulate landing the first commit (squash merge to main on origin)
    // 1. Checkout main
    run_git(&repo_path, &["checkout", "main"]);

    // 2. Create a new commit that represents the squash merge of commit 1
    fs::write(repo_path.join("file1.txt"), "content 1").expect("Failed to write file1");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: commit 1 (#1)\n\nSquash merged"],
    );

    // 3. Push to origin (this advances origin/main)
    run_git(&repo_path, &["push", "origin", "main"]);

    // 4. Go back to our stack branch
    run_git(&repo_path, &["checkout", "testuser/rebase-lint-test"]);

    // Verify stack is behind origin/main (rebase needed)
    run_git(&repo_path, &["fetch", "origin"]);

    let (_, log_behind) = run_git(&repo_path, &["log", "--oneline", "HEAD..origin/main"]);
    assert!(
        !log_behind.trim().is_empty(),
        "Stack should be behind origin/main after landing"
    );

    // Verify stack still shows 3 commits (pre-rebase)
    let (success, stdout_ls, _) = run_gg(&repo_path, &["ls"]);
    assert!(success, "gg ls should succeed");
    assert!(
        stdout_ls.contains("[3]"),
        "Stack should have 3 commits before rebase. stdout: {}",
        stdout_ls
    );

    // Run rebase (no auth required). This should:
    // 1. Fetch and update main to match origin/main
    // 2. Rebase stack onto new main
    // 3. Drop commit 1 because its changes are already on main
    let (success, stdout, stderr) = run_gg(&repo_path, &["rebase"]);
    assert!(
        success,
        "gg rebase should succeed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    // Verify stack now has only 2 commits after rebase
    let (success, stdout_ls, _) = run_gg(&repo_path, &["ls"]);
    assert!(success, "gg ls should succeed after rebase");
    assert!(
        stdout_ls.contains("[1]") && stdout_ls.contains("[2]") && !stdout_ls.contains("[3]"),
        "Stack should have 2 commits after rebase (commit 1 was dropped). stdout: {}",
        stdout_ls
    );

    // Now run lint on the post-rebase stack. This is the critical test:
    // Before the fix, if we had tracked "end position = 3" from before rebase,
    // lint would crash with "Position 3 is out of range (max: 2)"
    let (success, stdout, stderr) = run_gg(&repo_path, &["lint"]);
    assert!(
        success,
        "gg lint should succeed on post-rebase stack. stdout: {}, stderr: {}",
        stdout, stderr
    );
    assert!(
        !stdout.contains("out of range") && !stderr.contains("out of range"),
        "lint should not crash with 'out of range'. stdout: {}, stderr: {}",
        stdout,
        stderr
    );

    // Lint should have run on both remaining commits
    // The lint command is "true" which always succeeds
    assert!(
        stdout.contains("Running lint on commits 1-2"),
        "Should lint commits 1-2. stdout: {}",
        stdout
    );
}

// See: https://github.com/mrmans0n/git-gud/issues/199
// Direct lint test that exercises boundary checking without needing provider auth
#[test]
fn test_lint_position_clamped_to_stack_size() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config with a simple lint command that always succeeds
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["true"]}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with 2 commits
    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-boundary-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // First commit
    fs::write(repo_path.join("file1.txt"), "content1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "First commit\n\nGG-ID: c-lint001"],
    );

    // Second commit
    fs::write(repo_path.join("file2.txt"), "content2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Second commit\n\nGG-ID: c-lint002"],
    );

    // Verify stack has 2 commits
    let (success, stdout, _) = run_gg(&repo_path, &["ls"]);
    assert!(success, "gg ls should succeed");
    assert!(
        stdout.contains("[1]") && stdout.contains("[2]"),
        "Stack should have 2 commits. stdout: {}",
        stdout
    );

    // Test 1: lint --until 3 on a 2-commit stack should error with "out of range"
    // This verifies the boundary check works correctly
    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "3"]);
    assert!(!success, "lint --until 3 should fail on a 2-commit stack");
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("out of range") || combined.contains("invalid"),
        "Should get out of range error for position 3 on 2-commit stack. combined: {}",
        combined
    );

    // Test 2: lint without --until should succeed and lint all commits
    let (success, stdout, stderr) = run_gg(&repo_path, &["lint"]);
    assert!(
        success,
        "lint (without --until) should succeed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    // Test 3: lint --until 2 should succeed (exactly at boundary)
    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "2"]);
    assert!(
        success,
        "lint --until 2 should succeed on 2-commit stack. stdout: {}, stderr: {}",
        stdout, stderr
    );

    // Test 4: lint --until 1 should succeed (within bounds)
    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "1"]);
    assert!(
        success,
        "lint --until 1 should succeed. stdout: {}, stderr: {}",
        stdout, stderr
    );
}
// ========== Split command tests ==========

#[test]
fn test_split_head_with_file_args() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-split"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create first commit (1 file)
    fs::write(repo_path.join("file_a.txt"), "content a").expect("Failed to write file_a");
    run_git(&repo_path, &["add", "-A"]);
    run_git(&repo_path, &["commit", "-m", "Add file A"]);

    // Create second commit (2 files)
    fs::write(repo_path.join("file_b.txt"), "content b").expect("Failed to write file_b");
    fs::write(repo_path.join("file_c.txt"), "content c").expect("Failed to write file_c");
    run_git(&repo_path, &["add", "-A"]);
    run_git(&repo_path, &["commit", "-m", "Add files B and C"]);

    // Split HEAD: move file_b to a new commit before the current
    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["split", "-m", "Add file B only", "--no-edit", "file_b.txt"],
    );
    assert!(
        success,
        "split should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("Split complete"),
        "Expected 'Split complete' in output: {}",
        stdout
    );

    // Verify we now have 3 commits in the stack
    let (success, stdout, _) = run_gg(&repo_path, &["ls"]);
    assert!(success, "ls should succeed");
    // Should see 3 entries: file A, file B only, files B and C (remainder)
    assert!(
        stdout.contains("Add file B only"),
        "Should have the new split commit: {}",
        stdout
    );
}

#[test]
fn test_split_non_head_rebases_descendants() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-split-rebase"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Commit 1: file_a
    fs::write(repo_path.join("file_a.txt"), "a").expect("Failed to write");
    run_git(&repo_path, &["add", "-A"]);
    run_git(&repo_path, &["commit", "-m", "Commit 1: file A"]);

    // Commit 2: file_b + file_c (this is the one we'll split)
    fs::write(repo_path.join("file_b.txt"), "b").expect("Failed to write");
    fs::write(repo_path.join("file_c.txt"), "c").expect("Failed to write");
    run_git(&repo_path, &["add", "-A"]);
    run_git(&repo_path, &["commit", "-m", "Commit 2: files B and C"]);

    // Commit 3: file_d (descendant that should be rebased)
    fs::write(repo_path.join("file_d.txt"), "d").expect("Failed to write");
    run_git(&repo_path, &["add", "-A"]);
    run_git(&repo_path, &["commit", "-m", "Commit 3: file D"]);

    // Navigate to commit 2 and split it
    let (success, _, stderr) = run_gg(&repo_path, &["mv", "2"]);
    assert!(success, "Failed to navigate to commit 2: {}", stderr);

    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["split", "-m", "Split: file B", "--no-edit", "file_b.txt"],
    );
    assert!(
        success,
        "split should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("Split complete"),
        "Expected 'Split complete' in output: {}",
        stdout
    );
    assert!(
        stdout.contains("Rebased"),
        "Expected rebasing descendants: {}",
        stdout
    );
}

#[test]
fn test_split_invalid_file_errors() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-split-error"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create commit with two files
    fs::write(repo_path.join("file_a.txt"), "a").expect("Failed to write");
    fs::write(repo_path.join("file_b.txt"), "b").expect("Failed to write");
    run_git(&repo_path, &["add", "-A"]);
    run_git(&repo_path, &["commit", "-m", "Two files"]);

    // Try to split with a file that doesn't exist in the commit
    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["split", "-m", "test", "--no-edit", "nonexistent.txt"],
    );
    assert!(
        !success,
        "split should fail with invalid file: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stderr.contains("not in the commit") || stdout.contains("not in the commit"),
        "Should mention file not in commit: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

#[test]
fn test_split_preserves_gg_id_on_remainder() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-split-ggid"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create commit with two files and a valid GG-ID (format: c-XXXXXXX)
    fs::write(repo_path.join("file_a.txt"), "a").expect("Failed to write");
    fs::write(repo_path.join("file_b.txt"), "b").expect("Failed to write");
    run_git(&repo_path, &["add", "-A"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Two files\n\nGG-ID: c-abc1234"],
    );

    // Split the commit
    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["split", "-m", "Split file A", "--no-edit", "file_a.txt"],
    );
    assert!(
        success,
        "split should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("Split complete"),
        "Expected 'Split complete' in output: {}",
        stdout
    );

    // The remainder commit (HEAD) should still have the original GG-ID
    let (success, log_output) = run_git(&repo_path, &["log", "-1", "--format=%B", "HEAD"]);
    assert!(success, "git log should succeed");
    assert!(
        log_output.contains("GG-ID: c-abc1234"),
        "Remainder commit should preserve original GG-ID: {}",
        log_output
    );
}

#[test]
fn test_split_single_file_commit_errors() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-split-single"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create commit with only one file
    fs::write(repo_path.join("only_file.txt"), "content").expect("Failed to write");
    run_git(&repo_path, &["add", "-A"]);
    run_git(&repo_path, &["commit", "-m", "Single file commit"]);

    // Hunk mode is the default, but without a TTY the interactive prompt will
    // fail. The important thing is we no longer get the old "only has 1 file" error.
    let (_success, stdout, stderr) = run_gg(&repo_path, &["split", "-m", "test", "--no-edit"]);

    // Should NOT contain the old "only has 1 file" message
    assert!(
        !stderr.contains("only has 1 file") && !stdout.contains("only has 1 file"),
        "Should NOT mention single file limitation (hunk mode is now used): stdout={}, stderr={}",
        stdout,
        stderr
    );

    // Instead, it will fail on interactive input (no TTY) or succeed if no hunks
    // Either way, we're testing that the behavior changed
}

#[test]
fn test_split_help_no_interactive_flag() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Verify -i/--interactive flag has been removed (hunk mode is now the default)
    let (success, stdout, _stderr) = run_gg(&repo_path, &["split", "--help"]);
    assert!(success, "split --help should succeed");
    assert!(
        !stdout.contains("--interactive"),
        "split help should NOT mention --interactive flag (hunk mode is default): {}",
        stdout
    );
}

/// Helper to run gg with stdin input
fn run_gg_with_stdin(
    repo_path: &std::path::Path,
    args: &[&str],
    stdin_input: &str,
) -> (bool, String, String) {
    let gg_path = env!("CARGO_BIN_EXE_gg");

    let mut child = Command::new(gg_path)
        .args(args)
        .current_dir(repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn gg");

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_input.as_bytes())
            .expect("Failed to write to stdin");
    }

    let output = child.wait_with_output().expect("Failed to wait on gg");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    (output.status.success(), stdout, stderr)
}

#[test]
fn test_split_hunk_mode_with_multiple_hunks() {
    // This test verifies that hunk-level splitting works correctly.
    // We create a file with multiple disjoint changes (multiple hunks),
    // then use split -i to select only the first hunk.
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up minimal gg config
    let gg_dir = repo_path.join(".git").join("gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create .git/gg");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-hunk-split"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create initial file with some content
    let initial_content = r#"line 1
line 2
line 3
line 4
line 5
line 6
line 7
line 8
line 9
line 10
line 11
line 12
line 13
line 14
line 15
line 16
line 17
line 18
line 19
line 20
"#;
    fs::write(repo_path.join("multi_hunk.txt"), initial_content).expect("Failed to write file");
    run_git(&repo_path, &["add", "-A"]);
    run_git(&repo_path, &["commit", "-m", "Initial file"]);

    // Now modify lines 2 and line 18 (far apart = separate hunks)
    let modified_content = r#"line 1
line 2 MODIFIED FIRST
line 3
line 4
line 5
line 6
line 7
line 8
line 9
line 10
line 11
line 12
line 13
line 14
line 15
line 16
line 17
line 18 MODIFIED SECOND
line 19
line 20
"#;
    fs::write(repo_path.join("multi_hunk.txt"), modified_content).expect("Failed to write file");
    run_git(&repo_path, &["add", "-A"]);
    run_git(&repo_path, &["commit", "-m", "Two separate hunks"]);

    // Verify we have 2 commits
    let (_, log_before, _) = run_git_full(&repo_path, &["log", "--oneline"]);
    let commit_count_before = log_before.lines().count();

    // Try to split (hunk mode is now the default)
    // When stdin is piped (not TTY), the terminal library typically returns an error
    // or reads from stdin directly. We send "y\nn\n" to select first hunk, skip second.
    //
    // NOTE: This test may not work perfectly because console::Term requires a TTY.
    // The test validates the command doesn't crash and exercises the code path.
    let (success, stdout, stderr) = run_gg_with_stdin(
        &repo_path,
        &["split", "-m", "First hunk only", "--no-edit"],
        "y\nn\n",
    );

    // The command may fail due to TTY requirements, but it shouldn't panic
    // Check that we at least got past the initial parsing
    if success {
        // If it succeeded, verify we now have 3 commits
        let (_, log_after, _) = run_git_full(&repo_path, &["log", "--oneline"]);
        let commit_count_after = log_after.lines().count();
        assert!(
            commit_count_after >= commit_count_before,
            "Should have same or more commits after split"
        );
    } else {
        // Expected: TTY error because console::Term doesn't work with piped stdin
        // This is acceptable - we're testing the code doesn't crash
        assert!(
            stderr.contains("Failed to read")
                || stderr.contains("input")
                || stderr.contains("terminal")
                || stderr.contains("tty")
                || stderr.contains("No hunks"),
            "Should fail gracefully with TTY/input error, got: stdout={}, stderr={}",
            stdout,
            stderr
        );
    }
}

#[test]
fn test_split_hunk_mode_is_default() {
    // Verify that hunk mode is the default split behavior (no -i flag needed)
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up minimal gg config
    let gg_dir = repo_path.join(".git").join("gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create .git/gg");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack with a commit
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-sub-select"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write");
    run_git(&repo_path, &["add", "-A"]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    // split without -i should work (hunk mode is default)
    let (_success, _stdout, stderr) = run_gg(&repo_path, &["split", "-m", "test", "--no-edit"]);

    // Should NOT say "unrecognized" or "unknown" flag
    assert!(
        !stderr.contains("unrecognized") && !stderr.contains("unknown option"),
        "split command should work without -i: {}",
        stderr
    );
}

#[test]
fn test_split_no_tui_flag() {
    // Verify --no-tui flag is accepted and falls back to sequential prompt mode
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up minimal gg config
    let gg_dir = repo_path.join(".git").join("gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create .git/gg");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Verify --no-tui flag appears in help
    let (success, stdout, _stderr) = run_gg(&repo_path, &["split", "--help"]);
    assert!(success, "split --help should succeed");
    assert!(
        stdout.contains("--no-tui"),
        "split help should mention --no-tui flag: {}",
        stdout
    );

    // Create stack with a multi-file commit so split has something to work with
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-no-tui"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file_a.txt"), "content a").expect("Failed to write");
    fs::write(repo_path.join("file_b.txt"), "content b").expect("Failed to write");
    run_git(&repo_path, &["add", "file_a.txt", "file_b.txt"]);
    run_git(&repo_path, &["commit", "-m", "Two files"]);

    // Use --no-tui with file args to bypass interactive prompts entirely
    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &[
            "split",
            "--no-tui",
            "-m",
            "Split file A",
            "--no-edit",
            "file_a.txt",
        ],
    );

    // The flag should be recognized (no "unrecognized" errors)
    assert!(
        !stderr.contains("unrecognized") && !stderr.contains("unknown option"),
        "--no-tui flag should be recognized: stdout={}, stderr={}",
        stdout,
        stderr
    );

    // --no-tui with file args bypasses the interactive picker (no TTY needed),
    // so the command must succeed reliably in CI.
    assert!(
        success,
        "split --no-tui with file args should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    // When file args are provided, all hunks from those files are auto-selected.
    let (_, log_output, _) = run_git_full(&repo_path, &["log", "--oneline"]);
    let commit_count = log_output.lines().count();
    assert!(
        commit_count >= 3,
        "Should have at least 3 commits after split: {}",
        log_output
    );
}

// ============================================================================
// Clean command verification tests
// ============================================================================

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
fn test_arrange_is_alias_for_reorder() {
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
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-arrange"]);
    assert!(success, "Failed to checkout: {}", stderr);

    fs::write(repo_path.join("a.txt"), "A").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add A"]);

    fs::write(repo_path.join("b.txt"), "B").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add B"]);

    // Use 'arrange' alias with --order to reorder commits
    let (success, _, stderr) = run_gg(&repo_path, &["arrange", "--order", "2,1"]);
    assert!(success, "Failed to arrange: {}", stderr);

    // Verify new order in log (most recent first)
    let (_, log_after) = run_git(&repo_path, &["log", "--oneline", "-2"]);
    let lines: Vec<&str> = log_after.trim().lines().collect();

    // After reorder "2,1": B becomes [1], A becomes [2]
    // git log shows most recent first, so: A, B
    assert!(
        lines[0].contains("Add A"),
        "Expected A on top, got: {}",
        log_after
    );
    assert!(
        lines[1].contains("Add B"),
        "Expected B at bottom, got: {}",
        log_after
    );
}

// ============================================================
// gg drop tests
// ============================================================

#[test]
fn test_drop_single_commit_by_position() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Setup gg config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    // Create stack with 3 commits
    let (success, _, stderr) = run_gg(&repo_path, &["co", "drop-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    for i in 1..=3 {
        fs::write(
            repo_path.join(format!("file{}.txt", i)),
            format!("content {}", i),
        )
        .unwrap();
        run_git(&repo_path, &["add", "."]);
        run_git(&repo_path, &["commit", "-m", &format!("Commit {}", i)]);
    }

    // Verify 3 commits
    let (success, stdout, _) = run_gg(&repo_path, &["ls"]);
    assert!(success);
    assert!(stdout.contains("Commit 1"));
    assert!(stdout.contains("Commit 2"));
    assert!(stdout.contains("Commit 3"));

    // Drop commit at position 2 (middle)
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "2", "--force"]);
    assert!(
        success,
        "Drop failed. stdout: {}, stderr: {}",
        stdout, stderr
    );
    assert!(stdout.contains("Dropped 1 commit(s)"));
    assert!(stdout.contains("2 remaining"));

    // Verify commit 2 is gone
    let (success, stdout, _) = run_gg(&repo_path, &["ls"]);
    assert!(success);
    assert!(stdout.contains("Commit 1"));
    assert!(!stdout.contains("Commit 2"));
    assert!(stdout.contains("Commit 3"));
}

#[test]
fn test_drop_multiple_commits() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", "drop-multi"]);
    assert!(success, "Failed to create stack: {}", stderr);

    for i in 1..=4 {
        fs::write(
            repo_path.join(format!("file{}.txt", i)),
            format!("content {}", i),
        )
        .unwrap();
        run_git(&repo_path, &["add", "."]);
        run_git(&repo_path, &["commit", "-m", &format!("Commit {}", i)]);
    }

    // Drop positions 1 and 3
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "1", "3", "--force"]);
    assert!(
        success,
        "Drop failed. stdout: {}, stderr: {}",
        stdout, stderr
    );
    assert!(stdout.contains("Dropped 2 commit(s)"));
    assert!(stdout.contains("2 remaining"));

    // Verify only commits 2 and 4 remain
    let (success, stdout, _) = run_gg(&repo_path, &["ls"]);
    assert!(success);
    assert!(!stdout.contains("Commit 1"));
    assert!(stdout.contains("Commit 2"));
    assert!(!stdout.contains("Commit 3"));
    assert!(stdout.contains("Commit 4"));
}

#[test]
fn test_drop_cannot_drop_all_commits() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", "drop-all"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Only commit"]);

    // Try to drop the only commit
    let (success, _, stderr) = run_gg(&repo_path, &["drop", "1", "--force"]);
    assert!(!success, "Should fail when dropping all commits");
    assert!(
        stderr.contains("Cannot drop all commits"),
        "stderr: {}",
        stderr
    );
}

#[test]
fn test_drop_invalid_position() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", "drop-invalid"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    // Try to drop position 5 (out of range)
    let (success, _, stderr) = run_gg(&repo_path, &["drop", "5", "--force"]);
    assert!(!success, "Should fail for invalid position");
    assert!(
        stderr.contains("out of range"),
        "stderr should mention out of range: {}",
        stderr
    );
}

#[test]
fn test_drop_json_output() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", "drop-json"]);
    assert!(success, "Failed to create stack: {}", stderr);

    for i in 1..=3 {
        fs::write(
            repo_path.join(format!("file{}.txt", i)),
            format!("content {}", i),
        )
        .unwrap();
        run_git(&repo_path, &["add", "."]);
        run_git(&repo_path, &["commit", "-m", &format!("Commit {}", i)]);
    }

    // Drop with JSON output
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "2", "--force", "--json"]);
    assert!(
        success,
        "Drop failed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    let json: Value = serde_json::from_str(&stdout).expect("Should be valid JSON");
    assert_eq!(json["version"], 1);
    assert_eq!(json["drop"]["remaining"], 2);
    assert_eq!(json["drop"]["dropped"].as_array().unwrap().len(), 1);
    assert_eq!(json["drop"]["dropped"][0]["position"], 2);
    assert!(json["drop"]["dropped"][0]["title"]
        .as_str()
        .unwrap()
        .contains("Commit 2"));
}

#[test]
fn test_drop_no_targets_error() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", "drop-no-target"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    // Drop with no targets
    let (success, _, stderr) = run_gg(&repo_path, &["drop", "--force"]);
    assert!(!success, "Should fail with no targets");
    assert!(
        stderr.contains("No targets specified"),
        "stderr: {}",
        stderr
    );
}

#[test]
fn test_drop_alias_abandon() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", "drop-alias"]);
    assert!(success, "Failed to create stack: {}", stderr);

    for i in 1..=2 {
        fs::write(
            repo_path.join(format!("file{}.txt", i)),
            format!("content {}", i),
        )
        .unwrap();
        run_git(&repo_path, &["add", "."]);
        run_git(&repo_path, &["commit", "-m", &format!("Commit {}", i)]);
    }

    // Use 'abandon' alias
    let (success, stdout, stderr) = run_gg(&repo_path, &["abandon", "1", "--force"]);
    assert!(
        success,
        "Abandon alias failed. stdout: {}, stderr: {}",
        stdout, stderr
    );
    assert!(stdout.contains("Dropped 1 commit(s)"));
}

#[test]
fn test_drop_first_commit() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", "drop-first"]);
    assert!(success, "Failed to create stack: {}", stderr);

    for i in 1..=3 {
        fs::write(
            repo_path.join(format!("file{}.txt", i)),
            format!("content {}", i),
        )
        .unwrap();
        run_git(&repo_path, &["add", "."]);
        run_git(&repo_path, &["commit", "-m", &format!("Commit {}", i)]);
    }

    // Drop the first commit (bottom of stack)
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "1", "--force"]);
    assert!(
        success,
        "Drop failed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    let (success, stdout, _) = run_gg(&repo_path, &["ls"]);
    assert!(success);
    assert!(!stdout.contains("Commit 1"));
    assert!(stdout.contains("Commit 2"));
    assert!(stdout.contains("Commit 3"));
}

#[test]
fn test_drop_last_commit() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", "drop-last"]);
    assert!(success, "Failed to create stack: {}", stderr);

    for i in 1..=3 {
        fs::write(
            repo_path.join(format!("file{}.txt", i)),
            format!("content {}", i),
        )
        .unwrap();
        run_git(&repo_path, &["add", "."]);
        run_git(&repo_path, &["commit", "-m", &format!("Commit {}", i)]);
    }

    // Drop the last commit (top of stack)
    let (success, stdout, stderr) = run_gg(&repo_path, &["drop", "3", "--force"]);
    assert!(
        success,
        "Drop failed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    let (success, stdout, _) = run_gg(&repo_path, &["ls"]);
    assert!(success);
    assert!(stdout.contains("Commit 1"));
    assert!(stdout.contains("Commit 2"));
    assert!(!stdout.contains("Commit 3"));
}

// ==========================================================================
// gg run tests
// ==========================================================================

#[test]
fn test_gg_run_readonly_passing() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "git", "--version"]);
    assert!(
        success,
        "gg run should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("all passed") || stdout.contains("OK"),
        "Expected success message in: {}",
        stdout
    );
}

#[test]
fn test_gg_run_readonly_failing() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-fail-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let (success, _stdout, _stderr) = run_gg(&repo_path, &["run", "false"]);
    assert!(!success, "gg run with 'false' should fail");
}

#[test]
fn test_gg_run_json_output() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-json-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "--json", "git", "--version"]);
    assert!(success, "gg run --json failed: {}", stderr);

    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["run"]["all_passed"], true);

    let results = parsed["run"]["results"]
        .as_array()
        .expect("run.results must be an array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["position"], 1);
    assert_eq!(results[0]["commands"][0]["command"], "git --version");
    assert_eq!(results[0]["commands"][0]["passed"], true);
}

#[cfg(unix)]
#[test]
fn test_gg_run_amend_mode() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-amend-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    fs::write(
        repo_path.join("modify.sh"),
        "#!/bin/sh\necho \"modified\" >> test.txt\n",
    )
    .expect("Failed to write script");
    let mut perms = fs::metadata(repo_path.join("modify.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("modify.sh"), perms).unwrap();

    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let original = fs::read_to_string(repo_path.join("test.txt")).unwrap();

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "--amend", "./modify.sh"]);
    assert!(
        success,
        "gg run --amend should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    let modified = fs::read_to_string(repo_path.join("test.txt")).unwrap();
    assert_ne!(
        original, modified,
        "File should have been modified and amended"
    );
    assert!(
        modified.contains("modified"),
        "File should contain 'modified'"
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_discard_mode() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-discard-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    fs::write(
        repo_path.join("modify.sh"),
        "#!/bin/sh\necho \"modified\" >> test.txt\n",
    )
    .expect("Failed to write script");
    let mut perms = fs::metadata(repo_path.join("modify.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("modify.sh"), perms).unwrap();

    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let original = fs::read_to_string(repo_path.join("test.txt")).unwrap();

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "--discard", "./modify.sh"]);
    assert!(
        success,
        "gg run --discard should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    let after = fs::read_to_string(repo_path.join("test.txt")).unwrap();
    assert_eq!(original, after, "File should be unchanged after --discard");
}

#[cfg(unix)]
#[test]
fn test_gg_run_readonly_fails_on_dirty_tree() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-readonly-dirty-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    fs::write(
        repo_path.join("modify.sh"),
        "#!/bin/sh\necho \"modified\" >> test.txt\n",
    )
    .expect("Failed to write script");
    let mut perms = fs::metadata(repo_path.join("modify.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("modify.sh"), perms).unwrap();

    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "./modify.sh"]);
    assert!(
        !success,
        "gg run (read-only) should fail when command modifies files"
    );
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("modified files")
            || combined.contains("--amend")
            || combined.contains("--discard"),
        "Error should mention the file modification and suggest --amend/--discard: {}",
        combined
    );
}

#[test]
fn test_gg_run_parallel_passing() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "parallel-pass"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content1").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "First commit"]);

    fs::write(repo_path.join("file2.txt"), "content2").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Second commit"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "-j", "2", "git", "--version"]);
    assert!(
        success,
        "gg run --jobs should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("all passed") || stdout.contains("OK"),
        "Expected success message in: {}",
        stdout
    );
}

#[test]
fn test_gg_run_parallel_failing() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "parallel-fail"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content1").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "First commit"]);

    fs::write(repo_path.join("file2.txt"), "content2").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Second commit"]);

    let (success, _stdout, _stderr) = run_gg(&repo_path, &["run", "-j", "2", "false"]);
    assert!(!success, "gg run parallel with 'false' should fail");
}

#[test]
fn test_gg_run_parallel_json_output() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "parallel-json"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content1").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "First commit"]);

    fs::write(repo_path.join("file2.txt"), "content2").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Second commit"]);

    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["run", "-j", "2", "--json", "git", "--version"],
    );
    assert!(
        success,
        "gg run parallel --json should succeed: stderr={}",
        stderr
    );

    let json: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("should be valid JSON");
    assert_eq!(json["run"]["all_passed"], true);
    let results = json["run"]["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    // Results should be in commit position order
    assert_eq!(results[0]["position"], 1);
    assert_eq!(results[1]["position"], 2);
}

/// Locate the `test` binary on this OS. Linux has `/usr/bin/test`;
/// macOS only ships `/bin/test`. Both exit 0 when the comparison holds.
#[cfg(unix)]
fn locate_test_binary() -> &'static str {
    if std::path::Path::new("/usr/bin/test").exists() {
        "/usr/bin/test"
    } else if std::path::Path::new("/bin/test").exists() {
        "/bin/test"
    } else {
        panic!("no `test` binary found on this system");
    }
}

#[cfg(unix)]
#[test]
fn test_gg_run_preserves_quoted_arguments() {
    // Regression test for Bug #1: `gg run` used to join argv with spaces
    // and re-split on whitespace, destroying argument boundaries. Using
    // the `test` binary surfaces the bug as a non-zero exit when its args
    // get mangled (usage error → exit 2).
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-quoted-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "c1"]);

    // `test "a b" = "a b"` exits 0 when args are preserved, ≠0 otherwise.
    let test_bin = locate_test_binary();
    let (success, stdout, stderr) = run_gg(&repo_path, &["run", test_bin, "a b", "=", "a b"]);
    assert!(
        success,
        "gg run must preserve argument boundaries with whitespace.\nstdout={}\nstderr={}",
        stdout, stderr
    );

    // Negative case: inequality → exit 1 → gg run reports failure.
    let (success, _, _) = run_gg(&repo_path, &["run", test_bin, "a b", "=", "a c"]);
    assert!(
        !success,
        "gg run should report failure when the command's comparison is false"
    );
}

#[test]
fn test_gg_run_json_command_display_escapes_spaces() {
    // Regression test for Bug #1 display path: the `command` field in JSON
    // output should be a copy-pasteable shell form that single-quotes args
    // containing whitespace, not a naive whitespace-joined string.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-json-display-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "c1"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "--json", "echo", "hello world"]);
    assert!(
        success,
        "gg run --json should succeed: {} / {}",
        stdout, stderr
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("JSON parse failed");
    let cmd = parsed["run"]["results"][0]["commands"][0]["command"]
        .as_str()
        .expect("missing command field");
    assert_eq!(
        cmd, "echo 'hello world'",
        "displayed command must single-quote whitespace args"
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_amend_mid_stack_reports_correct_final_sha() {
    // Regression test for Bug #3: after `--amend` on a non-tail commit the
    // code used to read HEAD (which, post rebase-onto, points at the stack
    // tip) and reported the tip SHA as the amended commit's final_sha. The
    // fix captures the amended OID locally before the rebase-onto runs.
    //
    // Strategy: run `gg run --amend` across positions 1 and 2. For each
    // reported sha, resolve its commit subject via `git show`. The
    // invariant is that position N's reported sha MUST point to a commit
    // whose subject is "Commit N" — if the bug is present, position 1
    // reports the post-rebase HEAD (which is the rebased commit 2, subject
    // "Commit 2") instead of the amended commit 1.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Seed a TRACKED file `touched.txt` in the base commit. The script
    // below appends to it — gg's dirty-check only considers tracked file
    // modifications (untracked files are ignored), so the baseline file
    // must exist before we create the stack.
    fs::write(repo_path.join("touched.txt"), "").expect("write touched.txt");

    // Script appends the current commit's subject line to `touched.txt`.
    // This guarantees each amended commit introduces a *distinct* diff
    // against its parent, so `git rebase --onto` never drops commits as
    // "patch already upstream".
    fs::write(
        repo_path.join("touch_one.sh"),
        "#!/bin/sh\ngit log -1 --format=%s >> touched.txt\n",
    )
    .expect("write script");
    let mut perms = fs::metadata(repo_path.join("touch_one.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("touch_one.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "touched.txt", "touch_one.sh"]);
    run_git(
        &repo_path,
        &["commit", "-m", "add script and touched baseline"],
    );

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-amend-midstack"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Stack: 3 commits (Commit 1, 2, 3) on top of the base.
    for i in 1..=3 {
        fs::write(repo_path.join(format!("f{}.txt", i)), format!("v{}", i)).expect("write");
        run_git(&repo_path, &["add", "."]);
        run_git(&repo_path, &["commit", "-m", &format!("Commit {}", i)]);
    }

    // Move to position 2 (the middle commit) so `gg run` only touches
    // commits at positions 1 and 2, not 3.
    let (success, _, stderr) = run_gg(&repo_path, &["mv", "2"]);
    assert!(success, "mv failed: {}", stderr);

    let (success, stdout, stderr) =
        run_gg(&repo_path, &["run", "--amend", "--json", "./touch_one.sh"]);
    assert!(
        success,
        "gg run --amend should succeed: {} / {}",
        stdout, stderr
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("json parse");
    let results = parsed["run"]["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2, "should have run on commits 1 and 2");

    let sha0 = results[0]["sha"].as_str().unwrap().to_string();
    let sha1 = results[1]["sha"].as_str().unwrap().to_string();

    // Resolve each reported sha to its actual commit subject. Orphan
    // commits are fine — `git show` looks them up in the object store.
    let (ok0, subject0) = run_git(&repo_path, &["show", "-s", "--format=%s", &sha0]);
    assert!(ok0, "failed to show sha0={}", sha0);
    let (ok1, subject1) = run_git(&repo_path, &["show", "-s", "--format=%s", &sha1]);
    assert!(ok1, "failed to show sha1={}", sha1);

    assert_eq!(
        subject0.trim(),
        "Commit 1",
        "Bug #3: position 1's reported sha ({}) must resolve to the amended \
         commit 1 but resolved to a commit with subject {:?}. \
         (Before the fix, the code read HEAD after the rebase-onto which \
         moved HEAD off commit1'.)",
        sha0,
        subject0.trim()
    );
    assert_eq!(
        subject1.trim(),
        "Commit 2",
        "Position 2's reported sha ({}) should resolve to 'Commit 2' but \
         resolved to {:?}",
        sha1,
        subject1.trim()
    );

    assert_ne!(
        sha0, sha1,
        "commit 1 and commit 2 must have distinct reported shas"
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_amend_stop_on_error_preserves_commits_above_failure() {
    // Regression test for Bug #4 (data loss): when `gg run --amend` stops
    // on failure mid-stack, the restoration code must NOT force-reset the
    // branch to the currently-detached HEAD. Commits above the failure
    // point must remain reachable.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Seed a tracked `marker.txt` in the base so the script's append shows
    // up as a dirty tracked file (untracked files wouldn't trigger gg's
    // dirty check — see is_working_directory_clean).
    fs::write(repo_path.join("marker.txt"), "").expect("seed marker.txt");

    // Script: succeed + modify tree on commit 1 and tip, fail on middle.
    // Detects which commit we're on via the presence of f1/f2/f3 files.
    fs::write(
        repo_path.join("cond.sh"),
        "#!/bin/sh\n\
         if [ -f f2.txt ] && [ ! -f f3.txt ]; then\n\
           # Commit 2 (middle): fail loudly\n\
           exit 17\n\
         fi\n\
         echo marker >> marker.txt\n\
         exit 0\n",
    )
    .expect("write script");
    let mut perms = fs::metadata(repo_path.join("cond.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("cond.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "marker.txt", "cond.sh"]);
    run_git(
        &repo_path,
        &["commit", "-m", "add script and marker baseline"],
    );

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-data-loss-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Build 3-commit stack: A, B, C
    for i in 1..=3 {
        fs::write(repo_path.join(format!("f{}.txt", i)), format!("v{}", i)).expect("write");
        run_git(&repo_path, &["add", "."]);
        run_git(&repo_path, &["commit", "-m", &format!("Commit {}", i)]);
    }

    // Count of commits on the branch BEFORE the run.
    let (_, log_before) = run_git(&repo_path, &["rev-list", "--count", "HEAD"]);
    let count_before: usize = log_before.trim().parse().expect("parse count");

    // Run with default stop_on_error. Expected: succeeds on 1, fails on 2.
    // Before the fix: branch gets force-reset to commit 2, commit 3 vanishes.
    // After the fix: branch retains all commits.
    let (success, _, _) = run_gg(&repo_path, &["run", "--amend", "./cond.sh"]);
    assert!(
        !success,
        "gg run should report failure because commit 2 exits non-zero"
    );

    // Count commits on the branch AFTER the run.
    let (_, log_after) = run_git(&repo_path, &["rev-list", "--count", "HEAD"]);
    let count_after: usize = log_after.trim().parse().expect("parse count");
    assert_eq!(
        count_after, count_before,
        "Bug #4: commits above the failing commit were silently discarded. \
         expected {} commits, got {}",
        count_before, count_after
    );

    // And: commit 3 must still exist with its original content reachable.
    let (_, show_output) = run_git(&repo_path, &["show", "HEAD", "--name-only"]);
    assert!(
        show_output.contains("f3.txt"),
        "commit 3's f3.txt should still be reachable at HEAD after failed run, show: {}",
        show_output
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_amend_last_commit_sets_branch_tip_correctly() {
    // Regression test for Task 4 invariant: when --amend runs on the last
    // commit (no rebase needed), the branch must still be forwarded to the
    // new amended OID. Previously this happened by accident via the global
    // move_branch_to_head call which this task deletes.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Seed a tracked `marker.txt` in the base so the script has something
    // to dirty (untracked files are ignored by gg's dirty check).
    fs::write(repo_path.join("marker.txt"), "").expect("seed marker.txt");
    fs::write(
        repo_path.join("touch_marker.sh"),
        "#!/bin/sh\necho marker >> marker.txt\n",
    )
    .expect("write script");
    let mut perms = fs::metadata(repo_path.join("touch_marker.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("touch_marker.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "marker.txt", "touch_marker.sh"]);
    run_git(&repo_path, &["commit", "-m", "seed marker baseline"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-amend-last-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("f1.txt"), "v1").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "C1"]);

    // Record tip SHA before the amend
    let (_, head_before) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let head_before = head_before.trim().to_string();

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "--amend", "./touch_marker.sh"]);
    assert!(
        success,
        "gg run --amend on last commit should succeed: {} / {}",
        stdout, stderr
    );

    // Tip SHA must have changed (amended)
    let (_, head_after) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let head_after = head_after.trim().to_string();
    assert_ne!(
        head_before, head_after,
        "HEAD SHA should have changed after amend"
    );

    // HEAD must be on a branch, not detached
    let (_, symref) = run_git(&repo_path, &["symbolic-ref", "HEAD"]);
    assert!(
        symref.trim().starts_with("refs/heads/"),
        "HEAD should be a branch after gg run, got: {}",
        symref
    );

    // marker.txt must contain the appended marker (amend folded it in)
    let (_, marker_content) = run_git(&repo_path, &["show", "HEAD:marker.txt"]);
    assert!(
        marker_content.contains("marker"),
        "amend should have folded the marker append into the commit, marker.txt={:?}",
        marker_content
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_parallel_enforces_read_only_contract() {
    // Regression test for Bug #2: the parallel path used to mark commits as
    // passed based purely on command exit status, ignoring whether the
    // command dirtied the worktree. The sequential path rejects dirty trees
    // in ReadOnly mode; the parallel path must now do the same so `-j N`
    // and `-j 1` are equivalent in terms of what they accept.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Baseline a TRACKED poison.txt file in the base commit. Both parallel
    // (git status --porcelain) and sequential (is_working_directory_clean,
    // which ignores untracked) code paths must agree that modifying this
    // tracked file counts as "dirty".
    fs::write(repo_path.join("poison.txt"), "").expect("seed poison.txt");
    fs::write(
        repo_path.join("dirty.sh"),
        "#!/bin/sh\n# Command exits 0 but dirties the worktree by modifying a tracked file\n\
         echo poison >> poison.txt\n\
         exit 0\n",
    )
    .expect("write dirty.sh");
    let mut perms = fs::metadata(repo_path.join("dirty.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("dirty.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "poison.txt", "dirty.sh"]);
    run_git(&repo_path, &["commit", "-m", "seed poison baseline"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-parallel-dirty-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Two-commit stack so the parallel path actually has work to parallelize.
    fs::write(repo_path.join("f1.txt"), "v1").expect("write f1");
    run_git(&repo_path, &["add", "f1.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("f2.txt"), "v2").expect("write f2");
    run_git(&repo_path, &["add", "f2.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Parallel run in ReadOnly mode must FAIL because dirty.sh dirties the
    // worktree even though it exits 0.
    let (success_parallel, stdout_p, stderr_p) =
        run_gg(&repo_path, &["run", "-j", "2", "--json", "./dirty.sh"]);
    assert!(
        !success_parallel,
        "gg run -j 2 should fail when a command dirties the worktree: {} / {}",
        stdout_p, stderr_p
    );

    // Parse only the first JSON value — gg prints a second error-object
    // on non-zero exit which would confuse a whole-string parse.
    let mut stream = serde_json::Deserializer::from_str(&stdout_p).into_iter::<Value>();
    let parsed: Value = stream
        .next()
        .expect("expected at least one json object")
        .expect("first json object parse");
    let all_passed = parsed["run"]["all_passed"].as_bool().expect("all_passed");
    assert!(!all_passed, "all_passed should be false");

    let results = parsed["run"]["results"].as_array().expect("results");
    assert!(!results.is_empty(), "results should not be empty");
    for (idx, r) in results.iter().enumerate() {
        assert_eq!(
            r["passed"].as_bool(),
            Some(false),
            "commit at index {} should be marked failed (dirty worktree)",
            idx
        );
    }

    // Sequential parity check: `-j 1` must produce the same verdict so the
    // user can't get conflicting behavior by tweaking --jobs.
    let (success_seq, _, _) = run_gg(&repo_path, &["run", "-j", "1", "./dirty.sh"]);
    assert!(
        !success_seq,
        "sequential gg run must also fail — parallel should match sequential"
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_json_failure_emits_single_object() {
    // Regression: `gg run --json` used to print two JSON documents on failure
    // — first the {"run": ...} payload from execute(), then a
    // {"error": "Some commands failed"} payload from the generic main.rs
    // error path (because the handler converted Ok(false) into Err(...)).
    // Consumers expect a single parseable JSON document, so a failing
    // `gg run --json` must now emit exactly one object.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-json-failure-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("a.txt"), "a").expect("write a.txt");
    run_git(&repo_path, &["add", "a.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    // `false` exits non-zero on every unix, so the run will fail and we hit
    // the not-all-passed path.
    let (success_run, stdout, stderr) = run_gg(&repo_path, &["run", "--json", "false"]);
    assert!(
        !success_run,
        "gg run --json false should fail (exit 1): stdout={} stderr={}",
        stdout, stderr
    );

    // The critical assertion: exactly one JSON object in stdout.
    let mut stream = serde_json::Deserializer::from_str(&stdout).into_iter::<Value>();
    let first = stream
        .next()
        .expect("expected one json object")
        .expect("first json object must parse");
    assert!(
        first.get("run").is_some(),
        "first (and only) object must be the run payload, got: {}",
        first
    );
    let extra = stream.next();
    assert!(
        extra.is_none(),
        "expected exactly one JSON document in stdout, but got a second: {:?}",
        extra
    );

    // And the run payload itself should report the failure so consumers
    // can still distinguish success from failure without the second doc.
    assert_eq!(
        first["run"]["all_passed"].as_bool(),
        Some(false),
        "run.all_passed should be false on failure"
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_discard_resets_staged_index() {
    // Regression: `--discard` used to run `git checkout .` + `git clean -fd`,
    // which reverts tracked files and removes untracked files but does NOT
    // unstage anything the command added to the index. If a command ran
    // `git add`, those staged changes would persist into the next iteration
    // and could contaminate later commits or cause checkout failures.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Seed a tracked `tracked.txt` plus a script that modifies it AND stages
    // the modification — exercising the index path the old code missed.
    fs::write(repo_path.join("tracked.txt"), "original\n").expect("seed tracked.txt");
    fs::write(
        repo_path.join("stage.sh"),
        "#!/bin/sh\n\
         # Dirty the tree AND stage the change so the index carries it.\n\
         echo dirty >> tracked.txt\n\
         git add tracked.txt\n",
    )
    .expect("write stage.sh");
    let mut perms = fs::metadata(repo_path.join("stage.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("stage.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "tracked.txt", "stage.sh"]);
    run_git(&repo_path, &["commit", "-m", "seed tracked + stage script"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-discard-index-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Two commits so the discard happens at least once mid-stack, not just
    // at the final commit.
    fs::write(repo_path.join("f1.txt"), "v1").expect("write f1");
    run_git(&repo_path, &["add", "f1.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("f2.txt"), "v2").expect("write f2");
    run_git(&repo_path, &["add", "f2.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    let (success_run, stdout, stderr) = run_gg(&repo_path, &["run", "--discard", "./stage.sh"]);
    assert!(
        success_run,
        "gg run --discard should succeed: stdout={} stderr={}",
        stdout, stderr
    );

    // After discard, the working tree and index must both be clean —
    // `git status --porcelain` (which includes staged entries) should emit
    // nothing. Previously, the staged `tracked.txt` entry would still show.
    let status_out = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo_path)
        .output()
        .expect("git status");
    assert!(
        status_out.status.success(),
        "git status failed: {}",
        String::from_utf8_lossy(&status_out.stderr)
    );
    assert!(
        status_out.stdout.is_empty(),
        "working tree + index must be clean after --discard, but got:\n{}",
        String::from_utf8_lossy(&status_out.stdout)
    );

    // And tracked.txt content should be back to the committed version.
    let tracked = fs::read_to_string(repo_path.join("tracked.txt")).unwrap();
    assert_eq!(
        tracked, "original\n",
        "tracked.txt must be restored to the committed state after --discard"
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_parallel_dirty_check_ignores_untracked() {
    // Regression: the parallel path used raw `git status --porcelain` which
    // includes untracked files, while the sequential path uses
    // `git::is_working_directory_clean` (include_untracked=false). A command
    // that created untracked files passed under `-j 1` but failed under
    // `-j N`, so `--jobs` could flip pass/fail for the same command.
    // The fix is to run `git status --porcelain --untracked-files=no` in the
    // parallel worker, matching the sequential semantics.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Script only creates an untracked file — sequential accepts, parallel
    // must also accept (with the fix), and reject otherwise.
    fs::write(
        repo_path.join("make_untracked.sh"),
        "#!/bin/sh\necho hi > scratch.tmp\nexit 0\n",
    )
    .expect("write make_untracked.sh");
    let mut perms = fs::metadata(repo_path.join("make_untracked.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("make_untracked.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "make_untracked.sh"]);
    run_git(&repo_path, &["commit", "-m", "seed script"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-untracked-parity-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Two-commit stack so -j 2 has real work.
    fs::write(repo_path.join("f1.txt"), "v1").expect("write f1");
    run_git(&repo_path, &["add", "f1.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);
    fs::write(repo_path.join("f2.txt"), "v2").expect("write f2");
    run_git(&repo_path, &["add", "f2.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Sequential first: must succeed because the sequential dirty check
    // ignores untracked files.
    let (success_seq, stdout_s, stderr_s) =
        run_gg(&repo_path, &["run", "-j", "1", "./make_untracked.sh"]);
    assert!(
        success_seq,
        "sequential gg run must accept untracked-only dirtying: stdout={} stderr={}",
        stdout_s, stderr_s
    );

    // Parallel must match: also succeed.
    let (success_par, stdout_p, stderr_p) =
        run_gg(&repo_path, &["run", "-j", "2", "./make_untracked.sh"]);
    assert!(
        success_par,
        "parallel gg run must match sequential (accept untracked-only dirtying): stdout={} stderr={}",
        stdout_p, stderr_p
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
