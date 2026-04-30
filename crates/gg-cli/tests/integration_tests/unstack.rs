use crate::helpers::{create_test_repo, create_test_repo_with_worktree_support, run_gg, run_git};

use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

#[test]
fn test_gg_unstack_worktree_creates_new_stack_worktree_and_preserves_current_branch() {
    let (_temp_dir, repo_path) = create_test_repo_with_worktree_support();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
    )
    .expect("Failed to write config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "lower"]);
    assert!(
        success,
        "checkout should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    for i in 1..=4 {
        fs::write(
            repo_path.join(format!("file{i}.txt")),
            format!("commit {i}\n"),
        )
        .expect("Failed to write test file");
        run_git(&repo_path, &["add", "."]);
        let (success, _) = run_git(&repo_path, &["commit", "-m", &format!("commit {i}")]);
        assert!(success, "commit {i} should succeed");
    }

    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["unstack", "-t", "3", "--name", "upper", "--worktree"],
    );
    assert!(
        success,
        "unstack --worktree should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    let (_, current_branch) = run_git(&repo_path, &["branch", "--show-current"]);
    assert_eq!(current_branch.trim(), "testuser/lower");

    let (_, lower_log) = run_git(&repo_path, &["log", "--format=%s", "main..HEAD"]);
    assert!(lower_log.contains("commit 1"), "lower log: {lower_log}");
    assert!(lower_log.contains("commit 2"), "lower log: {lower_log}");
    assert!(!lower_log.contains("commit 3"), "lower log: {lower_log}");
    assert!(!lower_log.contains("commit 4"), "lower log: {lower_log}");

    let expected_path = repo_path.parent().expect("repo parent").join(format!(
        "{}.{}",
        repo_path.file_name().unwrap().to_string_lossy(),
        "upper"
    ));
    assert!(
        expected_path.exists(),
        "Expected worktree path to exist: {}",
        expected_path.display()
    );

    let (_, worktree_branch) = run_git(&expected_path, &["branch", "--show-current"]);
    assert_eq!(worktree_branch.trim(), "testuser/upper");

    let (_, upper_log) = run_git(&expected_path, &["log", "--format=%s", "main..HEAD"]);
    assert!(!upper_log.contains("commit 1"), "upper log: {upper_log}");
    assert!(!upper_log.contains("commit 2"), "upper log: {upper_log}");
    assert!(upper_log.contains("commit 3"), "upper log: {upper_log}");
    assert!(upper_log.contains("commit 4"), "upper log: {upper_log}");

    let config: Value =
        serde_json::from_str(&fs::read_to_string(gg_dir.join("config.json")).unwrap()).unwrap();
    let worktree_path = config["stacks"]["upper"]["worktree_path"]
        .as_str()
        .expect("upper stack should persist worktree_path");
    assert_eq!(
        PathBuf::from(worktree_path).canonicalize().unwrap(),
        expected_path.canonicalize().unwrap()
    );

    let (success, stdout, stderr) = run_gg(&repo_path, &["ls", "--all"]);
    assert!(success, "ls --all should succeed: {}", stderr);
    assert!(
        stdout.contains("upper"),
        "ls should show upper stack: {stdout}"
    );
    assert!(
        stdout.contains("[wt]"),
        "ls should show worktree marker: {stdout}"
    );
}

#[test]
fn test_gg_unstack_worktree_failure_leaves_original_stack_unchanged() {
    let (_temp_dir, repo_path) = create_test_repo_with_worktree_support();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
    )
    .expect("Failed to write config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "rollback-lower"]);
    assert!(
        success,
        "checkout should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    for i in 1..=3 {
        fs::write(
            repo_path.join(format!("rollback-file{i}.txt")),
            format!("commit {i}\n"),
        )
        .expect("Failed to write test file");
        run_git(&repo_path, &["add", "."]);
        let (success, _) = run_git(
            &repo_path,
            &["commit", "-m", &format!("rollback commit {i}")],
        );
        assert!(success, "commit {i} should succeed");
    }

    let before_log = rev_list(&repo_path, "main..testuser/rollback-lower");
    let blocked_path = repo_path.parent().expect("repo parent").join(format!(
        "{}.{}",
        repo_path.file_name().unwrap().to_string_lossy(),
        "rollback-upper"
    ));
    fs::create_dir_all(&blocked_path).expect("Failed to create blocked worktree path");
    fs::write(blocked_path.join("occupied.txt"), "already here\n")
        .expect("Failed to occupy blocked worktree path");

    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &[
            "unstack",
            "-t",
            "2",
            "--name",
            "rollback-upper",
            "--worktree",
        ],
    );
    assert!(
        !success,
        "unstack --worktree should fail when target path exists: stdout={stdout}"
    );
    assert!(
        stderr.contains("Failed to create worktree"),
        "stderr should report worktree creation failure: {stderr}"
    );

    let after_log = rev_list(&repo_path, "main..testuser/rollback-lower");
    assert_eq!(after_log, before_log);

    let (_, current_branch) = run_git(&repo_path, &["branch", "--show-current"]);
    assert_eq!(current_branch.trim(), "testuser/rollback-lower");

    let (branch_exists, _) = run_git(&repo_path, &["rev-parse", "testuser/rollback-upper"]);
    assert!(
        !branch_exists,
        "temporary new stack branch should be removed after worktree failure"
    );

    let config: Value =
        serde_json::from_str(&fs::read_to_string(gg_dir.join("config.json")).unwrap()).unwrap();
    assert!(
        config["stacks"]["rollback-upper"].is_null(),
        "failed unstack should not persist new stack config: {config}"
    );
}

