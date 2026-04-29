use crate::helpers::{create_test_repo, run_gg, run_git};

use serde_json::Value;
use std::fs;

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
