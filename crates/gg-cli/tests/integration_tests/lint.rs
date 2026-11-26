use crate::helpers::{
    create_test_repo, create_test_repo_with_remote, create_test_repo_with_worktree_support,
    create_worktree, run_gg, run_gg_with_env, run_git, run_git_full,
};

use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

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
fn test_gg_lint_no_commands_shows_example_configuration() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "lint-no-commands"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint"]);
    assert!(success, "gg lint failed: {}", stderr);
    assert!(stdout.contains("No lint commands configured"));
    assert!(stdout.contains("Example configuration:"));
    assert!(stdout.contains(r#""lint": ["cargo fmt", "cargo clippy -- -D warnings"]"#));
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

    // Commits above --until must stay on the branch, rebased onto the linted ones
    let (_, log) = run_git(&repo_path, &["log", "--format=%s", "-4"]);
    let subjects: Vec<&str> = log.lines().collect();
    assert_eq!(
        subjects,
        vec![
            "Third commit",
            "Second commit",
            "First commit",
            "Add lint script"
        ],
        "lint --until must not drop commits above the range"
    );
    let (_, file1) = run_git(&repo_path, &["show", "HEAD:file1.txt"]);
    assert!(file1.contains("linted"), "Lint fix should be in the stack");
    let (_, file3) = run_git(&repo_path, &["show", "HEAD:file3.txt"]);
    assert!(
        !file3.contains("linted"),
        "Commit above --until must not be linted"
    );
}

#[cfg(unix)]
fn create_detached_restore_repo(lint_command: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let (_temp_dir, repo_path) = create_test_repo();

    fs::write(
        repo_path.join("lint.sh"),
        format!("#!/bin/sh\n{lint_command}\nexit 0\n"),
    )
    .expect("Failed to write lint script");
    let mut permissions = fs::metadata(repo_path.join("lint.sh"))
        .unwrap()
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(repo_path.join("lint.sh"), permissions).unwrap();
    run_git(&repo_path, &["add", "lint.sh"]);
    run_git(&repo_path, &["commit", "-m", "Add lint script"]);

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["./lint.sh"]}}"#,
    )
    .expect("Failed to write config");

    (_temp_dir, repo_path)
}

#[cfg(unix)]
#[test]
fn test_lint_until_restores_rewritten_detached_entry() {
    let (_temp_dir, repo_path) = create_detached_restore_repo(
        "[ -f file2.txt ] && [ ! -f file3.txt ] && echo linted >> file2.txt",
    );

    let stack_branch = "testuser/lint-detached-until";
    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-detached-until"]);
    assert!(success, "Failed to create stack: {stderr}");

    for position in 1..=3 {
        fs::write(
            repo_path.join(format!("file{position}.txt")),
            format!("content{position}\n"),
        )
        .expect("Failed to write file");
        run_git(&repo_path, &["add", "."]);
        run_git(
            &repo_path,
            &[
                "commit",
                "-m",
                &format!("Entry {position}\n\nGG-ID: c-dea000{position}"),
            ],
        );
    }

    let (_, old_stack_tip) = run_git(&repo_path, &["rev-parse", stack_branch]);
    let (success, _, stderr) = run_gg(&repo_path, &["mv", "2"]);
    assert!(success, "Failed to detach at entry 2: {stderr}");
    let (attached, _) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert!(!attached, "gg mv 2 should leave HEAD detached");
    let (_, old_entry_two) = run_git(&repo_path, &["rev-parse", "HEAD"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "2"]);
    assert!(success, "Lint should succeed: {stdout} {stderr}");

    let (_, stack_count) = run_git(
        &repo_path,
        &["rev-list", "--count", &format!("main..{stack_branch}")],
    );
    assert_eq!(stack_count.trim(), "3");
    let (_, stack_tip) = run_git(&repo_path, &["rev-parse", stack_branch]);
    assert_ne!(stack_tip.trim(), old_stack_tip.trim());
    let (_, tip_message) = run_git(&repo_path, &["show", "-s", "--format=%B", stack_tip.trim()]);
    assert!(tip_message.contains("GG-ID: c-dea0003"));

    let (_, rewritten_entry_two) = run_git(
        &repo_path,
        &[
            "log",
            "-1",
            "--format=%H",
            "--grep=GG-ID: c-dea0002",
            stack_branch,
        ],
    );
    assert_ne!(rewritten_entry_two.trim(), old_entry_two.trim());
    let (_, head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), rewritten_entry_two.trim());
    let (attached, _) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert!(
        !attached,
        "HEAD should remain detached at rewritten entry 2"
    );
}