#[test]
fn test_gg_unstack_wt_alias_preserves_existing_old_stack_worktree_metadata() {
    let (_temp_dir, repo_path) = create_test_repo_with_worktree_support();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
    )
    .expect("Failed to write config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "old-wt", "--worktree"]);
    assert!(
        success,
        "checkout --worktree should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    let old_worktree_path = repo_path.parent().expect("repo parent").join(format!(
        "{}.{}",
        repo_path.file_name().unwrap().to_string_lossy(),
        "old-wt"
    ));

    for i in 1..=3 {
        fs::write(
            old_worktree_path.join(format!("wt-file{i}.txt")),
            format!("commit {i}\n"),
        )
        .expect("Failed to write test file");
        run_git(&old_worktree_path, &["add", "."]);
        let (success, _) = run_git(
            &old_worktree_path,
            &["commit", "-m", &format!("wt commit {i}")],
        );
        assert!(success, "worktree commit {i} should succeed");
    }

    let (success, stdout, stderr) = run_gg(
        &old_worktree_path,
        &["unstack", "-t", "2", "--name", "new-wt", "--wt"],
    );
    assert!(
        success,
        "unstack --wt should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    let (_, current_branch) = run_git(&old_worktree_path, &["branch", "--show-current"]);
    assert_eq!(current_branch.trim(), "testuser/old-wt");

    let config: Value =
        serde_json::from_str(&fs::read_to_string(gg_dir.join("config.json")).unwrap()).unwrap();
    let old_config_worktree_path = config["stacks"]["old-wt"]["worktree_path"]
        .as_str()
        .expect("old stack should keep worktree_path");
    assert_eq!(
        PathBuf::from(old_config_worktree_path)
            .canonicalize()
            .unwrap(),
        old_worktree_path.canonicalize().unwrap()
    );

    let new_worktree_path = config["stacks"]["new-wt"]["worktree_path"]
        .as_str()
        .expect("new stack should persist worktree_path");
    assert_ne!(
        PathBuf::from(new_worktree_path).canonicalize().unwrap(),
        PathBuf::from(old_config_worktree_path)
            .canonicalize()
            .unwrap()
    );
    assert!(
        PathBuf::from(new_worktree_path).exists(),
        "new worktree should exist at {new_worktree_path}"
    );
}

#[test]
fn test_gg_unstack_without_worktree_switches_current_repo_to_new_stack() {
    let (_temp_dir, repo_path) = create_test_repo_with_worktree_support();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser","base":"main"}}"#,
    )
    .expect("Failed to write config");

    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "plain-unstack"]);
    assert!(
        success,
        "checkout should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    for i in 1..=3 {
        fs::write(
            repo_path.join(format!("plain-file{i}.txt")),
            format!("commit {i}\n"),
        )
        .expect("Failed to write test file");
        run_git(&repo_path, &["add", "."]);
        let (success, _) = run_git(&repo_path, &["commit", "-m", &format!("plain commit {i}")]);
        assert!(success, "commit {i} should succeed");
    }

    let (success, stdout, stderr) =
        run_gg(&repo_path, &["unstack", "-t", "2", "--name", "plain-upper"]);
    assert!(
        success,
        "unstack should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    let (_, current_branch) = run_git(&repo_path, &["branch", "--show-current"]);
    assert_eq!(current_branch.trim(), "testuser/plain-upper");

    let (_, upper_log) = run_git(&repo_path, &["log", "--format=%s", "main..HEAD"]);
    assert!(
        !upper_log.contains("plain commit 1"),
        "upper log: {upper_log}"
    );
    assert!(
        upper_log.contains("plain commit 2"),
        "upper log: {upper_log}"
    );
    assert!(
        upper_log.contains("plain commit 3"),
        "upper log: {upper_log}"
    );

    let config: Value =
        serde_json::from_str(&fs::read_to_string(gg_dir.join("config.json")).unwrap()).unwrap();
    assert!(
        config["stacks"]["plain-upper"]["worktree_path"].is_null(),
        "plain unstack should not create worktree metadata: {config}"
    );
}

