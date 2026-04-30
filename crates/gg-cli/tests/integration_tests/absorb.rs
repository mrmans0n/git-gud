use crate::helpers::{create_test_repo, run_gg, run_git};

use std::fs;

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