#[cfg(unix)]
#[test]
fn test_failed_lint_restores_rewritten_detached_entry() {
    let (_temp_dir, repo_path) =
        create_detached_restore_repo("[ -f file1.txt ] && echo linted >> file1.txt && rm \"$0\"");

    let stack_branch = "testuser/lint-detached-error";
    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-detached-error"]);
    assert!(success, "Failed to create stack: {stderr}");

    for position in 1..=2 {
        fs::write(
            repo_path.join(format!("file{position}.txt")),
            format!("entry {position}\n"),
        )
        .expect("Failed to write file");
        run_git(&repo_path, &["add", "."]);
        run_git(
            &repo_path,
            &[
                "commit",
                "-m",
                &format!("Entry {position}\n\nGG-ID: c-eee000{position}"),
            ],
        );
    }

    let (success, _, stderr) = run_gg(&repo_path, &["mv", "1"]);
    assert!(success, "Failed to detach at entry 1: {stderr}");
    let (_, original_head) = run_git(&repo_path, &["rev-parse", "HEAD"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "2"]);
    assert!(
        !success,
        "Lint should fail after its command removes itself: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("not found"),
        "Expected command-not-found failure: stdout={stdout}, stderr={stderr}"
    );

    let (_, stack_count) = run_git(
        &repo_path,
        &["rev-list", "--count", &format!("main..{stack_branch}")],
    );
    assert_eq!(stack_count.trim(), "2");
    let (_, stack_tip_message) = run_git(&repo_path, &["show", "-s", "--format=%B", stack_branch]);
    assert!(stack_tip_message.contains("GG-ID: c-eee0002"));

    let (_, rewritten_entry_one) = run_git(
        &repo_path,
        &[
            "log",
            "-1",
            "--format=%H",
            "--fixed-strings",
            "--grep=GG-ID: c-eee0001",
            stack_branch,
        ],
    );
    assert_ne!(rewritten_entry_one.trim(), original_head.trim());
    let (_, linted_file) = run_git(
        &repo_path,
        &["show", &format!("{}:file1.txt", rewritten_entry_one.trim())],
    );
    assert!(linted_file.contains("linted"));

    let (attached, _) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert!(!attached, "HEAD should remain detached after lint fails");
    let (_, head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(
        head.trim(),
        rewritten_entry_one.trim(),
        "original={}, stdout={stdout}, stderr={stderr}",
        original_head.trim()
    );
}

#[cfg(unix)]
#[test]
fn test_failed_lint_refresh_restores_rewritten_detached_entry() {
    let (_temp_dir, repo_path) = create_detached_restore_repo(
        "[ -f file1.txt ] && echo linted >> file1.txt && git branch -D main",
    );

    let stack_branch = "testuser/lint-detached-refresh-error";
    let (_, base_oid) = run_git(&repo_path, &["rev-parse", "main"]);
    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-detached-refresh-error"]);
    assert!(success, "Failed to create stack: {stderr}");

    for position in 1..=2 {
        fs::write(
            repo_path.join(format!("file{position}.txt")),
            format!("entry {position}\n"),
        )
        .expect("Failed to write file");
        run_git(&repo_path, &["add", "."]);
        run_git(
            &repo_path,
            &[
                "commit",
                "-m",
                &format!("Entry {position}\n\nGG-ID: c-fff000{position}"),
            ],
        );
    }

    let (success, _, stderr) = run_gg(&repo_path, &["mv", "1"]);
    assert!(success, "Failed to detach at entry 1: {stderr}");
    let (_, original_head) = run_git(&repo_path, &["rev-parse", "HEAD"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "1"]);
    assert!(
        !success,
        "Lint should fail when the base disappears before refresh: stdout={stdout}, stderr={stderr}"
    );
    let (base_exists, _) = run_git(&repo_path, &["rev-parse", "--verify", "refs/heads/main"]);
    assert!(!base_exists, "Lint command should remove the base branch");

    let (_, stack_count) = run_git(
        &repo_path,
        &[
            "rev-list",
            "--count",
            &format!("{}..{stack_branch}", base_oid.trim()),
        ],
    );
    assert_eq!(stack_count.trim(), "2");
    let (_, stack_tip_message) = run_git(&repo_path, &["show", "-s", "--format=%B", stack_branch]);
    assert!(stack_tip_message.contains("GG-ID: c-fff0002"));

    let (_, rewritten_entry_one) = run_git(
        &repo_path,
        &[
            "log",
            "-1",
            "--format=%H",
            "--fixed-strings",
            "--grep=GG-ID: c-fff0001",
            stack_branch,
        ],
    );
    assert_ne!(rewritten_entry_one.trim(), original_head.trim());
    let (_, linted_file) = run_git(
        &repo_path,
        &["show", &format!("{}:file1.txt", rewritten_entry_one.trim())],
    );
    assert!(linted_file.contains("linted"));

    let (attached, _) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert!(!attached, "HEAD should remain detached after refresh fails");
    let (_, head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(
        head.trim(),
        rewritten_entry_one.trim(),
        "original={}, stdout={stdout}, stderr={stderr}",
        original_head.trim()
    );
}

#[cfg(unix)]
#[test]
fn test_failed_lint_refresh_uses_local_branch_when_tag_has_same_name() {
    let (_temp_dir, repo_path) = create_detached_restore_repo(
        "[ -f file1.txt ] && echo linted >> file1.txt && git branch -D main",
    );

    let stack_branch = "testuser/lint-ambiguous-refresh";
    let (_, base_oid) = run_git(&repo_path, &["rev-parse", "main"]);
    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-ambiguous-refresh"]);
    assert!(success, "Failed to create stack: {stderr}");

    for position in 1..=2 {
        fs::write(
            repo_path.join(format!("file{position}.txt")),
            format!("entry {position}\n"),
        )
        .expect("Failed to write file");
        run_git(&repo_path, &["add", "."]);
        run_git(
            &repo_path,
            &[
                "commit",
                "-m",
                &format!("Entry {position}\n\nGG-ID: c-bbb100{position}"),
            ],
        );
    }
    run_git(&repo_path, &["tag", stack_branch, base_oid.trim()]);
    let (_, tag_oid) = run_git(
        &repo_path,
        &["rev-parse", &format!("refs/tags/{stack_branch}")],
    );
    assert_eq!(tag_oid.trim(), base_oid.trim());

    let (success, _, stderr) = run_gg(&repo_path, &["mv", "1"]);
    assert!(success, "Failed to detach at entry 1: {stderr}");
    let (_, original_entry_one) = run_git(&repo_path, &["rev-parse", "HEAD"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "1"]);
    assert!(
        !success,
        "Lint should fail when the base disappears before canonical refresh: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("Could not find base branch"),
        "Expected canonical refresh failure: stdout={stdout}, stderr={stderr}"
    );

    let (_, rewritten_entry_one) = run_git(
        &repo_path,
        &[
            "log",
            "-1",
            "--format=%H",
            "--fixed-strings",
            "--grep=GG-ID: c-bbb1001",
            &format!("refs/heads/{stack_branch}"),
        ],
    );
    assert_ne!(rewritten_entry_one.trim(), original_entry_one.trim());
    assert_ne!(rewritten_entry_one.trim(), base_oid.trim());
    let (_, linted_file) = run_git(
        &repo_path,
        &["show", &format!("{}:file1.txt", rewritten_entry_one.trim())],
    );
    assert!(linted_file.contains("linted"));

    let (_, branch_count) = run_git(
        &repo_path,
        &[
            "rev-list",
            "--count",
            &format!("{}..refs/heads/{stack_branch}", base_oid.trim()),
        ],
    );
    assert_eq!(branch_count.trim(), "2");
    let (_, tag_after) = run_git(
        &repo_path,
        &["rev-parse", &format!("refs/tags/{stack_branch}")],
    );
    assert_eq!(tag_after.trim(), base_oid.trim());

    let (attached, _) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert!(!attached, "HEAD should remain detached after lint fails");
    let (_, head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(
        head.trim(),
        rewritten_entry_one.trim(),
        "Recovery must follow the rewritten local branch entry rather than the stale original or same-named tag: original={}, base={}, stdout={stdout}, stderr={stderr}",
        original_entry_one.trim(),
        base_oid.trim()
    );
}

#[cfg(unix)]
#[test]
fn test_lint_fails_when_rewritten_detached_checkout_is_blocked() {
    let lint_command = "printf 'tracked\\n' > blocked.txt && git add -f blocked.txt";
    let (_temp_dir, repo_path) = create_detached_restore_repo(lint_command);

    fs::write(repo_path.join(".gitignore"), "blocked.txt/\n").expect("Failed to write ignore rule");
    fs::write(
        repo_path.join(".gitattributes"),
        "trigger.txt filter=blocker\n",
    )
    .expect("Failed to write attributes");
    let smudge_path = repo_path.join(".git/gg/blocker-smudge");
    fs::write(
        &smudge_path,
        "#!/bin/sh\ncat\ngit_dir=$(git rev-parse --git-dir)\nif { [ -d \"$git_dir/rebase-merge\" ] || [ -d \"$git_dir/rebase-apply\" ]; } && [ ! -e blocked.txt ]; then\n  touch \"$git_dir/gg/smudge-ran\"\n  mkdir blocked.txt\n  printf 'obstruction\\n' > blocked.txt/obstruction\n  chmod 000 blocked.txt\nfi\n",
    )
    .expect("Failed to write smudge filter");
    let mut permissions = fs::metadata(&smudge_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&smudge_path, permissions).unwrap();
    run_git(
        &repo_path,
        &["config", "filter.blocker.smudge", ".git/gg/blocker-smudge"],
    );
    run_git(&repo_path, &["config", "filter.blocker.clean", "cat"]);
    run_git(&repo_path, &["config", "filter.blocker.required", "true"]);
    run_git(&repo_path, &["add", ".gitignore", ".gitattributes"]);
    run_git(&repo_path, &["commit", "--amend", "--no-edit"]);

    let stack_branch = "testuser/lint-blocked-restore";
    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-blocked-restore"]);
    assert!(success, "Failed to create stack: {stderr}");

    fs::write(repo_path.join("file1.txt"), "entry 1\n").expect("Failed to write entry 1");
    run_git(&repo_path, &["add", "file1.txt"]);
    run_git(&repo_path, &["commit", "-m", "Entry 1\n\nGG-ID: c-ccc1001"]);
    fs::write(repo_path.join("blocked.txt"), "tracked\n")
        .expect("Failed to write the future blocking path");
    run_git(&repo_path, &["add", "-f", "blocked.txt"]);
    run_git(&repo_path, &["commit", "-m", "Entry 2\n\nGG-ID: c-ccc1002"]);
    run_git(&repo_path, &["rm", "blocked.txt"]);
    fs::write(repo_path.join("trigger.txt"), "trigger\n").expect("Failed to write trigger");
    fs::write(repo_path.join("entry3.marker"), "entry 3\n")
        .expect("Failed to write entry 3 marker");
    run_git(&repo_path, &["add", "trigger.txt", "entry3.marker"]);
    run_git(&repo_path, &["commit", "-m", "Entry 3\n\nGG-ID: c-ccc1003"]);

    let (success, _, stderr) = run_gg(&repo_path, &["mv", "1"]);
    assert!(success, "Failed to detach at entry 1: {stderr}");
    let (_, original_entry_one) = run_git(&repo_path, &["rev-parse", "HEAD"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "1"]);

    let (_, rewritten_entry_one) = run_git(
        &repo_path,
        &[
            "log",
            "-1",
            "--format=%H",
            "--fixed-strings",
            "--grep=GG-ID: c-ccc1001",
            stack_branch,
        ],
    );
    assert_ne!(rewritten_entry_one.trim(), original_entry_one.trim());
    let (rewritten_tracks_blocker, _) = run_git(
        &repo_path,
        &[
            "cat-file",
            "-e",
            &format!("{}:blocked.txt", rewritten_entry_one.trim()),
        ],
    );
    assert!(
        rewritten_tracks_blocker,
        "The rewritten detached entry should contain the path that blocks checkout"
    );

    let (_, branch_count) = run_git(
        &repo_path,
        &["rev-list", "--count", &format!("main..{stack_branch}")],
    );
    assert_eq!(
        branch_count.trim(),
        "2",
        "The redundant entry should be dropped"
    );
    let (_, branch_tip) = run_git(&repo_path, &["rev-parse", stack_branch]);
    let (_, branch_tip_message) = run_git(
        &repo_path,
        &["show", "-s", "--format=%B", branch_tip.trim()],
    );
    assert!(branch_tip_message.contains("GG-ID: c-ccc1003"));
    let (tip_tracks_blocker, _) = run_git(
        &repo_path,
        &[
            "cat-file",
            "-e",
            &format!("{}:blocked.txt", branch_tip.trim()),
        ],
    );
    assert!(
        !tip_tracks_blocker,
        "The rewritten branch tip must delete the blocking path"
    );
    assert!(
        repo_path.join(".git/gg/smudge-ran").exists(),
        "The smudge filter should create the checkout blocker during rebase"
    );
    let blocker_path = repo_path.join("blocked.txt");
    let mut blocker_permissions = fs::metadata(&blocker_path).unwrap().permissions();
    assert_eq!(
        blocker_permissions.mode() & 0o777,
        0,
        "The ignored directory should deny removal during checkout"
    );
    blocker_permissions.set_mode(0o700);
    fs::set_permissions(&blocker_path, blocker_permissions).unwrap();
    assert_eq!(
        fs::read_to_string(blocker_path.join("obstruction")).unwrap(),
        "obstruction\n",
        "The ignored untracked directory should be the final checkout blocker"
    );
    let (_, ignored_status) = run_git(
        &repo_path,
        &["status", "--ignored", "--porcelain", "--", "blocked.txt"],
    );
    assert_eq!(ignored_status, "!! blocked.txt/\n");

    let (attached, _) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert!(!attached, "Cleanup should leave HEAD detached");
    let (_, head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(
        head.trim(),
        original_entry_one.trim(),
        "Best-effort cleanup should fall back to the exact original OID"
    );

    assert!(
        !success,
        "Lint must fail when final restoration cannot check out the rewritten detached entry: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stderr.contains("Permission denied")
            && stdout.contains("Could not restore the rewritten position"),
        "The failure should come from the blocked final checkout: stdout={stdout}, stderr={stderr}"
    );
}

#[cfg(unix)]
#[test]
fn test_lint_until_handles_dropped_requested_detached_entry() {
    let (_temp_dir, repo_path) =
        create_detached_restore_repo("[ -f file.txt ] && printf 'entry 2\\n' > file.txt");

    let stack_branch = "testuser/lint-dropped-requested-entry";
    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-dropped-requested-entry"]);
    assert!(success, "Failed to create stack: {stderr}");

    fs::write(repo_path.join("file.txt"), "entry 1\n").expect("Failed to write entry 1");
    run_git(&repo_path, &["add", "file.txt"]);
    run_git(&repo_path, &["commit", "-m", "Entry 1\n\nGG-ID: c-ddd2001"]);
    fs::write(repo_path.join("file.txt"), "entry 2\n").expect("Failed to write entry 2");
    run_git(&repo_path, &["add", "file.txt"]);
    run_git(&repo_path, &["commit", "-m", "Entry 2\n\nGG-ID: c-ddd2002"]);

    let (success, _, stderr) = run_gg(&repo_path, &["mv", "2"]);
    assert!(success, "Failed to detach at entry 2: {stderr}");
    let (_, original_entry_two) = run_git(&repo_path, &["rev-parse", "HEAD"]);

    let output = Command::new(env!("CARGO_BIN_EXE_gg"))
        .args(["lint", "--until", "2"])
        .current_dir(&repo_path)
        .env("HOME", repo_path.join(".test-home"))
        .output()
        .expect("Failed to run gg lint");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_ne!(
        output.status.code(),
        Some(101),
        "Lint must not panic after rebase drops the requested entry: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        output.status.success(),
        "Lint should succeed after processing the remaining entry: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("Ran on 1 commit(s) — all passed"),
        "Lint should report only the surviving processed entry: stdout={stdout}"
    );

    let (_, stack_count) = run_git(
        &repo_path,
        &["rev-list", "--count", &format!("main..{stack_branch}")],
    );
    assert_eq!(stack_count.trim(), "1");
    let (_, stack_tip) = run_git(&repo_path, &["rev-parse", stack_branch]);
    let (_, stack_tip_message) =
        run_git(&repo_path, &["show", "-s", "--format=%B", stack_tip.trim()]);
    assert!(stack_tip_message.contains("GG-ID: c-ddd2001"));
    assert!(!stack_tip_message.contains("GG-ID: c-ddd2002"));
    let (_, linted_file) = run_git(
        &repo_path,
        &["show", &format!("{}:file.txt", stack_tip.trim())],
    );
    assert_eq!(linted_file, "entry 2\n");

    let (attached, _) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert!(!attached, "HEAD should remain detached");
    let (_, head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(
        head.trim(),
        original_entry_two.trim(),
        "A dropped detached entry must be restored to its exact original OID"
    );
}

#[cfg(unix)]
#[test]
fn test_failed_lint_refresh_preserves_dropped_detached_entry_with_duplicate_base_gg_id() {
    let (_temp_dir, repo_path) = create_detached_restore_repo(
        "[ -f file.txt ] && printf 'entry 2\\n' > file.txt && git branch -D main",
    );

    run_git(
        &repo_path,
        &[
            "commit",
            "--amend",
            "-m",
            "Add lint script\n\nGG-ID: c-abc0002",
        ],
    );
    let (_, base_oid) = run_git(&repo_path, &["rev-parse", "main"]);

    let stack_branch = "testuser/lint-dropped-detached-refresh-error";
    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-dropped-detached-refresh-error"]);
    assert!(success, "Failed to create stack: {stderr}");

    fs::write(repo_path.join("file.txt"), "entry 1\n").expect("Failed to write entry 1");
    run_git(&repo_path, &["add", "file.txt"]);
    run_git(&repo_path, &["commit", "-m", "Entry 1\n\nGG-ID: c-abc0001"]);
    fs::write(repo_path.join("file.txt"), "entry 2\n").expect("Failed to write entry 2");
    run_git(&repo_path, &["add", "file.txt"]);
    run_git(&repo_path, &["commit", "-m", "Entry 2\n\nGG-ID: c-abc0002"]);

    let (success, _, stderr) = run_gg(&repo_path, &["mv", "2"]);
    assert!(success, "Failed to detach at entry 2: {stderr}");
    let (_, original_entry_two) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_ne!(original_entry_two.trim(), base_oid.trim());

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "1"]);
    assert!(
        !success,
        "Lint should fail when the base disappears before refresh: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        format!("{stdout}{stderr}").contains("Could not find base branch"),
        "Expected canonical refresh failure: stdout={stdout}, stderr={stderr}"
    );
    let (base_exists, _) = run_git(&repo_path, &["rev-parse", "--verify", "refs/heads/main"]);
    assert!(!base_exists, "Lint command should remove the base branch");

    let (_, stack_count) = run_git(
        &repo_path,
        &[
            "rev-list",
            "--count",
            &format!("{}..{stack_branch}", base_oid.trim()),
        ],
    );
    assert_eq!(
        stack_count.trim(),
        "1",
        "The now-empty descendant should be dropped during rebase"
    );
    let (_, stack_history) = run_git(
        &repo_path,
        &[
            "log",
            "--format=%B%x00",
            &format!("{}..{stack_branch}", base_oid.trim()),
        ],
    );
    assert!(stack_history.contains("GG-ID: c-abc0001"));
    assert!(!stack_history.contains("GG-ID: c-abc0002"));

    let (attached, _) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert!(!attached, "HEAD should remain detached after refresh fails");
    let (_, head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(
        head.trim(),
        original_entry_two.trim(),
        "A dropped detached entry must fall back to its original OID: base={}, stdout={stdout}, stderr={stderr}",
        base_oid.trim()
    );
    assert_ne!(
        head.trim(),
        base_oid.trim(),
        "The duplicate base GG-ID must not be selected"
    );
}

#[cfg(unix)]
#[test]
fn test_lint_until_restores_rewritten_detached_last_entry() {
    let (_temp_dir, repo_path) =
        create_detached_restore_repo("[ -f file2.txt ] && echo linted >> file2.txt");

    let stack_branch = "testuser/lint-detached-last";
    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-detached-last"]);
    assert!(success, "Failed to create stack: {stderr}");
    for position in 1..=2 {
        fs::write(
            repo_path.join(format!("file{position}.txt")),
            format!("entry {position}\n"),
        )
        .expect("Failed to write file");
        run_git(&repo_path, &["add", "."]);
        run_git(
            &repo_path,
            &[
                "commit",
                "-m",
                &format!("Entry {position}\n\nGG-ID: c-ddd000{position}"),
            ],
        );
    }

    let (success, _, stderr) = run_gg(&repo_path, &["mv", "2"]);
    assert!(success, "Failed to detach at entry 2: {stderr}");
    let (_, original_head) = run_git(&repo_path, &["rev-parse", "HEAD"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "2"]);
    assert!(success, "Lint should succeed: {stdout} {stderr}");

    let (_, rewritten_entry) = run_git(&repo_path, &["rev-parse", stack_branch]);
    assert_ne!(rewritten_entry.trim(), original_head.trim());
    let (_, head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), rewritten_entry.trim());
    let (attached, _) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert!(
        !attached,
        "HEAD should remain detached at rewritten entry 2"
    );
}

#[cfg(unix)]
#[test]
fn test_lint_until_preserves_detached_head_without_gg_id() {
    let (_temp_dir, repo_path) = create_detached_restore_repo("echo linted >> file1.txt");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-no-id"]);
    assert!(success, "Failed to create stack: {stderr}");
    fs::write(repo_path.join("file1.txt"), "one\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "file1.txt"]);
    run_git(&repo_path, &["commit", "-m", "Entry without ID"]);
    fs::write(repo_path.join("file2.txt"), "two\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "file2.txt"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Second entry\n\nGG-ID: c-aaa0002"],
    );

    let (success, _, stderr) = run_gg(&repo_path, &["mv", "1"]);
    assert!(success, "Failed to detach at entry 1: {stderr}");
    let (_, original_head) = run_git(&repo_path, &["rev-parse", "HEAD"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "1"]);
    assert!(success, "Lint should succeed: {stdout} {stderr}");

    let (attached, _) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert!(!attached, "HEAD should remain detached without a GG-ID");
    let (_, head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), original_head.trim());
}

#[cfg(unix)]
#[test]
fn test_lint_until_preserves_detached_sibling_with_stale_stack_context() {
    let (_temp_dir, repo_path) = create_detached_restore_repo("echo linted >> file1.txt");

    run_git(&repo_path, &["switch", "-c", "sibling"]);
    fs::write(repo_path.join("sibling.txt"), "sibling\n").expect("Failed to write file");
    run_git(&repo_path, &["add", "sibling.txt"]);
    run_git(&repo_path, &["commit", "-m", "Sibling commit"]);
    let (_, sibling_head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    run_git(&repo_path, &["switch", "main"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-stale-context"]);
    assert!(success, "Failed to create stack: {stderr}");
    for position in 1..=2 {
        fs::write(
            repo_path.join(format!("file{position}.txt")),
            format!("entry {position}\n"),
        )
        .expect("Failed to write file");
        run_git(&repo_path, &["add", "."]);
        run_git(
            &repo_path,
            &[
                "commit",
                "-m",
                &format!("Entry {position}\n\nGG-ID: c-bbb000{position}"),
            ],
        );
    }

    let (success, _, stderr) = run_gg(&repo_path, &["mv", "1"]);
    assert!(
        success,
        "Failed to establish stack navigation context: {stderr}"
    );
    let (success, _) = run_git(&repo_path, &["checkout", "--detach", "sibling"]);
    assert!(success, "Failed to detach at sibling commit");

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "1"]);
    assert!(success, "Lint should succeed: {stdout} {stderr}");

    let (attached, _) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert!(
        !attached,
        "HEAD should remain detached at the sibling commit"
    );
    let (_, head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), sibling_head.trim());
}

#[cfg(unix)]
#[test]
fn test_lint_until_preserves_detached_head_for_duplicate_gg_id() {
    let (_temp_dir, repo_path) =
        create_detached_restore_repo("[ -f file2.txt ] && echo linted >> file2.txt");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-duplicate-id"]);
    assert!(success, "Failed to create stack: {stderr}");
    for position in 1..=2 {
        fs::write(
            repo_path.join(format!("file{position}.txt")),
            format!("entry {position}\n"),
        )
        .expect("Failed to write file");
        run_git(&repo_path, &["add", "."]);
        run_git(
            &repo_path,
            &[
                "commit",
                "-m",
                &format!("Entry {position}\n\nGG-ID: c-ccc0001"),
            ],
        );
    }

    let (success, _, stderr) = run_gg(&repo_path, &["mv", "2"]);
    assert!(success, "Failed to detach at entry 2: {stderr}");
    let (_, original_head) = run_git(&repo_path, &["rev-parse", "HEAD"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "2"]);
    assert!(success, "Lint should succeed: {stdout} {stderr}");

    let (attached, _) = run_git(&repo_path, &["symbolic-ref", "--short", "HEAD"]);
    assert!(
        !attached,
        "HEAD should remain detached for duplicate GG-IDs"
    );
    let (_, head) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), original_head.trim());
}

