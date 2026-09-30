use crate::helpers::{
    create_test_repo, create_test_repo_with_remote, run_gg, run_gg_with_env, run_git,
};

use std::fs;
use std::path::Path;

#[test]
fn test_gg_checkout_creates_branch() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up a config file with username since glab isn't available in tests
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a new stack
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "my-feature"]);

    if !success {
        println!("stdout: {}", stdout);
        println!("stderr: {}", stderr);
    }

    assert!(success, "Failed to create stack: {}", stderr);
    assert!(stdout.contains("Created stack") || stdout.contains("my-feature"));

    // Verify we're on the new branch
    let (_, branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(branch.trim(), "testuser/my-feature");
}

#[test]
fn test_gg_checkout_switch_existing() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "stack1"]);
    assert!(success, "Failed to create stack1: {}", stderr);

    // Go back to main
    run_git(&repo_path, &["checkout", "main"]);

    // Create another stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "stack2"]);
    assert!(success, "Failed to create stack2: {}", stderr);

    // Switch back to stack1
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "stack1"]);
    assert!(success, "Failed to switch to stack1: {}", stderr);
    assert!(stdout.contains("Switched") || stdout.contains("stack1"));

    // Verify we're on stack1
    let (_, branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(branch.trim(), "testuser/stack1");
}

#[test]
fn test_gg_checkout_remote_stack() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack and push it
    run_gg(&repo_path, &["co", "remote-checkout-test"]);

    fs::write(repo_path.join("test.txt"), "test content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit for remote"]);

    // Push the branch to remote
    run_git(
        &repo_path,
        &["push", "-u", "origin", "testuser/remote-checkout-test"],
    );

    // Switch back to main and delete local stack branch
    run_git(&repo_path, &["checkout", "main"]);
    run_git(
        &repo_path,
        &["branch", "-D", "testuser/remote-checkout-test"],
    );

    // Verify we're on main and the stack branch doesn't exist locally
    let (_, current_branch) = run_git(&repo_path, &["branch", "--show-current"]);
    assert!(
        current_branch.trim() == "main",
        "Should be on main: {}",
        current_branch
    );

    // Now checkout the remote stack
    let (success, stdout, _stderr) = run_gg(&repo_path, &["co", "remote-checkout-test"]);
    assert!(success, "Should successfully checkout remote stack");
    assert!(
        stdout.contains("Checked out remote stack") || stdout.contains("remote-checkout-test"),
        "Should mention checking out remote: {}",
        stdout
    );

    // Verify we're now on the stack branch
    let (_, current_branch) = run_git(&repo_path, &["branch", "--show-current"]);
    assert!(
        current_branch.contains("remote-checkout-test"),
        "Should be on the stack branch: {}",
        current_branch
    );

    // Verify the file exists
    assert!(
        repo_path.join("test.txt").exists(),
        "test.txt should exist after checkout"
    );
}

fn checked_git(repo: &Path, args: &[&str]) -> String {
    let (success, output) = run_git(repo, args);
    assert!(success, "git {args:?} failed: {output}");
    output.trim().to_string()
}

fn checkout_state(repo: &Path) -> (String, String, Vec<u8>, String, String) {
    (
        checked_git(repo, &["rev-parse", "HEAD"]),
        checked_git(repo, &["branch", "--show-current"]),
        fs::read(repo.join(".git/gg/config.json")).unwrap(),
        checked_git(
            repo,
            &[
                "for-each-ref",
                "--format=%(refname):%(objectname)",
                "refs/heads",
            ],
        ),
        checked_git(repo, &["worktree", "list", "--porcelain"]),
    )
}

