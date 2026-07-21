use crate::helpers::{create_test_repo, run_gg, run_git};

use std::fs;

fn head_sha(repo_path: &std::path::Path) -> String {
    let (success, stdout) = run_git(repo_path, &["rev-parse", "HEAD"]);
    assert!(success, "git rev-parse HEAD failed");
    stdout.trim().to_string()
}

fn assert_staged_only_ignores_unstaged_action(unstaged_action: &str) {
    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        format!(
            r#"{{"defaults":{{"branch_username":"testuser","unstaged_action":"{unstaged_action}"}}}}"#
        ),
    )
    .expect("Failed to write config");

    run_gg(
        &repo_path,
        &["co", &format!("squash-staged-only-{unstaged_action}")],
    );
    fs::write(repo_path.join("staged.txt"), "original staged\n").unwrap();
    fs::write(repo_path.join("unstaged.txt"), "original unstaged\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Initial files"]);

    fs::write(repo_path.join("staged.txt"), "amended staged\n").unwrap();
    run_git(&repo_path, &["add", "staged.txt"]);
    fs::write(repo_path.join("unstaged.txt"), "working unstaged\n").unwrap();
    fs::write(repo_path.join("untracked.txt"), "working untracked\n").unwrap();

    let (success, stdout, stderr) = run_gg(&repo_path, &["sc", "--staged-only"]);
    assert!(
        success,
        "--staged-only must ignore unstaged_action={unstaged_action}: stdout={stdout} stderr={stderr}"
    );

    let (success, committed_staged) = run_git(&repo_path, &["show", "HEAD:staged.txt"]);
    assert!(success);
    assert_eq!(committed_staged, "amended staged\n");
    let (success, committed_unstaged) = run_git(&repo_path, &["show", "HEAD:unstaged.txt"]);
    assert!(success);
    assert_eq!(committed_unstaged, "original unstaged\n");
    let (success, _) = run_git(&repo_path, &["cat-file", "-e", "HEAD:untracked.txt"]);
    assert!(!success, "untracked content must not be amended");

    let (success, status) = run_git(&repo_path, &["status", "--porcelain"]);
    assert!(success);
    assert!(status.contains(" M unstaged.txt"), "status={status}");
    assert!(status.contains("?? untracked.txt"), "status={status}");
    assert!(
        !status
            .lines()
            .any(|line| line.get(3..) == Some("staged.txt")),
        "staged change should be clean: {status}"
    );

    let (success, stash_list) = run_git(&repo_path, &["stash", "list"]);
    assert!(success);
    assert!(
        stash_list.trim().is_empty(),
        "--staged-only must not auto-stash: {stash_list}"
    );
}

#[test]
fn test_gg_squash_staged_only_ignores_add_default_at_stack_head() {
    assert_staged_only_ignores_unstaged_action("add");
}

#[test]
fn test_gg_squash_staged_only_ignores_stash_default_at_stack_head() {
    assert_staged_only_ignores_unstaged_action("stash");
}

#[test]
fn test_gg_squash_staged_only_fails_before_mid_stack_rebase_with_unstaged_changes() {
    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","unstaged_action":"add"}}"#,
    )
    .expect("Failed to write config");

    run_gg(&repo_path, &["co", "squash-staged-only-mid-stack"]);
    fs::write(repo_path.join("first.txt"), "first\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "First"]);
    fs::write(repo_path.join("second.txt"), "second\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Second"]);
    let (success, _, stderr) = run_gg(&repo_path, &["mv", "1"]);
    assert!(success, "move failed: {stderr}");

    fs::write(repo_path.join("first.txt"), "unstaged first\n").unwrap();
    fs::write(repo_path.join("staged.txt"), "staged\n").unwrap();
    run_git(&repo_path, &["add", "staged.txt"]);
    let head_before = head_sha(&repo_path);
    let (_, status_before) = run_git(&repo_path, &["status", "--porcelain"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["sc", "--staged-only"]);
    assert!(
        !success,
        "mid-stack staged-only must refuse unstaged changes: stdout={stdout} stderr={stderr}"
    );
    assert!(
        stderr.contains("Unstaged changes detected"),
        "stderr={stderr}"
    );
    assert_eq!(
        head_sha(&repo_path),
        head_before,
        "HEAD must not be amended"
    );
    let (_, status_after) = run_git(&repo_path, &["status", "--porcelain"]);
    assert_eq!(
        status_after, status_before,
        "index/worktree must be untouched"
    );
    let (_, stash_list) = run_git(&repo_path, &["stash", "list"]);
    assert!(
        stash_list.trim().is_empty(),
        "must not auto-stash: {stash_list}"
    );
}