#[test]
fn test_lint_until_keeps_commits_when_last_in_range_amended() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Lint only touches file2.txt, so only the 2nd commit (the last in range) is amended
    fs::write(
        repo_path.join("lint.sh"),
        "#!/bin/sh\n[ -f file2.txt ] && echo linted >> file2.txt\nexit 0\n",
    )
    .expect("Failed to write lint script");
    Command::new("chmod")
        .args(["+x", "lint.sh"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to chmod lint script");
    run_git(&repo_path, &["add", "lint.sh"]);
    run_git(&repo_path, &["commit", "-m", "Add lint script"]);

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","lint":["./lint.sh"]}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-until-last"]);
    assert!(success, "Failed to create stack: {}", stderr);

    for (n, name) in [(1, "First"), (2, "Second"), (3, "Third")] {
        fs::write(
            repo_path.join(format!("file{n}.txt")),
            format!("content{n}\n"),
        )
        .expect("Failed to write file");
        run_git(&repo_path, &["add", "."]);
        run_git(
            &repo_path,
            &[
                "commit",
                "-m",
                &format!("{name} commit\n\nGG-ID: c-last00{n}"),
            ],
        );
    }

    let (success, stdout, stderr) = run_gg(&repo_path, &["lint", "--until", "2"]);
    assert!(success, "Lint should succeed: {} {}", stdout, stderr);

    let (_, log) = run_git(&repo_path, &["log", "--format=%s", "-3"]);
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        vec!["Third commit", "Second commit", "First commit"],
        "lint --until must not drop commits above the range"
    );
    let (_, file2) = run_git(&repo_path, &["show", "HEAD~1:file2.txt"]);
    assert!(file2.contains("linted"), "Lint fix should be in commit 2");
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