fn remote_entry_stack() -> (tempfile::TempDir, std::path::PathBuf, Vec<String>) {
    let (temp, writer, remote) = create_test_repo_with_remote();
    checked_git(&writer, &["checkout", "-b", "base-side"]);
    checked_git(
        &writer,
        &["commit", "--allow-empty", "-m", "Base side commit"],
    );
    checked_git(&writer, &["checkout", "main"]);
    checked_git(
        &writer,
        &["commit", "--allow-empty", "-m", "Base main commit"],
    );
    checked_git(
        &writer,
        &["merge", "--no-ff", "base-side", "-m", "Base merge"],
    );
    checked_git(&writer, &["push", "origin", "main"]);
    checked_git(&writer, &["checkout", "-b", "writer"]);
    let mut commits = Vec::new();
    // The bottom sorts first and the tip sorts in the middle. Neither lexical
    // endpoint identifies the tip, and GG-IDs do not encode commit order.
    for id in ["c-1111111", "c-9999999", "c-5555555"] {
        checked_git(
            &writer,
            &[
                "commit",
                "--allow-empty",
                "-m",
                &format!("Entry {id}\n\nGG-ID: {id}"),
            ],
        );
        commits.push(checked_git(&writer, &["rev-parse", "HEAD"]));
        checked_git(
            &writer,
            &[
                "push",
                "origin",
                &format!("HEAD:refs/heads/testuser/remote-tip--{id}"),
            ],
        );
    }
    let reader = temp.path().join("reader");
    checked_git(
        &writer,
        &["clone", remote.to_str().unwrap(), reader.to_str().unwrap()],
    );
    let gg_dir = reader.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
    )
    .unwrap();
    (temp, reader, commits)
}

#[test]
fn test_gg_checkout_remote_entries_selects_linear_tip_above_merged_base() {
    let (_temp, reader, commits) = remote_entry_stack();
    let (success, stdout, stderr) = run_gg(&reader, &["co", "remote-tip"]);
    assert!(success, "stdout: {stdout}\nstderr: {stderr}");
    let head = checked_git(&reader, &["rev-parse", "HEAD"]);
    let count = checked_git(&reader, &["rev-list", "--count", "main..HEAD"]);
    assert_eq!(
        (head.as_str(), count.as_str()),
        (commits[2].as_str(), "3"),
        "checkout must restore all entries, not the first sorted ref at {}",
        commits[0]
    );
    assert_eq!(
        checked_git(&reader, &["branch", "--show-current"]),
        "testuser/remote-tip"
    );
}

