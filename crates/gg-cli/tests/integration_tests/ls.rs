use crate::helpers::{
    create_test_repo, create_test_repo_with_remote, create_test_repo_with_worktree_support,
    create_worktree, run_gg, run_gg_with_env, run_git,
};

use serde_json::Value;
use std::fs;

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
fn test_gg_ls_warns_on_mismatched_stack_prefix() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "wrong-prefix-ls"]);
    assert!(success, "Failed to create stack: {}", stderr);
    run_git(&repo_path, &["branch", "-m", "other/wrong-prefix-ls"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["ls"]);
    assert!(success, "gg ls failed: {}", stderr);
    assert!(
        stdout.contains("Warning:"),
        "warning should appear: {stdout}"
    );
    assert!(
        stdout.contains("configured prefix 'testuser/'"),
        "configured prefix should appear: {stdout}"
    );
    assert!(
        stdout.contains("git branch -m testuser/wrong-prefix-ls"),
        "rename hint should appear: {stdout}"
    );
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