#[test]
fn test_sync_jsonl_streams_lint_progress_before_failure() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github","lint":["./lint-fail.sh"]}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-sync-jsonl"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(
        repo_path.join("lint-fail.sh"),
        "#!/bin/sh\necho 'lint-says-no' >&2\nexit 1\n",
    )
    .expect("Failed to write lint script");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(repo_path.join("lint-fail.sh"))
            .unwrap()
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(repo_path.join("lint-fail.sh"), perms).unwrap();
    }
    run_git(&repo_path, &["add", "lint-fail.sh"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Stack commit\n\nGG-ID: c-jsonl01"],
    );

    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).unwrap();
    fs::write(
        fake_bin.join("gh"),
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'gh version 2.0.0'\n  exit 0\nfi\nif [ \"$1\" = \"auth\" ] && [ \"$2\" = \"status\" ]; then\n  exit 0\nfi\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(fake_bin.join("gh")).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(fake_bin.join("gh"), perms).unwrap();
    }
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = std::ffi::OsString::from(fake_bin.as_os_str());
    new_path.push(":");
    new_path.push(old_path);

    let (success, stdout, _stderr) = run_gg_with_env(
        &repo_path,
        &["sync", "--lint", "--jsonl"],
        &[("PATH", new_path.as_os_str())],
    );
    assert!(!success, "sync should fail when lint fails");

    let events: Vec<Value> = stdout
        .lines()
        .filter(|l| l.trim_start().starts_with('{'))
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let names: Vec<&str> = events.iter().filter_map(|e| e["event"].as_str()).collect();
    assert_eq!(
        &names[..5],
        &[
            "start",
            "lint_started",
            "lint_entry_started",
            "lint_command_done",
            "lint_entry_done"
        ],
        "unexpected events: {:?}",
        names
    );
    assert!(names.contains(&"lint_done"));
    assert!(!names.contains(&"entry_started"), "push must not start");

    assert_eq!(events[0]["total_entries"], 1);
    assert_eq!(events[1]["commands"][0], "./lint-fail.sh");
    assert_eq!(events[3]["passed"], false);
    assert!(events[3]["output"]
        .as_str()
        .unwrap()
        .contains("lint-says-no"));
    assert!(events[3]["duration_ms"].is_u64());
    assert_eq!(events[4]["amended"], false);
    let done = events.iter().find(|e| e["event"] == "lint_done").unwrap();
    assert_eq!(done["all_passed"], false);
}

