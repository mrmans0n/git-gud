use crate::helpers::{
    create_test_repo, create_test_repo_with_remote, create_test_repo_with_worktree_support,
    create_worktree, run_gg, run_gg_with_env, run_git,
};

use serde_json::Value;
use std::fs;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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
