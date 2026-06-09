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

#[test]
fn test_restack_integrates_inserted_midstack_commit() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "two"]);
    run_gg(&repo_path, &["mv", "1"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "inserted"]);

    let (ok, out, err) = run_gg(&repo_path, &["restack"]);
    assert!(ok, "restack failed: {out}{err}");

    let (_ok, log) = run_git(&repo_path, &["log", "--oneline", "testuser/testing"]);
    assert!(
        log.contains("one") && log.contains("inserted") && log.contains("two"),
        "log: {log}"
    );

    let (_ok, head_subj) = run_git(&repo_path, &["log", "-1", "--pretty=%s", "HEAD"]);
    assert_eq!(head_subj.trim(), "inserted", "HEAD subject: {head_subj}");

    let (ok, out, err) = run_gg(&repo_path, &["ls"]);
    assert!(ok, "ls failed: {out}{err}");
    assert!(out.contains("3 commits"), "ls: {out}");
    assert!(
        !out.contains("Un-integrated commit"),
        "ls should be clean: {out}"
    );
    assert!(
        out.contains("inserted") && out.contains("<- HEAD"),
        "ls: {out}"
    );

    // Metadata was normalized during integration, so the inserted commit now has
    // a GG-ID: a follow-up `gg restack` must succeed (already consistent), not
    // fail with a missing-GG-ID error.
    let (ok, out, err) = run_gg(&repo_path, &["restack"]);
    assert!(ok, "follow-up restack failed: {out}{err}");
    assert!(
        out.contains("consistent"),
        "follow-up restack should be a no-op: {out}"
    );
}

#[test]
fn test_restack_dry_run_does_not_integrate_midstack_commit() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "two"]);
    run_gg(&repo_path, &["mv", "1"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "inserted"]);

    let head_before = run_git(&repo_path, &["rev-parse", "HEAD"]).1;
    let branch_before = run_git(&repo_path, &["rev-parse", "testuser/testing"]).1;

    // --dry-run must preview without mutating anything.
    let (ok, out, err) = run_gg(&repo_path, &["restack", "--dry-run"]);
    assert!(ok, "restack --dry-run failed: {out}{err}");
    assert!(
        out.contains("Would integrate"),
        "dry-run should preview: {out}"
    );

    // The branch tip is unchanged: "inserted" was NOT folded in.
    let (_ok, log) = run_git(&repo_path, &["log", "--oneline", "testuser/testing"]);
    assert!(!log.contains("inserted"), "branch must be untouched: {log}");

    // HEAD and the branch ref are byte-for-byte unchanged.
    assert_eq!(
        run_git(&repo_path, &["rev-parse", "HEAD"]).1,
        head_before,
        "HEAD moved under --dry-run"
    );
    assert_eq!(
        run_git(&repo_path, &["rev-parse", "testuser/testing"]).1,
        branch_before,
        "branch moved under --dry-run"
    );

    // No rebase was left in progress.
    assert!(
        !repo_path.join(".git/rebase-merge").exists()
            && !repo_path.join(".git/rebase-apply").exists(),
        "dry-run must not leave a rebase in progress"
    );

    // --dry-run --json reports the stack name ("testing"), not the branch name
    // ("testuser/testing"), for parity with every other restack JSON path.
    let (ok, out, err) = run_gg(&repo_path, &["restack", "--dry-run", "--json"]);
    assert!(ok, "restack --dry-run --json failed: {out}{err}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert_eq!(v["restack"]["stack_name"], "testing", "json: {out}");
    assert_eq!(v["restack"]["dry_run"], true, "json: {out}");
}

#[test]
fn test_restack_integrates_multiple_inserted_commits() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "two"]);
    run_gg(&repo_path, &["mv", "1"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "ins_a"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "ins_b"]);

    let (ok, out, err) = run_gg(&repo_path, &["restack"]);
    assert!(ok, "restack failed: {out}{err}");

    let (_ok, head_subj) = run_git(&repo_path, &["log", "-1", "--pretty=%s", "HEAD"]);
    assert_eq!(head_subj.trim(), "ins_b", "HEAD subject: {head_subj}");

    let (_ok, log) = run_git(&repo_path, &["log", "--oneline", "testuser/testing"]);
    for m in ["one", "ins_a", "ins_b", "two"] {
        assert!(log.contains(m), "missing {m} in log: {log}");
    }
}