#[cfg(unix)]
#[test]
fn test_sync_jsonl_closed_stream_restores_failed_lint_changes() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","provider":"github","lint":["./lint-fail.sh"]}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "lint-closed-stream"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("tracked.txt"), "original\n").expect("Failed to write tracked file");
    fs::write(
        repo_path.join("lint-fail.sh"),
        "#!/bin/sh\nprintf 'changed by failed lint\\n' > tracked.txt\nsleep 1\nexit 1\n",
    )
    .expect("Failed to write lint script");
    let mut perms = fs::metadata(repo_path.join("lint-fail.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("lint-fail.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "lint-fail.sh", "tracked.txt"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Stack commit\n\nGG-ID: c-closed01"],
    );

    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).unwrap();
    fs::write(
        fake_bin.join("gh"),
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'gh version 2.0.0'\n  exit 0\nfi\nif [ \"$1\" = \"auth\" ] && [ \"$2\" = \"status\" ]; then\n  exit 0\nfi\nexit 1\n",
    )
    .unwrap();
    let mut perms = fs::metadata(fake_bin.join("gh")).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(fake_bin.join("gh"), perms).unwrap();
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = std::ffi::OsString::from(fake_bin.as_os_str());
    new_path.push(":");
    new_path.push(old_path);

    let original_branch = run_git_full(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .1
        .trim()
        .to_string();
    let original_head = run_git_full(&repo_path, &["rev-parse", "HEAD"])
        .1
        .trim()
        .to_string();
    let original_refs = run_git_full(&repo_path, &["show-ref"]).1;

    let gg_path = env!("CARGO_BIN_EXE_gg");
    let mut child = Command::new(gg_path)
        .args(["sync", "--lint", "--jsonl"])
        .current_dir(&repo_path)
        .env("PATH", &new_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn gg sync");

    let stdout = child.stdout.take().expect("stdout must be piped");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .expect("Failed to read JSONL event");
        assert!(bytes > 0, "gg sync ended before lint started");
        let event: Value = serde_json::from_str(line.trim()).expect("stdout must contain JSONL");
        if event["event"] == "lint_entry_started" {
            break;
        }
    }
    drop(reader);

    let output = child
        .wait_with_output()
        .expect("Failed to wait for gg sync");
    assert!(
        !output.status.success(),
        "sync should abort after the stream closes"
    );

    let current_branch = run_git_full(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .1
        .trim()
        .to_string();
    let current_head = run_git_full(&repo_path, &["rev-parse", "HEAD"])
        .1
        .trim()
        .to_string();
    let current_refs = run_git_full(&repo_path, &["show-ref"]).1;

    assert_eq!(
        current_branch, original_branch,
        "sync must restore the original branch"
    );
    assert_eq!(
        current_head, original_head,
        "sync must restore the original HEAD"
    );
    assert_eq!(
        current_refs, original_refs,
        "sync must not move local or remote refs"
    );
}
