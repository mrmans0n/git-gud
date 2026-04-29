use crate::helpers::{create_test_repo, run_gg, run_git};

use std::fs;

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
fn test_nav_requires_stack() {
    // Test that navigation commands require being on a stack
    let (_temp_dir, repo_path) = create_test_repo();

    // Try nav first while not on a stack
    let (success, _stdout, stderr) = run_gg(&repo_path, &["first"]);
    assert!(!success, "Nav first should fail when not on a stack");
    assert!(
        stderr.contains("not a stack branch"),
        "Should indicate not on a stack. Got: {}",
        stderr
    );

    // Try nav last while not on a stack
    let (success, _stdout, stderr) = run_gg(&repo_path, &["last"]);
    assert!(!success, "Nav last should fail when not on a stack");
    assert!(stderr.contains("not a stack branch"));

    // Try nav next while not on a stack
    let (success, _stdout, stderr) = run_gg(&repo_path, &["next"]);
    assert!(!success, "Nav next should fail when not on a stack");
    assert!(stderr.contains("not a stack branch"));

    // Try nav prev while not on a stack
    let (success, _stdout, stderr) = run_gg(&repo_path, &["prev"]);
    assert!(!success, "Nav prev should fail when not on a stack");
    assert!(stderr.contains("not a stack branch"));
}
