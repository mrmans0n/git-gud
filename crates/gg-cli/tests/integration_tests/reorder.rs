use crate::helpers::{create_test_repo, run_gg, run_git};

use std::fs;
use std::process::Command;

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