fn setup_unstack_repo(stack_name: &str, num_commits: usize) -> (TempDir, PathBuf) {
    let (temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let (success, _, stderr) = run_gg(&repo_path, &["co", stack_name]);
    assert!(success, "gg co failed: {stderr}");

    let mut previous_gg_id: Option<String> = None;
    for i in 1..=num_commits {
        let gg_id = format!("c-{i:07}");
        fs::write(
            repo_path.join(format!("unstack-{i}.txt")),
            format!("content {i}\n"),
        )
        .unwrap();
        run_git(&repo_path, &["add", "."]);

        let mut message = format!("Commit {i}\n\nGG-ID: {gg_id}");
        if let Some(parent) = previous_gg_id {
            message.push_str(&format!("\nGG-Parent: {parent}"));
        }
        run_git(&repo_path, &["commit", "-m", &message]);
        previous_gg_id = Some(gg_id);
    }

    (temp_dir, repo_path)
}

fn rev_list(repo_path: &std::path::Path, range: &str) -> Vec<String> {
    let (success, stdout) = run_git(repo_path, &["rev-list", "--reverse", range]);
    assert!(success, "git rev-list failed for {range}");
    stdout.lines().map(str::to_string).collect()
}

fn commit_message(repo_path: &std::path::Path, rev: &str) -> String {
    let (success, stdout) = run_git(repo_path, &["show", "-s", "--format=%B", rev]);
    assert!(success, "git show failed for {rev}");
    stdout
}

#[test]
fn test_unstack_splits_stack_metadata_config_and_entry_branches() {
    let (_temp_dir, repo_path) = setup_unstack_repo("unstack-flow", 4);

    let original_commits = rev_list(&repo_path, "main..testuser/unstack-flow");
    assert_eq!(original_commits.len(), 4);

    run_git(
        &repo_path,
        &[
            "branch",
            "testuser/unstack-flow--c-0000003",
            &original_commits[2],
        ],
    );
    run_git(
        &repo_path,
        &[
            "branch",
            "testuser/unstack-flow--c-0000004",
            &original_commits[3],
        ],
    );
    run_git(
        &repo_path,
        &[
            "branch",
            "testuser/unstack-flow--c-0000002",
            &original_commits[1],
        ],
    );

    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value =
        serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    config["stacks"]["unstack-flow"]["mrs"] = serde_json::json!({
        "c-0000001": 101,
        "c-0000002": 102,
        "c-0000003": 103,
        "c-0000004": 104
    });
    fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &[
            "unstack",
            "--target",
            "3",
            "--name",
            "unstack-flow-top",
            "--no-tui",
        ],
    );
    assert!(
        success,
        "gg unstack failed. stdout={stdout} stderr={stderr}"
    );

    let lower_commits = rev_list(&repo_path, "main..testuser/unstack-flow");
    let upper_commits = rev_list(&repo_path, "main..testuser/unstack-flow-top");
    assert_eq!(lower_commits.len(), 2);
    assert_eq!(upper_commits.len(), 2);

    let lower_root = commit_message(&repo_path, &lower_commits[0]);
    let lower_head = commit_message(&repo_path, &lower_commits[1]);
    let upper_root = commit_message(&repo_path, &upper_commits[0]);
    let upper_head = commit_message(&repo_path, &upper_commits[1]);

    assert!(lower_root.contains("GG-ID: c-0000001"));
    assert!(!lower_root.contains("GG-Parent:"));
    assert!(lower_head.contains("GG-ID: c-0000002"));
    assert!(lower_head.contains("GG-Parent: c-0000001"));

    assert!(upper_root.contains("GG-ID: c-0000003"));
    assert!(
        !upper_root.contains("GG-Parent:"),
        "new stack root must not keep old parent: {upper_root}"
    );
    assert!(upper_head.contains("GG-ID: c-0000004"));
    assert!(upper_head.contains("GG-Parent: c-0000003"));

    let config: Value = serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(config["stacks"]["unstack-flow"]["mrs"]["c-0000001"], 101);
    assert_eq!(config["stacks"]["unstack-flow"]["mrs"]["c-0000002"], 102);
    assert!(config["stacks"]["unstack-flow"]["mrs"]["c-0000003"].is_null());
    assert_eq!(
        config["stacks"]["unstack-flow-top"]["mrs"]["c-0000003"],
        103
    );
    assert_eq!(
        config["stacks"]["unstack-flow-top"]["mrs"]["c-0000004"],
        104
    );

    let (success, branches) = run_git(&repo_path, &["branch", "--list"]);
    assert!(success);
    assert!(!branches.contains("testuser/unstack-flow--c-0000003"));
    assert!(!branches.contains("testuser/unstack-flow--c-0000004"));
    assert!(branches.contains("testuser/unstack-flow--c-0000002"));
}