#[test]
fn test_gg_squash_staged_only_rejects_untracked_descendant_collision_before_amend() {
    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","unstaged_action":"add"}}"#,
    )
    .unwrap();
    run_gg(&repo_path, &["co", "squash-staged-only-untracked"]);

    fs::write(repo_path.join("first.txt"), "first\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "First"]);
    fs::write(repo_path.join("descendant.txt"), "descendant\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Second"]);
    let (success, _, stderr) = run_gg(&repo_path, &["mv", "1"]);
    assert!(success, "move failed: {stderr}");

    fs::write(repo_path.join("prepared.txt"), "prepared\n").unwrap();
    run_git(&repo_path, &["add", "prepared.txt"]);
    fs::write(repo_path.join("descendant.txt"), "untracked collision\n").unwrap();
    let head_before = head_sha(&repo_path);
    let (_, status_before) = run_git(&repo_path, &["status", "--porcelain"]);
    let operations_before = fs::read_dir(gg_dir.join("operations"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .filter(|record| record["kind"] == "squash")
        .count();

    let (success, stdout, stderr) = run_gg(&repo_path, &["sc", "--staged-only"]);
    assert!(
        !success,
        "untracked collision must be refused before amend: stdout={stdout} stderr={stderr}"
    );
    assert!(
        stderr.contains("Untracked files detected"),
        "stderr={stderr}"
    );
    assert_eq!(
        head_sha(&repo_path),
        head_before,
        "HEAD must not be amended"
    );
    let (_, status_after) = run_git(&repo_path, &["status", "--porcelain"]);
    assert_eq!(
        status_after, status_before,
        "index/worktree must be untouched"
    );
    let operations_after = fs::read_dir(gg_dir.join("operations"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .filter(|record| record["kind"] == "squash")
        .count();
    assert_eq!(
        operations_after, operations_before,
        "preflight refusal must not create a Pending squash operation"
    );
}

#[test]
fn test_gg_squash_staged_only_help_and_all_conflict() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, stderr) = run_gg(&repo_path, &["sc", "--help"]);
    assert!(success, "sc help failed: {stderr}");
    assert!(stdout.contains("--staged-only"), "stdout={stdout}");

    let (success, stdout, stderr) = run_gg(&repo_path, &["sc", "--all", "--staged-only"]);
    assert!(
        !success && stderr.contains("cannot be used with"),
        "--all and --staged-only must conflict: stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn test_gg_squash_staged_only_without_staged_changes_does_not_amend() {
    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","unstaged_action":"add"}}"#,
    )
    .unwrap();
    run_gg(&repo_path, &["co", "squash-staged-only-empty"]);
    fs::write(repo_path.join("tracked.txt"), "original\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Tracked"]);
    let head_before = head_sha(&repo_path);

    fs::write(repo_path.join("tracked.txt"), "unstaged\n").unwrap();
    let (success, stdout, stderr) = run_gg(&repo_path, &["sc", "--staged-only"]);
    assert!(success, "staged-only no-op failed: {stderr}");
    assert!(stdout.contains("No staged changes"), "stdout={stdout}");
    assert_eq!(head_sha(&repo_path), head_before);
    assert_eq!(
        fs::read_to_string(repo_path.join("tracked.txt")).unwrap(),
        "unstaged\n"
    );
}

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
fn test_gg_squash_continue_restores_squashed_commit_navigation() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let branch_name = "testuser/squash-continue-nav";
    run_gg(&repo_path, &["co", "squash-continue-nav"]);

    fs::write(repo_path.join("README.md"), "one\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("README.md"), "two\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    let (success, _, _) = run_gg(&repo_path, &["mv", "1"]);
    assert!(success, "Failed to navigate to first commit");
    let head_before_squash = head_sha(&repo_path);

    fs::write(repo_path.join("README.md"), "amended one\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "README.md"]);
    let (success, stdout, stderr) = run_gg(&repo_path, &["sc"]);
    assert!(
        !success,
        "squash should conflict while rebasing descendants: stdout={stdout} stderr={stderr}"
    );

    fs::write(repo_path.join("README.md"), "resolved two\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "README.md"]);
    let (success, stdout, stderr) = run_gg(&repo_path, &["continue"]);
    assert!(
        success,
        "continue should complete interrupted squash: stdout={stdout} stderr={stderr}"
    );

    let (success, stdout) = run_git(&repo_path, &["log", "-1", "--pretty=%s"]);
    assert!(success, "git log failed");
    assert_eq!(
        stdout.trim(),
        "Commit 1",
        "continued squash should leave HEAD on the squashed commit"
    );

    let (success, stdout) = run_git(
        &repo_path,
        &[
            "log",
            "-1",
            "--pretty=%s",
            &format!("refs/heads/{branch_name}"),
        ],
    );
    assert!(success, "git log branch tip failed");
    assert_eq!(
        stdout.trim(),
        "Commit 2",
        "stack branch tip should remain on the rebased descendant"
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", "--json"]);
    assert!(success, "undo failed: stdout={stdout} stderr={stderr}");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("stdout must be JSON");
    assert_eq!(parsed["status"], "succeeded");
    assert_eq!(parsed["undone"]["kind"], "squash");
    assert_eq!(
        head_sha(&repo_path),
        head_before_squash,
        "undo should restore pre-squash HEAD"
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