#[test]
fn test_gg_checkout_remote_entries_uses_explicit_cli_base() {
    let (temp, writer, remote) = create_test_repo_with_remote();
    checked_git(&writer, &["checkout", "-b", "develop-side"]);
    checked_git(&writer, &["commit", "--allow-empty", "-m", "Develop side"]);
    checked_git(&writer, &["checkout", "-b", "develop", "main"]);
    checked_git(
        &writer,
        &["commit", "--allow-empty", "-m", "Develop mainline"],
    );
    checked_git(
        &writer,
        &["merge", "--no-ff", "develop-side", "-m", "Develop merge"],
    );
    checked_git(&writer, &["push", "origin", "develop"]);
    checked_git(&writer, &["checkout", "-b", "writer"]);
    checked_git(
        &writer,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-1111111"],
    );
    let tip = checked_git(&writer, &["rev-parse", "HEAD"]);
    checked_git(
        &writer,
        &[
            "push",
            "origin",
            "HEAD:refs/heads/testuser/develop-stack--c-1111111",
        ],
    );

    let reader = temp.path().join("develop-reader");
    checked_git(
        &writer,
        &["clone", remote.to_str().unwrap(), reader.to_str().unwrap()],
    );
    fs::create_dir_all(reader.join(".git/gg")).unwrap();
    fs::write(
        reader.join(".git/gg/config.json"),
        r#"{
            "defaults":{"branch_username":"testuser","base":"main"},
            "stacks":{"develop-stack":{"mrs":{"c-existing":42},"worktree_path":"/preserved"}}
        }"#,
    )
    .unwrap();

    let (success, stdout, stderr) = run_gg(&reader, &["co", "develop-stack", "--base", "develop"]);
    assert!(success, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("Could not import PR/MR mappings"),
        "provider-unavailable checkout should continue: {stdout}"
    );
    assert_eq!(checked_git(&reader, &["rev-parse", "HEAD"]), tip);
    assert_eq!(
        checked_git(&reader, &["rev-list", "--count", "origin/develop..HEAD"]),
        "1"
    );

    let config: serde_json::Value = serde_json::from_slice(
        &fs::read(reader.join(".git/gg/config.json")).expect("read checkout config"),
    )
    .expect("parse checkout config");
    assert_eq!(config["stacks"]["develop-stack"]["base"], "develop");
    assert_eq!(config["stacks"]["develop-stack"]["mrs"]["c-existing"], 42);
    assert_eq!(
        config["stacks"]["develop-stack"]["worktree_path"],
        "/preserved"
    );

    let (success, stdout, stderr) = run_gg(&reader, &["log", "--json"]);
    assert!(success, "stack should remain loadable: {stdout}\n{stderr}");
    let log: serde_json::Value = serde_json::from_str(&stdout).expect("parse gg log output");
    assert_eq!(log["log"]["base"], "develop");
    assert_eq!(log["log"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(log["log"]["entries"][0]["gg_id"], "c-1111111");
}

#[test]
fn test_gg_checkout_remote_entries_rejects_diverged_refs_without_creating_stack() {
    let (temp, reader, commits) = remote_entry_stack();
    let writer = temp.path().join("repo");
    checked_git(&writer, &["checkout", "--detach", &commits[0]]);
    checked_git(
        &writer,
        &["commit", "--allow-empty", "-m", "Diverged entry"],
    );
    checked_git(
        &writer,
        &[
            "push",
            "origin",
            "HEAD:refs/heads/testuser/remote-tip--c-0000000",
        ],
    );
    let before = checked_git(&reader, &["rev-parse", "HEAD"]);
    let (success, stdout, stderr) = run_gg(&reader, &["co", "remote-tip"]);
    assert!(!success, "ambiguous checkout must fail: {stdout}\n{stderr}");
    assert!(stderr.contains("diverged"), "{stderr}");
    assert_eq!(checked_git(&reader, &["rev-parse", "HEAD"]), before);
    assert_eq!(checked_git(&reader, &["branch", "--show-current"]), "main");
    assert!(
        !run_git(
            &reader,
            &["show-ref", "--verify", "refs/heads/testuser/remote-tip"]
        )
        .0
    );
}

#[test]
fn test_gg_checkout_remote_entries_accepts_duplicate_tip_refs_and_ignores_other_stacks() {
    let (temp, reader, commits) = remote_entry_stack();
    let writer = temp.path().join("repo");
    checked_git(
        &writer,
        &[
            "push",
            "origin",
            "HEAD:refs/heads/testuser/remote-tip--c-0000000",
        ],
    );
    checked_git(
        &writer,
        &["commit", "--allow-empty", "-m", "Unrelated entry"],
    );
    for branch in [
        "testuser/remote-tip--backup",
        "anotheruser/remote-tip--c-0000000",
        "testuser/remote-tip-other--c-0000000",
        // Slash-separated legacy names are not entry branches in the current
        // parser. Do not accidentally include them via a broad prefix match.
        "testuser/remote-tip/c-0000000",
    ] {
        checked_git(
            &writer,
            &["push", "origin", &format!("HEAD:refs/heads/{branch}")],
        );
    }
    let (success, stdout, stderr) = run_gg(&reader, &["co", "remote-tip"]);
    assert!(success, "{stdout}\n{stderr}");
    assert_eq!(checked_git(&reader, &["rev-parse", "HEAD"]), commits[2]);
}

#[test]
fn test_gg_checkout_ignores_invalid_only_remote_entry_when_creating_stack() {
    let (_temp, repo, _remote) = create_test_repo_with_remote();
    fs::create_dir_all(repo.join(".git/gg")).unwrap();
    fs::write(
        repo.join(".git/gg/config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
    )
    .unwrap();
    checked_git(
        &repo,
        &[
            "push",
            "origin",
            "HEAD:refs/heads/testuser/only-invalid--backup",
        ],
    );

    let main = checked_git(&repo, &["rev-parse", "main"]);
    let (success, stdout, stderr) = run_gg(&repo, &["co", "only-invalid"]);
    assert!(success, "{stdout}\n{stderr}");
    assert!(stdout.contains("Created stack"), "{stdout}");
    assert_eq!(
        checked_git(&repo, &["branch", "--show-current"]),
        "testuser/only-invalid"
    );
    assert_eq!(checked_git(&repo, &["rev-parse", "HEAD"]), main);

    checked_git(
        &repo,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "Usable entry\n\nGG-ID: c-1234567",
        ],
    );
    let (success, stdout, stderr) = run_gg(&repo, &["log", "--json"]);
    assert!(success, "new stack should be usable: {stdout}\n{stderr}");
}

#[test]
fn test_gg_checkout_remote_entries_accepts_single_entry() {
    let (temp, reader, commits) = remote_entry_stack();
    checked_git(
        &temp.path().join("repo"),
        &[
            "push",
            "origin",
            "--delete",
            "testuser/remote-tip--c-1111111",
            "testuser/remote-tip--c-9999999",
        ],
    );
    let (success, stdout, stderr) = run_gg(&reader, &["co", "remote-tip"]);
    assert!(success, "{stdout}\n{stderr}");
    assert_eq!(checked_git(&reader, &["rev-parse", "HEAD"]), commits[2]);
}

#[test]
fn test_gg_checkout_remote_stack_branch_keeps_precedence_over_entries() {
    let (temp, reader, commits) = remote_entry_stack();
    checked_git(
        &temp.path().join("repo"),
        &[
            "push",
            "origin",
            &format!("{}:refs/heads/testuser/remote-tip", commits[0]),
        ],
    );
    let (success, stdout, stderr) = run_gg(&reader, &["co", "remote-tip", "--base", "main"]);
    assert!(success, "{stdout}\n{stderr}");
    assert_eq!(checked_git(&reader, &["rev-parse", "HEAD"]), commits[0]);
}

#[test]
fn test_gg_checkout_remote_stack_branch_rejects_missing_explicit_base_before_mutation() {
    let (_temp, repo, _remote) = create_test_repo_with_remote();
    fs::create_dir_all(repo.join(".git/gg")).unwrap();
    fs::write(
        repo.join(".git/gg/config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
    )
    .unwrap();
    checked_git(
        &repo,
        &["commit", "--allow-empty", "-m", "Remote stack entry"],
    );
    checked_git(
        &repo,
        &["push", "origin", "HEAD:refs/heads/testuser/missing-base"],
    );
    checked_git(&repo, &["checkout", "main"]);

    let before = checkout_state(&repo);
    let (success, stdout, stderr) =
        run_gg(&repo, &["co", "missing-base", "--base", "does-not-exist"]);
    assert!(!success, "checkout must fail: {stdout}\n{stderr}");
    assert!(stderr.contains("Could not find base branch"), "{stderr}");
    assert_eq!(checkout_state(&repo), before);
}

#[test]
fn test_gg_checkout_remote_stack_branch_rejects_unrelated_base_before_worktree_mutation() {
    let (_temp, repo, _remote) = create_test_repo_with_remote();
    fs::create_dir_all(repo.join(".git/gg")).unwrap();
    fs::write(
        repo.join(".git/gg/config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
    )
    .unwrap();
    checked_git(
        &repo,
        &["commit", "--allow-empty", "-m", "Remote stack entry"],
    );
    checked_git(
        &repo,
        &["push", "origin", "HEAD:refs/heads/testuser/unrelated-base"],
    );
    checked_git(&repo, &["checkout", "--orphan", "unrelated"]);
    checked_git(&repo, &["commit", "--allow-empty", "-m", "Unrelated root"]);
    checked_git(&repo, &["checkout", "main"]);

    let before = checkout_state(&repo);
    let (success, stdout, stderr) = run_gg(
        &repo,
        &["co", "unrelated-base", "--base", "unrelated", "--worktree"],
    );
    assert!(!success, "checkout must fail: {stdout}\n{stderr}");
    assert!(stderr.contains("shares no history"), "{stderr}");
    assert_eq!(checkout_state(&repo), before);
}

#[test]
fn test_gg_checkout_remote_entries_accepts_explicit_base_that_advanced() {
    let (temp, writer, remote) = create_test_repo_with_remote();
    checked_git(&writer, &["checkout", "-b", "behind"]);
    checked_git(
        &writer,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-1234567"],
    );
    checked_git(
        &writer,
        &[
            "push",
            "origin",
            "HEAD:refs/heads/testuser/behind--c-1234567",
        ],
    );
    checked_git(&writer, &["checkout", "main"]);
    checked_git(&writer, &["commit", "--allow-empty", "-m", "Base advanced"]);
    checked_git(&writer, &["push", "origin", "main"]);

    let reader = temp.path().join("behind-reader");
    checked_git(
        &writer,
        &["clone", remote.to_str().unwrap(), reader.to_str().unwrap()],
    );
    fs::create_dir_all(reader.join(".git/gg")).unwrap();
    fs::write(
        reader.join(".git/gg/config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
    )
    .unwrap();

    let (success, stdout, stderr) = run_gg(&reader, &["co", "behind", "--base", "main"]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");
    let (success, stdout, stderr) = run_gg(&reader, &["log", "--json"]);
    assert!(success, "stack should remain loadable: {stdout}\n{stderr}");
    let log: serde_json::Value = serde_json::from_str(&stdout).expect("parse gg log output");
    assert_eq!(log["log"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(log["log"]["entries"][0]["gg_id"], "c-1234567");
}

#[test]
fn test_gg_checkout_remote_stack_branch_accepts_merged_nondefault_base() {
    let (temp, writer, remote) = create_test_repo_with_remote();
    checked_git(&writer, &["checkout", "-b", "develop-side"]);
    checked_git(&writer, &["commit", "--allow-empty", "-m", "Develop side"]);
    checked_git(&writer, &["checkout", "-b", "develop", "main"]);
    checked_git(
        &writer,
        &["commit", "--allow-empty", "-m", "Develop mainline"],
    );
    checked_git(
        &writer,
        &["merge", "--no-ff", "develop-side", "-m", "Develop merge"],
    );
    checked_git(&writer, &["push", "origin", "develop"]);
    checked_git(&writer, &["checkout", "-b", "explicit-valid"]);
    checked_git(
        &writer,
        &["commit", "--allow-empty", "-m", "Entry\n\nGG-ID: c-7654321"],
    );
    checked_git(
        &writer,
        &["push", "origin", "HEAD:refs/heads/testuser/explicit-valid"],
    );

    let reader = temp.path().join("explicit-valid-reader");
    checked_git(
        &writer,
        &["clone", remote.to_str().unwrap(), reader.to_str().unwrap()],
    );
    fs::create_dir_all(reader.join(".git/gg")).unwrap();
    fs::write(
        reader.join(".git/gg/config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
    )
    .unwrap();

    let (success, stdout, stderr) = run_gg(&reader, &["co", "explicit-valid", "--base", "develop"]);
    assert!(success, "checkout failed: {stdout}\n{stderr}");
    let (success, stdout, stderr) = run_gg(&reader, &["log", "--json"]);
    assert!(success, "stack should remain loadable: {stdout}\n{stderr}");
    let log: serde_json::Value = serde_json::from_str(&stdout).expect("parse gg log output");
    assert_eq!(log["log"]["base"], "develop");
    assert_eq!(log["log"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(log["log"]["entries"][0]["gg_id"], "c-7654321");
}

#[test]
fn test_gg_checkout_remote_entries_rejects_merge_joining_diverged_entries() {
    let (temp, reader, commits) = remote_entry_stack();
    let writer = temp.path().join("repo");
    checked_git(&writer, &["checkout", "--detach", &commits[0]]);
    checked_git(&writer, &["commit", "--allow-empty", "-m", "Side entry"]);
    checked_git(
        &writer,
        &[
            "push",
            "origin",
            "HEAD:refs/heads/testuser/remote-tip--c-0000000",
        ],
    );
    checked_git(
        &writer,
        &["merge", "--no-ff", &commits[2], "-m", "Common descendant"],
    );
    checked_git(
        &writer,
        &[
            "push",
            "origin",
            "HEAD:refs/heads/testuser/remote-tip--c-fffffff",
        ],
    );
    let before = checked_git(&reader, &["rev-parse", "HEAD"]);
    let config_before = fs::read(reader.join(".git/gg/config.json")).unwrap();
    let (success, stdout, stderr) = run_gg(&reader, &["co", "remote-tip", "--base", "main"]);
    assert!(
        !success,
        "merge-backed checkout must fail: {stdout}\n{stderr}"
    );
    assert!(stderr.contains("have diverged"), "{stderr}");
    assert!(stderr.contains("Resolve the remote branches"), "{stderr}");
    assert_eq!(checked_git(&reader, &["rev-parse", "HEAD"]), before);
    assert_eq!(checked_git(&reader, &["branch", "--show-current"]), "main");
    assert_eq!(
        fs::read(reader.join(".git/gg/config.json")).unwrap(),
        config_before
    );
    assert!(
        !run_git(
            &reader,
            &["show-ref", "--verify", "refs/heads/testuser/remote-tip"]
        )
        .0
    );
}

// ============================================================
// Tests for PRs #44-#48 (merged while claude-review was broken)
// ============================================================

#[test]
fn test_gg_checkout_with_worktree_creates_worktree_and_preserves_main_repo_head() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "wt-stack", "--worktree"]);
    assert!(
        success,
        "checkout --worktree should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    // Main repo should remain on its original branch (no branch checkout in primary worktree)
    let (_, current_branch) = run_git(&repo_path, &["branch", "--show-current"]);
    let branch = current_branch.trim();
    assert!(
        branch == "main" || branch == "master",
        "Expected main or master, got: {}",
        branch
    );

    let config = fs::read_to_string(gg_dir.join("config.json")).expect("Failed to read config");
    assert!(
        config.contains("worktree_path"),
        "Config should persist worktree path"
    );

    let expected_path = repo_path.parent().expect("repo parent").join(format!(
        "{}.{}",
        repo_path.file_name().unwrap().to_string_lossy(),
        "wt-stack"
    ));

    assert!(
        expected_path.exists(),
        "Expected worktree path to exist: {}",
        expected_path.display()
    );
}

#[test]
fn test_gg_checkout_with_worktree_requests_shell_cd() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let cd_file = repo_path.join(".git/gg/cd-target");
    let cd_file_os = cd_file.as_os_str();
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["co", "cd-stack", "--wt"],
        &[("GG_CD_FILE", cd_file_os)],
    );
    assert!(
        success,
        "checkout --wt should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    let cd_target = fs::read_to_string(&cd_file).expect("Failed to read cd target");
    assert!(
        Path::new(&cd_target).exists(),
        "cd target should exist: {}",
        cd_target
    );
    assert!(
        cd_target.ends_with(".cd-stack"),
        "cd target should point to the stack worktree: {}",
        cd_target
    );
}

#[test]
fn test_gg_checkout_existing_worktree_requests_shell_cd() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "existing-cd-stack", "--wt"]);
    assert!(
        success,
        "initial checkout --wt should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    let cd_file = repo_path.join(".git/gg/cd-target-existing");
    let cd_file_os = cd_file.as_os_str();
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["co", "existing-cd-stack", "--wt"],
        &[("GG_CD_FILE", cd_file_os)],
    );
    assert!(
        success,
        "existing checkout --wt should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    let cd_target = fs::read_to_string(&cd_file).expect("Failed to read cd target");
    assert!(
        Path::new(&cd_target).exists(),
        "cd target should exist: {}",
        cd_target
    );
    assert!(
        cd_target.ends_with(".existing-cd-stack"),
        "cd target should point to the stack worktree: {}",
        cd_target
    );
}

#[test]
fn test_gg_checkout_without_worktree_does_not_request_shell_cd() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let cd_file = repo_path.join(".git/gg/cd-target-plain");
    let cd_file_os = cd_file.as_os_str();
    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &["co", "plain-stack"],
        &[("GG_CD_FILE", cd_file_os)],
    );
    assert!(
        success,
        "plain checkout should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(!cd_file.exists(), "plain checkout should not request cd");
}

#[test]
fn test_gg_checkout_failure_does_not_request_shell_cd() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let cd_file = repo_path.join(".git/gg/cd-target-failed");
    let cd_file_os = cd_file.as_os_str();
    let (success, _stdout, _stderr) = run_gg_with_env(
        &repo_path,
        &["co", "bad/stack", "--wt"],
        &[("GG_CD_FILE", cd_file_os)],
    );
    assert!(!success, "invalid stack checkout should fail");
    assert!(!cd_file.exists(), "failed checkout should not request cd");
}
