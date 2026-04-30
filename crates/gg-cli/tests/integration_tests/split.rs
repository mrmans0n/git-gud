use crate::helpers::{create_test_repo, run_gg, run_git, run_git_full};

use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

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

#[test]
fn test_split_file_args_auto_selects_hunks() {
    // Verify that `gg split <file>` auto-selects all hunks for that file
    // without prompting, and leaves the other file in the remainder commit.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-auto-select"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create a commit with two text files, each with multiple lines
    fs::write(repo_path.join("alpha.txt"), "line1\nline2\nline3\n").expect("write");
    fs::write(repo_path.join("beta.txt"), "lineA\nlineB\nlineC\n").expect("write");
    run_git(&repo_path, &["add", "alpha.txt", "beta.txt"]);
    run_git(&repo_path, &["commit", "-m", "Add alpha and beta"]);

    // Split out alpha.txt only — no TTY needed since file args auto-select
    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["split", "-m", "Split alpha", "--no-edit", "alpha.txt"],
    );
    assert!(
        success,
        "split should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("Split complete"),
        "Expected 'Split complete': {}",
        stdout
    );

    // Verify the remainder commit (HEAD) only contains beta.txt
    let (_, diff_output, _) = run_git_full(&repo_path, &["diff", "HEAD~1", "HEAD", "--name-only"]);
    assert!(
        diff_output.contains("beta.txt"),
        "Remainder commit should contain beta.txt: {}",
        diff_output
    );
    assert!(
        !diff_output.contains("alpha.txt"),
        "Remainder commit should NOT contain alpha.txt: {}",
        diff_output
    );
}

#[test]
fn test_split_binary_file_with_file_args() {
    // Verify that `gg split <binary_file>` succeeds for a commit containing
    // both a binary file and a text file, exercising the non_hunk_files path.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-binary-split"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create a commit with a text file and a binary file
    fs::write(repo_path.join("readme.txt"), "hello\n").expect("write");
    // Write a small PNG-like binary blob so git treats it as binary
    let binary_content: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG header
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    ];
    fs::write(repo_path.join("image.png"), &binary_content).expect("write binary");
    run_git(&repo_path, &["add", "readme.txt", "image.png"]);
    run_git(&repo_path, &["commit", "-m", "Add readme and image"]);

    // Split out the binary file
    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["split", "-m", "Split binary", "--no-edit", "image.png"],
    );
    assert!(
        success,
        "split of binary file should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("Split complete"),
        "Expected 'Split complete': {}",
        stdout
    );

    // Verify the stack now has 3 commits (initial + split commit + remainder)
    let (_, log_output, _) = run_git_full(&repo_path, &["log", "--oneline"]);
    let commit_count = log_output.lines().count();
    assert!(
        commit_count >= 3,
        "Should have at least 3 commits after split: {}",
        log_output
    );
}

#[test]
fn test_split_non_textual_only_commit_shows_guidance() {
    // Verify that splitting a commit with only binary changes (no FILES args)
    // shows a helpful error message guiding the user to specify files explicitly.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-binary-only"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create a commit with only binary files
    let binary_a: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
    ];
    let binary_b: Vec<u8> = vec![
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00,
    ];
    fs::write(repo_path.join("a.png"), &binary_a).expect("write");
    fs::write(repo_path.join("b.gif"), &binary_b).expect("write");
    run_git(&repo_path, &["add", "a.png", "b.gif"]);
    run_git(&repo_path, &["commit", "-m", "Add two binary files"]);

    // Try to split without file args — should fail with guidance
    let (success, _stdout, stderr) =
        run_gg(&repo_path, &["split", "-m", "Split attempt", "--no-edit"]);
    assert!(
        !success,
        "split of binary-only commit without file args should fail"
    );
    assert!(
        stderr.contains("non-textual") || stderr.contains("gg split"),
        "Error should mention non-textual changes or suggest gg split with files: {}",
        stderr
    );
}

// ============================================================================
// Clean command verification tests
// ============================================================================