#[test]
fn test_unstack_preserves_inherited_base_config() {
    let (_temp_dir, repo_path) = setup_unstack_repo("unstack-base", 3);

    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: Value =
        serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    config["defaults"]["base"] = serde_json::json!("main");
    config["stacks"]["unstack-base"]
        .as_object_mut()
        .unwrap()
        .remove("base");
    fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    let (success, _stdout, stderr) = run_gg(
        &repo_path,
        &[
            "unstack",
            "--target",
            "2",
            "--name",
            "unstack-base-top",
            "--no-tui",
        ],
    );
    assert!(success, "gg unstack failed: {stderr}");

    let config: Value = serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(config["defaults"]["base"], "main");
    assert!(
        config["stacks"]["unstack-base-top"]["base"].is_null(),
        "new stack should inherit defaults.base instead of pinning an override: {config}"
    );
}

#[test]
fn test_unstack_default_name_handles_trailing_hyphen_stack_name() {
    let (_temp_dir, repo_path) = setup_unstack_repo("unstack-edge-", 3);

    let (success, _stdout, stderr) = run_gg(&repo_path, &["unstack", "--target", "2", "--no-tui"]);
    assert!(success, "gg unstack failed: {stderr}");

    let lower_commits = rev_list(&repo_path, "main..testuser/unstack-edge-");
    let upper_commits = rev_list(&repo_path, "main..testuser/unstack-edge-2");
    assert_eq!(lower_commits.len(), 1);
    assert_eq!(upper_commits.len(), 2);
}

#[test]
fn test_unstack_rejects_first_position_without_mutating() {
    let (_temp_dir, repo_path) = setup_unstack_repo("unstack-invalid", 3);
    let before = rev_list(&repo_path, "main..testuser/unstack-invalid");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["unstack", "--target", "1", "--no-tui"]);
    assert!(!success, "unstack at position 1 should fail");
    assert!(
        stderr.contains("position 1") || stderr.contains("original stack would be empty"),
        "unexpected stderr: {stderr}"
    );

    let after = rev_list(&repo_path, "main..testuser/unstack-invalid");
    assert_eq!(after, before);
    let (success, _stdout) = run_git(&repo_path, &["rev-parse", "testuser/unstack-invalid-2"]);
    assert!(!success, "new stack branch should not be created");
}

#[test]
fn test_unstack_last_entry_json_creates_one_entry_stack() {
    let (_temp_dir, repo_path) = setup_unstack_repo("unstack-json", 3);

    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &[
            "unstack",
            "--target",
            "3",
            "--name",
            "unstack-json-tail",
            "--no-tui",
            "--json",
        ],
    );
    assert!(success, "gg unstack --json failed: {stderr}");
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSON mode: {stderr}"
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["unstack"]["original_stack"], "unstack-json");
    assert_eq!(parsed["unstack"]["new_stack"], "unstack-json-tail");
    assert_eq!(parsed["unstack"]["split_position"], 3);
    assert_eq!(
        parsed["unstack"]["remaining_entries"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        parsed["unstack"]["moved_entries"].as_array().unwrap().len(),
        1
    );

    let upper_commits = rev_list(&repo_path, "main..testuser/unstack-json-tail");
    assert_eq!(upper_commits.len(), 1);
    let upper_root = commit_message(&repo_path, &upper_commits[0]);
    assert!(upper_root.contains("GG-ID: c-0000003"));
    assert!(!upper_root.contains("GG-Parent:"));
}