#[test]
fn test_restack_without_orphan_is_unchanged() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "two"]);

    // No detached orphan: the integration path must NOT fire. restack falls
    // through to its normal path, which (on a stack without GG-IDs) reports the
    // usual reconcile guidance rather than integrating anything.
    let (_ok, out, err) = run_gg(&repo_path, &["restack"]);
    let combined = format!("{out}{err}");
    assert!(
        !combined.contains("Integrated"),
        "should not integrate: {combined}"
    );
    assert!(
        combined.contains("GG-ID") || combined.contains("reconcile"),
        "expected normal restack path (reconcile guidance): {combined}"
    );
}

#[test]
fn test_restack_integration_conflict_reports_guidance() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    // "one" leaves conflict.txt absent; "two" adds conflict.txt = "two".
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    fs::write(repo_path.join("conflict.txt"), "two\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "two"]);

    run_gg(&repo_path, &["mv", "1"]);
    // Inserted commit writes the same file with different content -> conflict
    // when "two" is replayed on top.
    fs::write(repo_path.join("conflict.txt"), "inserted\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "inserted"]);

    let (ok, out, err) = run_gg(&repo_path, &["restack"]);
    assert!(!ok, "expected conflict failure: {out}{err}");
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("gg continue")
            || combined.contains("gg abort")
            || combined.contains("conflict"),
        "expected conflict guidance: {combined}"
    );
}

#[test]
fn test_restack_integration_conflict_continue_finishes_integration() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    fs::write(repo_path.join("conflict.txt"), "two\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "two"]);

    run_gg(&repo_path, &["mv", "1"]);
    fs::write(repo_path.join("conflict.txt"), "inserted\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "inserted"]);

    // Conflicts while replaying "two" onto "inserted".
    let (ok, _out, _err) = run_gg(&repo_path, &["restack"]);
    assert!(!ok, "expected conflict");

    // Resolve the conflict and continue.
    fs::write(repo_path.join("conflict.txt"), "resolved\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    let (ok, out, err) = run_gg(&repo_path, &["continue"]);
    assert!(ok, "gg continue failed: {out}{err}");

    // Integration finished: HEAD is back on the inserted commit (detached).
    let (_ok, head_subj) = run_git(&repo_path, &["log", "-1", "--pretty=%s", "HEAD"]);
    assert_eq!(head_subj.trim(), "inserted", "HEAD subject: {head_subj}");

    // Branch contains all three in order.
    let (_ok, log) = run_git(&repo_path, &["log", "--oneline", "testuser/testing"]);
    assert!(
        log.contains("one") && log.contains("inserted") && log.contains("two"),
        "log: {log}"
    );

    // Metadata was normalized: a follow-up `gg restack` is a no-op, not a
    // missing-GG-ID error.
    let (ok, out, err) = run_gg(&repo_path, &["restack"]);
    assert!(ok, "follow-up restack failed: {out}{err}");
    assert!(
        out.contains("consistent"),
        "follow-up restack should be a no-op: {out}"
    );
}

#[test]
fn test_restack_integrates_amended_midstack_commit() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "two"]);
    run_gg(&repo_path, &["mv", "1"]);
    // Amend the navigated commit in place (rewrites "one").
    // --allow-empty is required because the original commit had no tree changes.
    run_git(
        &repo_path,
        &["commit", "--allow-empty", "--amend", "-m", "one_amended"],
    );

    let (ok, out, err) = run_gg(&repo_path, &["restack"]);
    assert!(ok, "restack failed: {out}{err}");

    // HEAD stays on the amended commit.
    let (_ok, head_subj) = run_git(&repo_path, &["log", "-1", "--pretty=%s", "HEAD"]);
    assert_eq!(head_subj.trim(), "one_amended", "HEAD subject: {head_subj}");

    // Branch is one_amended -> two (2 commits, "one" gone).
    let (_ok, log) = run_git(&repo_path, &["log", "--oneline", "testuser/testing"]);
    assert!(
        log.contains("one_amended") && log.contains("two"),
        "log: {log}"
    );
}
