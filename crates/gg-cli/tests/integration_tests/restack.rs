use crate::helpers::{create_test_repo, run_gg, run_git};

use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

fn setup_restack_repo(stack_name: &str, num_commits: usize) -> (TempDir, PathBuf) {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", stack_name]);
    assert!(success, "co failed: {}", stderr);

    // Position 1 has no GG-Parent (it's the base), subsequent commits
    // reference the previous commit's GG-ID.
    let mut prev_gg_id: Option<String> = None;
    for i in 1..=num_commits {
        let gg_id = format!("c-{:07x}", i);
        fs::write(
            repo_path.join(format!("file{}.txt", i)),
            format!("content {}", i),
        )
        .unwrap();
        run_git(&repo_path, &["add", "."]);

        let mut msg = format!("Commit {}\n\nGG-ID: {}", i, gg_id);
        if let Some(ref parent) = prev_gg_id {
            msg.push_str(&format!("\nGG-Parent: {}", parent));
        }
        run_git(&repo_path, &["commit", "-m", &msg]);

        prev_gg_id = Some(gg_id);
    }

    (_temp_dir, repo_path)
}

/// Helper to create a repo with a broken GG-Parent chain at a specific position.
/// Commits above `break_at` (1-indexed) will have stale GG-Parent trailers.
fn setup_restack_repo_broken(
    stack_name: &str,
    num_commits: usize,
    break_at: usize,
) -> (TempDir, PathBuf) {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", stack_name]);
    assert!(success, "co failed: {}", stderr);

    let mut prev_gg_id: Option<String> = None;
    for i in 1..=num_commits {
        let gg_id = format!("c-{:07x}", i);
        fs::write(
            repo_path.join(format!("file{}.txt", i)),
            format!("content {}", i),
        )
        .unwrap();
        run_git(&repo_path, &["add", "."]);

        let mut msg = format!("Commit {}\n\nGG-ID: {}", i, gg_id);
        if let Some(ref parent) = prev_gg_id {
            if i == break_at + 1 {
                // Intentionally wrong GG-Parent
                msg.push_str("\nGG-Parent: c-deadbee");
            } else {
                msg.push_str(&format!("\nGG-Parent: {}", parent));
            }
        }
        run_git(&repo_path, &["commit", "-m", &msg]);

        prev_gg_id = Some(gg_id);
    }

    (_temp_dir, repo_path)
}

#[test]
fn test_restack_consistent_stack_is_noop() {
    let (_temp_dir, repo_path) = setup_restack_repo("restack-noop", 3);

    // Restack should be a no-op
    let (success, stdout, stderr) = run_gg(&repo_path, &["restack"]);
    assert!(
        success,
        "restack failed. stdout: {}, stderr: {}",
        stdout, stderr
    );
    assert!(
        stdout.contains("already consistent"),
        "Expected 'already consistent', got: {}",
        stdout
    );
}

