use crate::helpers::{create_test_repo, run_gg, run_git};

use std::fs;

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
