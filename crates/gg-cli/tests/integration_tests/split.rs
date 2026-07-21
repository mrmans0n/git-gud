use crate::helpers::{create_test_repo, run_gg, run_gg_with_env, run_git, run_git_full};

use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::process::{Command, Stdio};

fn create_two_hunk_split_commit() -> (tempfile::TempDir, std::path::PathBuf) {
    let (temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let initial_content = (1..=20)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    fs::write(repo_path.join("multi_hunk.txt"), initial_content)
        .expect("Failed to write initial file");
    run_git(&repo_path, &["add", "multi_hunk.txt"]);
    run_git(&repo_path, &["commit", "--amend", "--no-edit"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-split-describe"]);
    assert!(success, "Failed to create stack: {stderr}");

    let modified_content = (1..=20)
        .map(|line| match line {
            2 => "line 2 modified\n".to_string(),
            18 => "line 18 modified\n".to_string(),
            _ => format!("line {line}\n"),
        })
        .collect::<String>();
    fs::write(repo_path.join("multi_hunk.txt"), modified_content)
        .expect("Failed to write modified file");
    fs::write(repo_path.join("binary.dat"), [0, 159, 146, 150]).expect("write binary fixture");
    run_git(&repo_path, &["add", "multi_hunk.txt", "binary.dat"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Two separated hunks\n\nGG-ID: c-abc1234"],
    );

    (temp_dir, repo_path)
}

fn create_byte_content_split_commit(
    stack_name: &str,
    file_name: &str,
    initial: &[u8],
    modified: &[u8],
) -> (tempfile::TempDir, std::path::PathBuf) {
    let (temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();
    run_git(&repo_path, &["config", "core.autocrlf", "false"]);

    fs::write(repo_path.join(file_name), initial).unwrap();
    run_git(&repo_path, &["add", file_name]);
    run_git(&repo_path, &["commit", "--amend", "--no-edit"]);
    let (success, _, stderr) = run_gg(&repo_path, &["co", stack_name]);
    assert!(success, "create stack failed: {stderr}");

    fs::write(repo_path.join(file_name), modified).unwrap();
    run_git(&repo_path, &["add", file_name]);
    run_git(
        &repo_path,
        &["commit", "-m", "Byte-sensitive change\n\nGG-ID: c-byt1234"],
    );
    (temp_dir, repo_path)
}

fn create_directory_to_file_split_commit(
    stack_name: &str,
) -> (tempfile::TempDir, std::path::PathBuf) {
    let (temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let node_dir = repo_path.join("node");
    fs::create_dir_all(&node_dir).unwrap();
    fs::write(node_dir.join("last.txt"), "nested parent\n").unwrap();
    fs::write(repo_path.join("remainder.txt"), "remainder parent\n").unwrap();
    run_git(&repo_path, &["add", "node/last.txt", "remainder.txt"]);
    run_git(&repo_path, &["commit", "--amend", "--no-edit"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", stack_name]);
    assert!(success, "create stack failed: {stderr}");
    fs::remove_file(node_dir.join("last.txt")).unwrap();
    fs::remove_dir(&node_dir).unwrap();
    fs::write(&node_dir, "top-level target\n").unwrap();
    fs::write(repo_path.join("remainder.txt"), "remainder target\n").unwrap();
    run_git(&repo_path, &["add", "-A"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Directory to file\n\nGG-ID: c-node123"],
    );

    (temp_dir, repo_path)
}

fn create_file_to_directory_split_commit(
    stack_name: &str,
) -> (tempfile::TempDir, std::path::PathBuf) {
    let (temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    fs::write(repo_path.join("node"), "top-level parent\n").unwrap();
    fs::write(repo_path.join("remainder.txt"), "remainder parent\n").unwrap();
    run_git(&repo_path, &["add", "node", "remainder.txt"]);
    run_git(&repo_path, &["commit", "--amend", "--no-edit"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", stack_name]);
    assert!(success, "create stack failed: {stderr}");
    fs::remove_file(repo_path.join("node")).unwrap();
    fs::create_dir(repo_path.join("node")).unwrap();
    fs::write(repo_path.join("node/last.txt"), "nested target\n").unwrap();
    fs::write(repo_path.join("remainder.txt"), "remainder target\n").unwrap();
    run_git(&repo_path, &["add", "-A"]);
    run_git(
        &repo_path,
        &["commit", "-m", "File to directory\n\nGG-ID: c-dir1234"],
    );

    (temp_dir, repo_path)
}

fn create_file_to_directory_with_non_textual_addition() -> (tempfile::TempDir, std::path::PathBuf) {
    let (temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    fs::write(repo_path.join("node"), "top-level parent\n").unwrap();
    fs::write(repo_path.join("remainder.txt"), "remainder parent\n").unwrap();
    run_git(&repo_path, &["add", "node", "remainder.txt"]);
    run_git(&repo_path, &["commit", "--amend", "--no-edit"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "file-to-directory-non-textual"]);
    assert!(success, "create stack failed: {stderr}");
    fs::remove_file(repo_path.join("node")).unwrap();
    fs::create_dir(repo_path.join("node")).unwrap();
    fs::write(repo_path.join("node/last.bin"), b"invalid: \xff\n").unwrap();
    fs::write(repo_path.join("remainder.txt"), "remainder target\n").unwrap();
    run_git(&repo_path, &["add", "-A"]);
    run_git(
        &repo_path,
        &[
            "commit",
            "-m",
            "File to non-textual directory\n\nGG-ID: c-non1234",
        ],
    );

    (temp_dir, repo_path)
}

fn create_directory_to_file_with_non_textual_deletion() -> (tempfile::TempDir, std::path::PathBuf) {
    let (temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    fs::create_dir(repo_path.join("node")).unwrap();
    fs::write(repo_path.join("node/last.bin"), b"invalid: \xff\n").unwrap();
    fs::write(repo_path.join("remainder.txt"), "remainder parent\n").unwrap();
    run_git(&repo_path, &["add", "node/last.bin", "remainder.txt"]);
    run_git(&repo_path, &["commit", "--amend", "--no-edit"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "non-textual-transition"]);
    assert!(success, "create stack failed: {stderr}");
    fs::remove_file(repo_path.join("node/last.bin")).unwrap();
    fs::remove_dir(repo_path.join("node")).unwrap();
    fs::write(repo_path.join("node"), "top-level target\n").unwrap();
    fs::write(repo_path.join("remainder.txt"), "remainder target\n").unwrap();
    run_git(&repo_path, &["add", "-A"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Non-textual transition\n\nGG-ID: c-bin1234"],
    );

    (temp_dir, repo_path)
}

fn eof_fixture_content(modified_line: bool, trailing_newline: bool) -> Vec<u8> {
    let mut content = (1..=20)
        .map(|line| {
            if line == 2 && modified_line {
                "line 2 modified".to_string()
            } else {
                format!("line {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();
    if trailing_newline {
        content.push(b'\n');
    }
    content
}

fn assert_structured_eof_newline_selection(parent_newline: bool, target_newline: bool) {
    let parent = eof_fixture_content(false, parent_newline);
    let target = eof_fixture_content(true, target_newline);
    let (_temp_dir, repo_path) = create_byte_content_split_commit(
        if target_newline {
            "eof-newline-add"
        } else {
            "eof-newline-remove"
        },
        "eof.txt",
        &parent,
        &target,
    );
    let describe = describe_split(&repo_path, "1");
    let eof_hunk = describe["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|hunk| hunk["patch"].as_str().unwrap().contains("line 20"))
        .expect("Describe should expose the EOF-newline hunk");
    assert_eq!(describe["hunks"].as_array().unwrap().len(), 2);
    let plan_path = write_split_plan(&repo_path, &describe, vec![eof_hunk["id"].clone()]);

    let (success, stdout, stderr) = apply_split_plan(&repo_path, &plan_path);
    assert!(success, "apply failed: stdout={stdout} stderr={stderr}");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let first_sha = result["first"]["sha"].as_str().unwrap();
    let first_content = run_git_full(&repo_path, &["show", &format!("{first_sha}:eof.txt")])
        .1
        .into_bytes();
    assert_eq!(
        first_content,
        eof_fixture_content(false, target_newline),
        "first commit must contain only the selected EOF-newline change"
    );
}

fn describe_split(repo_path: &Path, target: &str) -> serde_json::Value {
    let (success, stdout, stderr) = run_gg(
        repo_path,
        &["split", "--describe", "--commit", target, "--json"],
    );
    assert!(success, "describe failed: {stderr}");
    serde_json::from_str(&stdout).expect("describe should return JSON")
}

fn write_split_plan(
    repo_path: &Path,
    describe: &serde_json::Value,
    selected_hunk_ids: Vec<serde_json::Value>,
) -> std::path::PathBuf {
    let path = repo_path.join(".git/gg/test-split-plan.json");
    let plan = serde_json::json!({
        "version": 1,
        "plan_token": describe["plan_token"],
        "target": describe["target"],
        "selected_hunk_ids": selected_hunk_ids,
        "first_message": "Structured first",
        "remainder_message": "Structured remainder",
    });
    fs::write(&path, serde_json::to_vec_pretty(&plan).unwrap()).expect("write split plan");
    path
}

fn operation_records(repo_path: &Path) -> Vec<std::ffi::OsString> {
    let operations_dir = repo_path.join(".git/gg/operations");
    if !operations_dir.exists() {
        return Vec::new();
    }
    let mut records = fs::read_dir(operations_dir)
        .expect("read operation records")
        .map(|entry| entry.expect("read operation record").file_name())
        .collect::<Vec<_>>();
    records.sort();
    records
}

fn apply_split_plan(repo_path: &Path, plan_path: &Path) -> (bool, String, String) {
    run_gg(
        repo_path,
        &[
            "split",
            "--plan-json",
            plan_path.to_str().unwrap(),
            "--json",
        ],
    )
}

fn assert_path_dependent_plan_rejected(
    repo_path: &Path,
    describe: &serde_json::Value,
    selected_path: &str,
) {
    let selected_hunk = describe["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|hunk| hunk["path"] == selected_path)
        .unwrap_or_else(|| panic!("Describe should expose {selected_path}"));
    let plan_path = write_split_plan(repo_path, describe, vec![selected_hunk["id"].clone()]);
    let refs_before = run_git_full(repo_path, &["show-ref"]).1;
    let operations_before = operation_records(repo_path);

    let (success, stdout, stderr) = apply_split_plan(repo_path, &plan_path);
    assert!(!success, "dependent plan unexpectedly applied: {stdout}");
    let error: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(error["version"], 1);
    assert!(
        error["error"].as_str().unwrap().contains("path-dependent"),
        "error should explain the dependency: {stdout}"
    );
    assert!(
        stderr.is_empty(),
        "structured error should use stdout: {stderr}"
    );
    assert_eq!(run_git_full(repo_path, &["show-ref"]).1, refs_before);
    assert_eq!(operation_records(repo_path), operations_before);
}

fn assert_file_to_directory_deletion_only(
    repo_path: &Path,
    describe: &serde_json::Value,
    target_child: &str,
) {
    let deletion_hunk = describe["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|hunk| hunk["path"] == "node")
        .expect("Describe should expose the top-level deletion hunk");
    let plan_path = write_split_plan(repo_path, describe, vec![deletion_hunk["id"].clone()]);
    let original_head = run_git_full(repo_path, &["rev-parse", "HEAD"]).1;
    let operations_before = operation_records(repo_path);

    let (success, stdout, stderr) = apply_split_plan(repo_path, &plan_path);
    assert!(success, "apply failed: stdout={stdout} stderr={stderr}");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let first_sha = result["first"]["sha"].as_str().unwrap();
    let remainder_sha = result["remainder"]["sha"].as_str().unwrap();
    assert!(
        !run_git(repo_path, &["cat-file", "-e", &format!("{first_sha}:node")]).0,
        "deletion-only first commit must not copy the replacement directory"
    );
    assert_eq!(
        run_git_full(
            repo_path,
            &["rev-parse", &format!("{remainder_sha}^{{tree}}")]
        )
        .1
        .trim(),
        describe["target"]["tree"].as_str().unwrap()
    );
    assert!(
        run_git(
            repo_path,
            &["cat-file", "-e", &format!("{remainder_sha}:{target_child}")],
        )
        .0
    );
    assert_eq!(
        operation_records(repo_path).len(),
        operations_before.len() + 1
    );

    let operation_id = result["operation_id"].as_str().unwrap();
    let (success, stdout, stderr) = run_gg(repo_path, &["undo", operation_id, "--json"]);
    assert!(success, "undo failed: stdout={stdout} stderr={stderr}");
    assert_eq!(
        run_git_full(repo_path, &["rev-parse", "HEAD"]).1,
        original_head
    );
}

#[cfg(unix)]
fn assert_same_path_type_change_is_non_textual(
    stack_name: &str,
    parent_is_symlink: bool,
    invalid_regular: bool,
) {
    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let node_path = repo_path.join("node");
    if parent_is_symlink {
        symlink("parent-link-target", &node_path).unwrap();
    } else if invalid_regular {
        fs::write(&node_path, b"invalid parent: \xff\n").unwrap();
    } else {
        fs::write(&node_path, "regular parent\n").unwrap();
    }
    fs::write(repo_path.join("selected.txt"), "selected parent\n").unwrap();
    fs::write(repo_path.join("remainder.txt"), "remainder parent\n").unwrap();
    run_git(
        &repo_path,
        &["add", "node", "selected.txt", "remainder.txt"],
    );
    run_git(&repo_path, &["commit", "--amend", "--no-edit"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", stack_name]);
    assert!(success, "create stack failed: {stderr}");
    fs::remove_file(&node_path).unwrap();
    if parent_is_symlink {
        fs::write(&node_path, "regular target\n").unwrap();
    } else {
        symlink("target-link-destination", &node_path).unwrap();
    }
    fs::write(repo_path.join("selected.txt"), "selected target\n").unwrap();
    fs::write(repo_path.join("remainder.txt"), "remainder target\n").unwrap();
    run_git(&repo_path, &["add", "-A"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Same-path type change\n\nGG-ID: c-type123"],
    );

    let original_head = run_git_full(&repo_path, &["rev-parse", "HEAD"]).1;
    let describe = describe_split(&repo_path, "1");
    assert!(!describe["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|hunk| hunk["path"] == "node"));
    assert_eq!(
        describe["non_textual_files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|path| **path == "node")
            .count(),
        1,
        "type-change path must be classified exactly once"
    );
    let selected_hunk = describe["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|hunk| hunk["path"] == "selected.txt")
        .unwrap();
    let plan_path = write_split_plan(&repo_path, &describe, vec![selected_hunk["id"].clone()]);

    let (success, stdout, stderr) = apply_split_plan(&repo_path, &plan_path);
    assert!(success, "apply failed: stdout={stdout} stderr={stderr}");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let first_sha = result["first"]["sha"].as_str().unwrap();
    let remainder_sha = result["remainder"]["sha"].as_str().unwrap();
    assert_eq!(
        run_git_full(&repo_path, &["rev-parse", &format!("{first_sha}:node")]).1,
        run_git_full(&repo_path, &["rev-parse", "main:node"]).1,
        "type change must stay out of the first commit"
    );
    assert_eq!(
        run_git_full(&repo_path, &["ls-tree", first_sha, "node"]).1,
        run_git_full(&repo_path, &["ls-tree", "main", "node"]).1,
        "first commit must preserve the parent node mode"
    );
    assert_eq!(
        run_git_full(
            &repo_path,
            &["rev-parse", &format!("{remainder_sha}^{{tree}}")],
        )
        .1
        .trim(),
        describe["target"]["tree"].as_str().unwrap()
    );

    let operation_id = result["operation_id"].as_str().unwrap();
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", operation_id, "--json"]);
    assert!(success, "undo failed: stdout={stdout} stderr={stderr}");
    assert_eq!(
        run_git_full(&repo_path, &["rev-parse", "HEAD"]).1,
        original_head
    );
}

#[test]
fn test_split_describe_classifies_crlf_file_as_non_textual() {
    let initial = b"line 1\r\nline 2\r\nline 3\r\n";
    let modified = b"line 1\r\nline 2 changed\r\nline 3\r\n";
    let (_temp_dir, repo_path) =
        create_byte_content_split_commit("crlf", "crlf.txt", initial, modified);

    let describe = describe_split(&repo_path, "1");
    assert!(describe["hunks"].as_array().unwrap().is_empty());
    assert_eq!(
        describe["non_textual_files"],
        serde_json::json!(["crlf.txt"])
    );
}

#[test]
fn test_split_describe_classifies_invalid_utf8_file_as_non_textual() {
    let initial = b"line 1\ninvalid: \xff\nline 3\n";
    let modified = b"line 1 changed\ninvalid: \xff\nline 3\n";
    let (_temp_dir, repo_path) =
        create_byte_content_split_commit("invalid-utf8", "invalid.txt", initial, modified);

    let describe = describe_split(&repo_path, "1");
    assert!(describe["hunks"].as_array().unwrap().is_empty());
    assert_eq!(
        describe["non_textual_files"],
        serde_json::json!(["invalid.txt"])
    );
}

#[test]
fn test_split_plan_selects_eof_newline_addition_exactly() {
    assert_structured_eof_newline_selection(false, true);
}

#[test]
fn test_split_plan_selects_eof_newline_removal_exactly() {
    assert_structured_eof_newline_selection(true, false);
}

#[test]
fn test_split_plan_composes_selected_nested_file_hunks_and_undoes() {
    let (temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    let nested_dir = repo_path.join("src/shared/deeper");
    fs::create_dir_all(&nested_dir).unwrap();
    fs::write(nested_dir.join("a.txt"), "a parent\n").unwrap();
    fs::write(nested_dir.join("b.txt"), "b parent\n").unwrap();
    fs::write(repo_path.join("remainder.txt"), "remainder parent\n").unwrap();
    run_git(&repo_path, &["add", "src", "remainder.txt"]);
    run_git(&repo_path, &["commit", "--amend", "--no-edit"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "nested-compose"]);
    assert!(success, "create stack failed: {stderr}");
    fs::write(nested_dir.join("a.txt"), "a selected\n").unwrap();
    fs::write(nested_dir.join("b.txt"), "b selected\n").unwrap();
    fs::write(repo_path.join("remainder.txt"), "remainder target\n").unwrap();
    run_git(&repo_path, &["add", "src", "remainder.txt"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Nested siblings\n\nGG-ID: c-nest123"],
    );
    let original_head = run_git_full(&repo_path, &["rev-parse", "HEAD"]).1;
    let describe = describe_split(&repo_path, "1");
    let selected = describe["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|hunk| {
            hunk["path"]
                .as_str()
                .unwrap()
                .starts_with("src/shared/deeper/")
        })
        .map(|hunk| hunk["id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 2);
    assert_eq!(describe["hunks"].as_array().unwrap().len(), 3);
    let plan_path = write_split_plan(&repo_path, &describe, selected);

    let (success, stdout, stderr) = apply_split_plan(&repo_path, &plan_path);
    assert!(success, "apply failed: stdout={stdout} stderr={stderr}");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let first_sha = result["first"]["sha"].as_str().unwrap();
    let remainder_sha = result["remainder"]["sha"].as_str().unwrap();
    for (path, expected) in [
        ("src/shared/deeper/a.txt", "a selected\n"),
        ("src/shared/deeper/b.txt", "b selected\n"),
        ("remainder.txt", "remainder parent\n"),
    ] {
        assert_eq!(
            run_git_full(&repo_path, &["show", &format!("{first_sha}:{path}")]).1,
            expected,
            "unexpected first-commit content for {path}"
        );
    }
    for (path, expected) in [
        ("src/shared/deeper/a.txt", "a selected\n"),
        ("src/shared/deeper/b.txt", "b selected\n"),
        ("remainder.txt", "remainder target\n"),
    ] {
        assert_eq!(
            run_git_full(&repo_path, &["show", &format!("{remainder_sha}:{path}")],).1,
            expected,
            "unexpected remainder content for {path}"
        );
    }

    let operation_id = result["operation_id"].as_str().unwrap();
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", operation_id, "--json"]);
    assert!(success, "undo failed: stdout={stdout} stderr={stderr}");
    assert_eq!(
        run_git_full(&repo_path, &["rev-parse", "HEAD"]).1,
        original_head
    );
    drop(temp_dir);
}

#[test]
fn test_split_plan_prunes_empty_nested_directory_before_file_transition() {
    let (_temp_dir, repo_path) = create_directory_to_file_split_commit("directory-to-file");
    let original_head = run_git_full(&repo_path, &["rev-parse", "HEAD"]).1;
    let describe = describe_split(&repo_path, "1");
    let deletion_hunk = describe["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|hunk| hunk["path"] == "node/last.txt")
        .expect("Describe should expose the nested deletion hunk");
    assert!(describe["hunks"].as_array().unwrap().len() >= 2);
    let plan_path = write_split_plan(&repo_path, &describe, vec![deletion_hunk["id"].clone()]);

    let (success, stdout, stderr) = apply_split_plan(&repo_path, &plan_path);
    assert!(success, "apply failed: stdout={stdout} stderr={stderr}");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let first_sha = result["first"]["sha"].as_str().unwrap();
    let remainder_sha = result["remainder"]["sha"].as_str().unwrap();
    assert!(
        !run_git(
            &repo_path,
            &["cat-file", "-e", &format!("{first_sha}:node")]
        )
        .0,
        "selected deletion must prune the empty node/ tree"
    );
    assert_eq!(
        run_git_full(&repo_path, &["show", &format!("{remainder_sha}:node")]).1,
        "top-level target\n"
    );

    let operation_id = result["operation_id"].as_str().unwrap();
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", operation_id, "--json"]);
    assert!(success, "undo failed: stdout={stdout} stderr={stderr}");
    assert_eq!(
        run_git_full(&repo_path, &["rev-parse", "HEAD"]).1,
        original_head
    );
}

#[test]
fn test_split_plan_keeps_selected_file_when_nested_deletion_is_already_applied() {
    let (_temp_dir, repo_path) = create_directory_to_file_split_commit("directory-to-file-both");
    let describe = describe_split(&repo_path, "1");
    let selected = describe["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|hunk| matches!(hunk["path"].as_str().unwrap(), "node" | "node/last.txt"))
        .map(|hunk| hunk["id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        selected.len(),
        2,
        "Describe must expose both transition sides"
    );
    assert!(describe["hunks"].as_array().unwrap().len() >= 3);
    let plan_path = write_split_plan(&repo_path, &describe, selected);

    let (success, stdout, stderr) = apply_split_plan(&repo_path, &plan_path);
    assert!(success, "apply failed: stdout={stdout} stderr={stderr}");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let first_sha = result["first"]["sha"].as_str().unwrap();
    let remainder_sha = result["remainder"]["sha"].as_str().unwrap();
    assert_eq!(
        run_git_full(&repo_path, &["show", &format!("{first_sha}:node")]).1,
        "top-level target\n"
    );
    assert_eq!(
        run_git_full(&repo_path, &["show", &format!("{first_sha}:remainder.txt")]).1,
        "remainder parent\n"
    );
    assert_eq!(
        run_git_full(&repo_path, &["show", &format!("{remainder_sha}:node")]).1,
        "top-level target\n"
    );
}

#[test]
fn test_split_plan_rejects_directory_to_file_addition_without_deletion() {
    let (_temp_dir, repo_path) =
        create_directory_to_file_split_commit("directory-to-file-addition-only");
    let describe = describe_split(&repo_path, "1");

    assert_path_dependent_plan_rejected(&repo_path, &describe, "node");
}

#[test]
fn test_split_plan_rejects_file_to_directory_addition_without_deletion() {
    let (_temp_dir, repo_path) =
        create_file_to_directory_split_commit("file-to-directory-addition-only");
    let describe = describe_split(&repo_path, "1");

    assert_path_dependent_plan_rejected(&repo_path, &describe, "node/last.txt");
}

#[test]
fn test_split_plan_file_to_directory_deletion_only_does_not_copy_text_child() {
    let (_temp_dir, repo_path) =
        create_file_to_directory_split_commit("file-to-directory-deletion-only");
    let describe = describe_split(&repo_path, "1");

    assert_file_to_directory_deletion_only(&repo_path, &describe, "node/last.txt");
}

#[test]
fn test_split_plan_file_to_directory_deletion_only_does_not_copy_non_textual_child() {
    let (_temp_dir, repo_path) = create_file_to_directory_with_non_textual_addition();
    let describe = describe_split(&repo_path, "1");
    assert!(describe["non_textual_files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path == "node/last.bin"));

    assert_file_to_directory_deletion_only(&repo_path, &describe, "node/last.bin");
}

#[cfg(unix)]
#[test]
fn test_split_plan_regular_to_symlink_type_change_stays_non_textual() {
    assert_same_path_type_change_is_non_textual("regular-to-symlink", false, false);
}

#[cfg(unix)]
#[test]
fn test_split_plan_symlink_to_regular_type_change_stays_non_textual() {
    assert_same_path_type_change_is_non_textual("symlink-to-regular", true, false);
}

#[cfg(unix)]
#[test]
fn test_split_plan_invalid_utf8_regular_to_symlink_stays_non_textual() {
    assert_same_path_type_change_is_non_textual("invalid-regular-to-symlink", false, true);
}

#[test]
fn test_split_plan_rejects_addition_with_non_textual_dependent_deletion() {
    let (_temp_dir, repo_path) = create_directory_to_file_with_non_textual_deletion();
    let describe = describe_split(&repo_path, "1");
    assert!(describe["non_textual_files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path == "node/last.bin"));

    assert_path_dependent_plan_rejected(&repo_path, &describe, "node");
}

#[test]
fn test_split_plan_applies_selected_hunk_and_undo_restores_stack() {
    let (_temp_dir, repo_path) = create_two_hunk_split_commit();
    let describe = describe_split(&repo_path, "1");
    let original_sha = describe["target"]["sha"].as_str().unwrap().to_string();
    let original_head = run_git_full(&repo_path, &["rev-parse", "HEAD"]).1;
    let selected = vec![describe["hunks"][0]["id"].clone()];
    let plan_path = write_split_plan(&repo_path, &describe, selected);

    let (success, stdout, stderr) = apply_split_plan(&repo_path, &plan_path);
    assert!(success, "apply failed: {stderr}");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["version"], 1);
    assert!(result["operation_id"].as_str().unwrap().starts_with("op_"));
    assert_eq!(result["original_sha"], original_sha);
    assert!(result["first"]["sha"].as_str().is_some());
    assert!(result["first"]["gg_id"].as_str().is_some());
    assert!(result["remainder"]["sha"].as_str().is_some());
    assert_eq!(result["remainder"]["gg_id"], "c-abc1234");
    assert_eq!(result["rewritten_descendants"], serde_json::json!([]));
    let first_sha = result["first"]["sha"].as_str().unwrap();
    let remainder_sha = result["remainder"]["sha"].as_str().unwrap();
    assert!(
        !run_git(
            &repo_path,
            &["cat-file", "-e", &format!("{first_sha}:binary.dat")]
        )
        .0
    );
    assert!(
        run_git(
            &repo_path,
            &["cat-file", "-e", &format!("{remainder_sha}:binary.dat")],
        )
        .0
    );
    assert_eq!(
        run_git_full(&repo_path, &["rev-list", "--count", "HEAD"])
            .1
            .trim(),
        "3"
    );

    let operation_id = result["operation_id"].as_str().unwrap();
    let (success, stdout, stderr) = run_gg(&repo_path, &["undo", operation_id, "--json"]);
    assert!(success, "undo failed: stdout={stdout} stderr={stderr}");
    assert_eq!(
        run_git_full(&repo_path, &["rev-parse", "HEAD"]).1,
        original_head
    );
    assert_eq!(
        run_git_full(&repo_path, &["rev-list", "--count", "HEAD"])
            .1
            .trim(),
        "2"
    );
}

#[test]
fn test_split_plan_rejects_stale_target_without_mutation() {
    let (_temp_dir, repo_path) = create_two_hunk_split_commit();
    let describe = describe_split(&repo_path, "1");
    let plan_path = write_split_plan(
        &repo_path,
        &describe,
        vec![describe["hunks"][0]["id"].clone()],
    );
    run_git(
        &repo_path,
        &[
            "commit",
            "--amend",
            "-m",
            "Changed after describe\n\nGG-ID: c-abc1234",
        ],
    );
    let refs_before = run_git_full(&repo_path, &["show-ref"]).1;
    let operations_before = operation_records(&repo_path);

    let (success, stdout, stderr) = apply_split_plan(&repo_path, &plan_path);
    assert!(!success, "stale plan unexpectedly applied: {stdout}");
    let error: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(error["error"]
        .as_str()
        .unwrap()
        .contains("stale split plan"));
    assert!(
        stderr.is_empty(),
        "structured error should use stdout: {stderr}"
    );
    assert_eq!(run_git_full(&repo_path, &["show-ref"]).1, refs_before);
    assert_eq!(operation_records(&repo_path), operations_before);
}

#[test]
fn test_split_plan_rejects_stale_target_without_gg_id_as_stale_plan() {
    let (_temp_dir, repo_path) = create_two_hunk_split_commit();
    assert!(
        run_git(
            &repo_path,
            &["commit", "--amend", "-m", "Two separated hunks"],
        )
        .0
    );
    let describe = describe_split(&repo_path, "1");
    assert!(describe["target"]["gg_id"].is_null());
    let plan_path = write_split_plan(
        &repo_path,
        &describe,
        vec![describe["hunks"][0]["id"].clone()],
    );
    assert!(
        run_git(
            &repo_path,
            &["commit", "--amend", "-m", "Changed after describe"],
        )
        .0
    );
    let refs_before = run_git_full(&repo_path, &["show-ref"]).1;
    let operations_before = operation_records(&repo_path);

    let (success, stdout, stderr) = apply_split_plan(&repo_path, &plan_path);
    assert!(!success, "stale plan unexpectedly applied: {stdout}");
    let error: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(error["error"]
        .as_str()
        .unwrap()
        .contains("stale split plan"));
    assert!(
        stderr.is_empty(),
        "structured error should use stdout: {stderr}"
    );
    assert_eq!(run_git_full(&repo_path, &["show-ref"]).1, refs_before);
    assert_eq!(operation_records(&repo_path), operations_before);
}

#[test]
fn test_split_plan_rejects_stale_target_after_gg_id_is_removed() {
    let (_temp_dir, repo_path) = create_two_hunk_split_commit();
    let describe = describe_split(&repo_path, "1");
    assert_eq!(describe["target"]["gg_id"], "c-abc1234");
    let plan_path = write_split_plan(
        &repo_path,
        &describe,
        vec![describe["hunks"][0]["id"].clone()],
    );
    assert!(
        run_git(
            &repo_path,
            &["commit", "--amend", "-m", "Changed after describe"],
        )
        .0
    );
    let refs_before = run_git_full(&repo_path, &["show-ref"]).1;
    let operations_before = operation_records(&repo_path);

    let (success, stdout, stderr) = apply_split_plan(&repo_path, &plan_path);
    assert!(!success, "stale plan unexpectedly applied: {stdout}");
    let error: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(error["error"]
        .as_str()
        .unwrap()
        .contains("stale split plan"));
    assert!(
        stderr.is_empty(),
        "structured error should use stdout: {stderr}"
    );
    assert_eq!(run_git_full(&repo_path, &["show-ref"]).1, refs_before);
    assert_eq!(operation_records(&repo_path), operations_before);
}

#[test]
fn test_split_plan_rejects_invalid_inputs_before_mutation() {
    let invalid_cases = [
        ("zero selected hunks", serde_json::json!([])),
        (
            "all selected hunks",
            serde_json::json!(["$first", "$second"]),
        ),
        ("unknown hunk", serde_json::json!(["h-unknown"])),
        ("duplicate hunk", serde_json::json!(["$first", "$first"])),
    ];

    for (name, selected_template) in invalid_cases {
        let (_temp_dir, repo_path) = create_two_hunk_split_commit();
        let describe = describe_split(&repo_path, "1");
        let first_id = describe["hunks"][0]["id"].clone();
        let second_id = describe["hunks"][1]["id"].clone();
        let selected = selected_template
            .as_array()
            .unwrap()
            .iter()
            .map(|id| match id.as_str().unwrap() {
                "$first" => first_id.clone(),
                "$second" => second_id.clone(),
                _ => id.clone(),
            })
            .collect();
        let plan_path = write_split_plan(&repo_path, &describe, selected);
        let refs_before = run_git_full(&repo_path, &["show-ref"]).1;
        let operations_before = operation_records(&repo_path);

        let (success, stdout, _stderr) = apply_split_plan(&repo_path, &plan_path);
        assert!(!success, "{name} unexpectedly applied: {stdout}");
        serde_json::from_str::<serde_json::Value>(&stdout)
            .unwrap_or_else(|_| panic!("{name} should return JSON: {stdout}"));
        assert_eq!(
            run_git_full(&repo_path, &["show-ref"]).1,
            refs_before,
            "{name}"
        );
        assert_eq!(operation_records(&repo_path), operations_before, "{name}");
    }

    for (name, field, value) in [
        (
            "empty first message",
            "first_message",
            serde_json::json!("  "),
        ),
        (
            "empty remainder message",
            "remainder_message",
            serde_json::json!("\n"),
        ),
        ("unsupported version", "version", serde_json::json!(2)),
    ] {
        let (_temp_dir, repo_path) = create_two_hunk_split_commit();
        let describe = describe_split(&repo_path, "1");
        let plan_path = write_split_plan(
            &repo_path,
            &describe,
            vec![describe["hunks"][0]["id"].clone()],
        );
        let mut plan: serde_json::Value =
            serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
        plan[field] = value;
        fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
        let refs_before = run_git_full(&repo_path, &["show-ref"]).1;
        let operations_before = operation_records(&repo_path);

        let (success, stdout, _stderr) = apply_split_plan(&repo_path, &plan_path);
        assert!(!success, "{name} unexpectedly applied: {stdout}");
        serde_json::from_str::<serde_json::Value>(&stdout)
            .unwrap_or_else(|_| panic!("{name} should return JSON: {stdout}"));
        assert_eq!(
            run_git_full(&repo_path, &["show-ref"]).1,
            refs_before,
            "{name}"
        );
        assert_eq!(operation_records(&repo_path), operations_before, "{name}");
    }
}

#[test]
fn test_split_plan_reports_rewritten_descendant_identity() {
    let (_temp_dir, repo_path) = create_two_hunk_split_commit();
    fs::write(repo_path.join("descendant.txt"), "descendant\n").unwrap();
    run_git(&repo_path, &["add", "descendant.txt"]);
    run_git(
        &repo_path,
        &["commit", "-m", "Descendant\n\nGG-ID: c-def5678"],
    );
    let describe = describe_split(&repo_path, "1");
    let plan_path = write_split_plan(
        &repo_path,
        &describe,
        vec![describe["hunks"][0]["id"].clone()],
    );

    let (success, stdout, stderr) = apply_split_plan(&repo_path, &plan_path);
    assert!(success, "apply failed: {stderr}");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let descendants = result["rewritten_descendants"].as_array().unwrap();
    assert_eq!(descendants.len(), 1);
    assert_eq!(descendants[0]["gg_id"], "c-def5678");
    assert!(descendants[0]["sha"].as_str().is_some());
}

#[test]
fn test_split_plan_rejects_immutable_target_without_force_path() {
    let (_temp_dir, repo_path) = create_two_hunk_split_commit();
    let describe = describe_split(&repo_path, "1");
    let plan_path = write_split_plan(
        &repo_path,
        &describe,
        vec![describe["hunks"][0]["id"].clone()],
    );
    let config_path = repo_path.join(".git/gg/config.json");
    let mut config: serde_json::Value =
        serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["provider"] = serde_json::json!("github");
    config["stacks"]["test-split-describe"]["mrs"]["c-abc1234"] = serde_json::json!(99);
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let fake_bin = repo_path.join("fake-bin");
    fs::create_dir_all(&fake_bin).unwrap();
    let fake_gh = fake_bin.join("gh");
    fs::write(
        &fake_gh,
        "#!/bin/sh\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then\n  echo '{\"number\":99,\"title\":\"Two separated hunks\",\"state\":\"MERGED\",\"url\":\"https://github.com/test/repo/pull/99\",\"headRefName\":\"testuser/test-split-describe--c-abc1234\",\"isDraft\":false,\"mergeable\":\"MERGEABLE\",\"reviews\":[]}'\n  exit 0\nfi\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&fake_gh).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_gh, permissions).unwrap();
    }
    let mut path = std::ffi::OsString::from(fake_bin.as_os_str());
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap_or_default());
    let refs_before = run_git_full(&repo_path, &["show-ref"]).1;
    let operations_before = operation_records(&repo_path);

    let (success, stdout, stderr) = run_gg_with_env(
        &repo_path,
        &[
            "split",
            "--plan-json",
            plan_path.to_str().unwrap(),
            "--json",
        ],
        &[("PATH", path.as_os_str())],
    );
    assert!(!success, "immutable target unexpectedly applied: {stdout}");
    let error: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(error["error"]
        .as_str()
        .unwrap()
        .contains("cannot rewrite immutable commits"));
    assert!(
        stderr.is_empty(),
        "structured error should use stdout: {stderr}"
    );
    assert_eq!(run_git_full(&repo_path, &["show-ref"]).1, refs_before);
    assert_eq!(operation_records(&repo_path), operations_before);
}

#[test]
fn test_split_describe_json() {
    let (_temp_dir, repo_path) = create_two_hunk_split_commit();
    let refs_before = run_git_full(&repo_path, &["show-ref"]).1;
    let operation_records = || {
        let operations_dir = repo_path.join(".git/gg/operations");
        if !operations_dir.exists() {
            return Vec::new();
        }
        let mut records = fs::read_dir(operations_dir)
            .expect("Failed to read operation records")
            .map(|entry| entry.expect("Failed to read operation record").file_name())
            .collect::<Vec<_>>();
        records.sort();
        records
    };
    let operations_before = operation_records();

    let describe = || {
        let (success, stdout, stderr) = run_gg(
            &repo_path,
            &["split", "--describe", "--commit", "1", "--json"],
        );
        assert!(success, "describe failed: {stderr}");
        serde_json::from_str::<serde_json::Value>(&stdout).expect("describe should return JSON")
    };

    let first = describe();
    assert_eq!(first["version"], 1);
    assert_eq!(first["hunks"].as_array().unwrap().len(), 2);
    assert!(first["plan_token"]
        .as_str()
        .unwrap()
        .starts_with("split-v1-"));
    assert_eq!(first["remainder_message"], "Two separated hunks");

    let second = describe();
    assert_eq!(second["target"], first["target"]);
    assert_eq!(second["plan_token"], first["plan_token"]);
    let hunk_ids = |value: &serde_json::Value| {
        value["hunks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|hunk| hunk["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(hunk_ids(&second), hunk_ids(&first));
    assert_eq!(run_git_full(&repo_path, &["status", "--porcelain"]).1, "");
    assert_eq!(run_git_full(&repo_path, &["show-ref"]).1, refs_before);
    assert_eq!(operation_records(), operations_before);
}

#[test]
fn test_split_describe_json_allows_dirty_worktree() {
    let (_temp_dir, repo_path) = create_two_hunk_split_commit();
    fs::write(repo_path.join("README.md"), "dirty but preserved\n")
        .expect("Failed to dirty worktree");
    let status_before = run_git_full(&repo_path, &["status", "--porcelain"]).1;

    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["split", "--describe", "--commit", "1", "--json"],
    );

    assert!(success, "describe failed for dirty worktree: {stderr}");
    serde_json::from_str::<serde_json::Value>(&stdout).expect("describe should return JSON");
    assert_eq!(
        run_git_full(&repo_path, &["status", "--porcelain"]).1,
        status_before
    );
}

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
    assert!(
        stdout.contains("--describe"),
        "split help should mention --describe: {stdout}"
    );
    assert!(
        stdout.contains("--json"),
        "split help should mention --json: {stdout}"
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