#[test]
fn test_restack_dry_run_json() {
    // Break at position 1: commit 2 has wrong GG-Parent
    let (_temp_dir, repo_path) = setup_restack_repo_broken("restack-dryrun", 3, 1);

    // Dry-run with JSON
    let (success, stdout, stderr) = run_gg(&repo_path, &["restack", "--dry-run", "--json"]);
    assert!(
        success,
        "restack dry-run failed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    let json: Value = serde_json::from_str(&stdout).expect("Should be valid JSON");
    assert_eq!(json["version"], 1);
    assert_eq!(json["restack"]["dry_run"], true);
    assert!(json["restack"]["total_entries"].as_u64().unwrap() >= 3);
    let steps = json["restack"]["steps"].as_array().unwrap();
    let reattach_count = steps.iter().filter(|s| s["action"] == "reattach").count();
    assert!(
        reattach_count > 0,
        "Expected at least one reattach step, got 0. Steps: {:?}",
        steps
    );
}

#[test]
fn test_restack_repairs_broken_chain() {
    // Break at position 1: commit 2 has wrong GG-Parent
    let (_temp_dir, repo_path) = setup_restack_repo_broken("restack-repair", 3, 1);

    // Execute restack
    let (success, stdout, stderr) = run_gg(&repo_path, &["restack"]);
    assert!(
        success,
        "restack failed. stdout: {}, stderr: {}",
        stdout, stderr
    );
    assert!(
        stdout.contains("Restacked"),
        "Expected 'Restacked' in output, got: {}",
        stdout
    );

    // Verify: a second restack should now be a no-op
    let (success, stdout, _) = run_gg(&repo_path, &["restack"]);
    assert!(success);
    assert!(
        stdout.contains("already consistent"),
        "Expected no-op after repair, got: {}",
        stdout
    );

    // Verify all 3 commits are still present
    let (success, stdout, _) = run_gg(&repo_path, &["ls"]);
    assert!(success);
    assert!(stdout.contains("Commit 1"));
    assert!(stdout.contains("Commit 2"));
    assert!(stdout.contains("Commit 3"));
}

#[test]
fn test_restack_from_partial_repair() {
    // Break at position 2: commit 3 has wrong GG-Parent
    let (_temp_dir, repo_path) = setup_restack_repo_broken("restack-from", 4, 2);

    // Partial restack from position 3 (skip position 1-2)
    let (success, stdout, stderr) = run_gg(&repo_path, &["restack", "--from", "3"]);
    assert!(
        success,
        "restack --from failed. stdout: {}, stderr: {}",
        stdout, stderr
    );
    assert!(
        stdout.contains("Restacked"),
        "Expected 'Restacked', got: {}",
        stdout
    );

    // Verify all 4 commits survived
    let (success, stdout, _) = run_gg(&repo_path, &["ls"]);
    assert!(success);
    assert!(stdout.contains("Commit 1"));
    assert!(stdout.contains("Commit 2"));
    assert!(stdout.contains("Commit 3"));
    assert!(stdout.contains("Commit 4"));
}

#[test]
fn test_restack_json_execution_output() {
    // Break at position 1: commit 2 has wrong GG-Parent
    let (_temp_dir, repo_path) = setup_restack_repo_broken("restack-json-exec", 2, 1);

    // Execute with JSON
    let (success, stdout, stderr) = run_gg(&repo_path, &["restack", "--json"]);
    assert!(
        success,
        "restack --json failed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    let json: Value = serde_json::from_str(&stdout).expect("Should be valid JSON");
    assert_eq!(json["version"], 1);
    assert_eq!(json["restack"]["dry_run"], false);
    assert!(json["restack"]["entries_restacked"].as_u64().unwrap() > 0);
    assert!(json["restack"]["steps"].as_array().unwrap().len() >= 2);
}

#[test]
fn test_restack_empty_stack_errors() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", "restack-empty"]);
    assert!(success, "co failed: {}", stderr);

    // Restack with no commits in the stack
    let (success, _, stderr) = run_gg(&repo_path, &["restack"]);
    assert!(!success, "Expected failure on empty stack");
    assert!(
        stderr.contains("empty") || stderr.contains("Empty"),
        "Expected 'empty' error, got: {}",
        stderr
    );
}

#[test]
fn test_restack_missing_gg_ids_errors() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", "restack-no-ids"]);
    assert!(success, "co failed: {}", stderr);

    // Create commits without syncing (so they have no GG-IDs)
    fs::write(repo_path.join("file1.txt"), "content 1").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    // Restack should fail because there are no GG-IDs
    let (success, _, stderr) = run_gg(&repo_path, &["restack"]);
    assert!(!success, "Expected failure without GG-IDs");
    assert!(
        stderr.contains("GG-ID") || stderr.contains("reconcile"),
        "Expected GG-ID or reconcile error, got: {}",
        stderr
    );
}
