use crate::helpers::{create_test_repo, run_gg, run_git};

use serde_json::Value;
use std::fs;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn test_gg_run_readonly_passing() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "git", "--version"]);
    assert!(
        success,
        "gg run should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("all passed") || stdout.contains("OK"),
        "Expected success message in: {}",
        stdout
    );
}

#[test]
fn test_gg_run_readonly_failing() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-fail-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let (success, _stdout, _stderr) = run_gg(&repo_path, &["run", "false"]);
    assert!(!success, "gg run with 'false' should fail");
}

#[test]
fn test_gg_run_json_output() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-json-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "--json", "git", "--version"]);
    assert!(success, "gg run --json failed: {}", stderr);

    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["run"]["all_passed"], true);

    let results = parsed["run"]["results"]
        .as_array()
        .expect("run.results must be an array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["position"], 1);
    assert_eq!(results[0]["commands"][0]["command"], "git --version");
    assert_eq!(results[0]["commands"][0]["passed"], true);
}

#[cfg(unix)]
#[test]
fn test_gg_run_amend_mode() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-amend-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    fs::write(
        repo_path.join("modify.sh"),
        "#!/bin/sh\necho \"modified\" >> test.txt\n",
    )
    .expect("Failed to write script");
    let mut perms = fs::metadata(repo_path.join("modify.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("modify.sh"), perms).unwrap();

    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let original = fs::read_to_string(repo_path.join("test.txt")).unwrap();

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "--amend", "./modify.sh"]);
    assert!(
        success,
        "gg run --amend should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    let modified = fs::read_to_string(repo_path.join("test.txt")).unwrap();
    assert_ne!(
        original, modified,
        "File should have been modified and amended"
    );
    assert!(
        modified.contains("modified"),
        "File should contain 'modified'"
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_discard_mode() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-discard-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    fs::write(
        repo_path.join("modify.sh"),
        "#!/bin/sh\necho \"modified\" >> test.txt\n",
    )
    .expect("Failed to write script");
    let mut perms = fs::metadata(repo_path.join("modify.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("modify.sh"), perms).unwrap();

    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let original = fs::read_to_string(repo_path.join("test.txt")).unwrap();

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "--discard", "./modify.sh"]);
    assert!(
        success,
        "gg run --discard should succeed: stdout={}, stderr={}",
        stdout, stderr
    );

    let after = fs::read_to_string(repo_path.join("test.txt")).unwrap();
    assert_eq!(original, after, "File should be unchanged after --discard");
}

#[cfg(unix)]
#[test]
fn test_gg_run_readonly_fails_on_dirty_tree() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-readonly-dirty-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    fs::write(
        repo_path.join("modify.sh"),
        "#!/bin/sh\necho \"modified\" >> test.txt\n",
    )
    .expect("Failed to write script");
    let mut perms = fs::metadata(repo_path.join("modify.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("modify.sh"), perms).unwrap();

    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "./modify.sh"]);
    assert!(
        !success,
        "gg run (read-only) should fail when command modifies files"
    );
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("modified files")
            || combined.contains("--amend")
            || combined.contains("--discard"),
        "Error should mention the file modification and suggest --amend/--discard: {}",
        combined
    );
}

#[test]
fn test_gg_run_parallel_passing() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "parallel-pass"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content1").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "First commit"]);

    fs::write(repo_path.join("file2.txt"), "content2").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Second commit"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "-j", "2", "git", "--version"]);
    assert!(
        success,
        "gg run --jobs should succeed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("all passed") || stdout.contains("OK"),
        "Expected success message in: {}",
        stdout
    );
}

#[test]
fn test_gg_run_parallel_failing() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "parallel-fail"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content1").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "First commit"]);

    fs::write(repo_path.join("file2.txt"), "content2").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Second commit"]);

    let (success, _stdout, _stderr) = run_gg(&repo_path, &["run", "-j", "2", "false"]);
    assert!(!success, "gg run parallel with 'false' should fail");
}

#[test]
fn test_gg_run_parallel_json_output() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "parallel-json"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file1.txt"), "content1").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "First commit"]);

    fs::write(repo_path.join("file2.txt"), "content2").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Second commit"]);

    let (success, stdout, stderr) = run_gg(
        &repo_path,
        &["run", "-j", "2", "--json", "git", "--version"],
    );
    assert!(
        success,
        "gg run parallel --json should succeed: stderr={}",
        stderr
    );

    let json: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("should be valid JSON");
    assert_eq!(json["run"]["all_passed"], true);
    let results = json["run"]["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    // Results should be in commit position order
    assert_eq!(results[0]["position"], 1);
    assert_eq!(results[1]["position"], 2);
}

/// Locate the `test` binary on this OS. Linux has `/usr/bin/test`;
/// macOS only ships `/bin/test`. Both exit 0 when the comparison holds.
#[cfg(unix)]
fn locate_test_binary() -> &'static str {
    if std::path::Path::new("/usr/bin/test").exists() {
        "/usr/bin/test"
    } else if std::path::Path::new("/bin/test").exists() {
        "/bin/test"
    } else {
        panic!("no `test` binary found on this system");
    }
}

#[cfg(unix)]
#[test]
fn test_gg_run_preserves_quoted_arguments() {
    // Regression test for Bug #1: `gg run` used to join argv with spaces
    // and re-split on whitespace, destroying argument boundaries. Using
    // the `test` binary surfaces the bug as a non-zero exit when its args
    // get mangled (usage error → exit 2).
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-quoted-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "c1"]);

    // `test "a b" = "a b"` exits 0 when args are preserved, ≠0 otherwise.
    let test_bin = locate_test_binary();
    let (success, stdout, stderr) = run_gg(&repo_path, &["run", test_bin, "a b", "=", "a b"]);
    assert!(
        success,
        "gg run must preserve argument boundaries with whitespace.\nstdout={}\nstderr={}",
        stdout, stderr
    );

    // Negative case: inequality → exit 1 → gg run reports failure.
    let (success, _, _) = run_gg(&repo_path, &["run", test_bin, "a b", "=", "a c"]);
    assert!(
        !success,
        "gg run should report failure when the command's comparison is false"
    );
}

#[test]
fn test_gg_run_json_command_display_escapes_spaces() {
    // Regression test for Bug #1 display path: the `command` field in JSON
    // output should be a copy-pasteable shell form that single-quotes args
    // containing whitespace, not a naive whitespace-joined string.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-json-display-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("file.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "c1"]);

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "--json", "echo", "hello world"]);
    assert!(
        success,
        "gg run --json should succeed: {} / {}",
        stdout, stderr
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("JSON parse failed");
    let cmd = parsed["run"]["results"][0]["commands"][0]["command"]
        .as_str()
        .expect("missing command field");
    assert_eq!(
        cmd, "echo 'hello world'",
        "displayed command must single-quote whitespace args"
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_amend_mid_stack_reports_correct_final_sha() {
    // Regression test for Bug #3: after `--amend` on a non-tail commit the
    // code used to read HEAD (which, post rebase-onto, points at the stack
    // tip) and reported the tip SHA as the amended commit's final_sha. The
    // fix captures the amended OID locally before the rebase-onto runs.
    //
    // Strategy: run `gg run --amend` across positions 1 and 2. For each
    // reported sha, resolve its commit subject via `git show`. The
    // invariant is that position N's reported sha MUST point to a commit
    // whose subject is "Commit N" — if the bug is present, position 1
    // reports the post-rebase HEAD (which is the rebased commit 2, subject
    // "Commit 2") instead of the amended commit 1.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Seed a TRACKED file `touched.txt` in the base commit. The script
    // below appends to it — gg's dirty-check only considers tracked file
    // modifications (untracked files are ignored), so the baseline file
    // must exist before we create the stack.
    fs::write(repo_path.join("touched.txt"), "").expect("write touched.txt");

    // Script appends the current commit's subject line to `touched.txt`.
    // This guarantees each amended commit introduces a *distinct* diff
    // against its parent, so `git rebase --onto` never drops commits as
    // "patch already upstream".
    fs::write(
        repo_path.join("touch_one.sh"),
        "#!/bin/sh\ngit log -1 --format=%s >> touched.txt\n",
    )
    .expect("write script");
    let mut perms = fs::metadata(repo_path.join("touch_one.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("touch_one.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "touched.txt", "touch_one.sh"]);
    run_git(
        &repo_path,
        &["commit", "-m", "add script and touched baseline"],
    );

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-amend-midstack"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Stack: 3 commits (Commit 1, 2, 3) on top of the base.
    for i in 1..=3 {
        fs::write(repo_path.join(format!("f{}.txt", i)), format!("v{}", i)).expect("write");
        run_git(&repo_path, &["add", "."]);
        run_git(&repo_path, &["commit", "-m", &format!("Commit {}", i)]);
    }

    // Move to position 2 (the middle commit) so `gg run` only touches
    // commits at positions 1 and 2, not 3.
    let (success, _, stderr) = run_gg(&repo_path, &["mv", "2"]);
    assert!(success, "mv failed: {}", stderr);

    let (success, stdout, stderr) =
        run_gg(&repo_path, &["run", "--amend", "--json", "./touch_one.sh"]);
    assert!(
        success,
        "gg run --amend should succeed: {} / {}",
        stdout, stderr
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("json parse");
    let results = parsed["run"]["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2, "should have run on commits 1 and 2");

    let sha0 = results[0]["sha"].as_str().unwrap().to_string();
    let sha1 = results[1]["sha"].as_str().unwrap().to_string();

    // Resolve each reported sha to its actual commit subject. Orphan
    // commits are fine — `git show` looks them up in the object store.
    let (ok0, subject0) = run_git(&repo_path, &["show", "-s", "--format=%s", &sha0]);
    assert!(ok0, "failed to show sha0={}", sha0);
    let (ok1, subject1) = run_git(&repo_path, &["show", "-s", "--format=%s", &sha1]);
    assert!(ok1, "failed to show sha1={}", sha1);

    assert_eq!(
        subject0.trim(),
        "Commit 1",
        "Bug #3: position 1's reported sha ({}) must resolve to the amended \
         commit 1 but resolved to a commit with subject {:?}. \
         (Before the fix, the code read HEAD after the rebase-onto which \
         moved HEAD off commit1'.)",
        sha0,
        subject0.trim()
    );
    assert_eq!(
        subject1.trim(),
        "Commit 2",
        "Position 2's reported sha ({}) should resolve to 'Commit 2' but \
         resolved to {:?}",
        sha1,
        subject1.trim()
    );

    assert_ne!(
        sha0, sha1,
        "commit 1 and commit 2 must have distinct reported shas"
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_amend_stop_on_error_preserves_commits_above_failure() {
    // Regression test for Bug #4 (data loss): when `gg run --amend` stops
    // on failure mid-stack, the restoration code must NOT force-reset the
    // branch to the currently-detached HEAD. Commits above the failure
    // point must remain reachable.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Seed a tracked `marker.txt` in the base so the script's append shows
    // up as a dirty tracked file (untracked files wouldn't trigger gg's
    // dirty check — see is_working_directory_clean).
    fs::write(repo_path.join("marker.txt"), "").expect("seed marker.txt");

    // Script: succeed + modify tree on commit 1 and tip, fail on middle.
    // Detects which commit we're on via the presence of f1/f2/f3 files.
    fs::write(
        repo_path.join("cond.sh"),
        "#!/bin/sh\n\
         if [ -f f2.txt ] && [ ! -f f3.txt ]; then\n\
           # Commit 2 (middle): fail loudly\n\
           exit 17\n\
         fi\n\
         echo marker >> marker.txt\n\
         exit 0\n",
    )
    .expect("write script");
    let mut perms = fs::metadata(repo_path.join("cond.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("cond.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "marker.txt", "cond.sh"]);
    run_git(
        &repo_path,
        &["commit", "-m", "add script and marker baseline"],
    );

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-data-loss-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Build 3-commit stack: A, B, C
    for i in 1..=3 {
        fs::write(repo_path.join(format!("f{}.txt", i)), format!("v{}", i)).expect("write");
        run_git(&repo_path, &["add", "."]);
        run_git(&repo_path, &["commit", "-m", &format!("Commit {}", i)]);
    }

    // Count of commits on the branch BEFORE the run.
    let (_, log_before) = run_git(&repo_path, &["rev-list", "--count", "HEAD"]);
    let count_before: usize = log_before.trim().parse().expect("parse count");

    // Run with default stop_on_error. Expected: succeeds on 1, fails on 2.
    // Before the fix: branch gets force-reset to commit 2, commit 3 vanishes.
    // After the fix: branch retains all commits.
    let (success, _, _) = run_gg(&repo_path, &["run", "--amend", "./cond.sh"]);
    assert!(
        !success,
        "gg run should report failure because commit 2 exits non-zero"
    );

    // Count commits on the branch AFTER the run.
    let (_, log_after) = run_git(&repo_path, &["rev-list", "--count", "HEAD"]);
    let count_after: usize = log_after.trim().parse().expect("parse count");
    assert_eq!(
        count_after, count_before,
        "Bug #4: commits above the failing commit were silently discarded. \
         expected {} commits, got {}",
        count_before, count_after
    );

    // And: commit 3 must still exist with its original content reachable.
    let (_, show_output) = run_git(&repo_path, &["show", "HEAD", "--name-only"]);
    assert!(
        show_output.contains("f3.txt"),
        "commit 3's f3.txt should still be reachable at HEAD after failed run, show: {}",
        show_output
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_amend_last_commit_sets_branch_tip_correctly() {
    // Regression test for Task 4 invariant: when --amend runs on the last
    // commit (no rebase needed), the branch must still be forwarded to the
    // new amended OID. Previously this happened by accident via the global
    // move_branch_to_head call which this task deletes.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Seed a tracked `marker.txt` in the base so the script has something
    // to dirty (untracked files are ignored by gg's dirty check).
    fs::write(repo_path.join("marker.txt"), "").expect("seed marker.txt");
    fs::write(
        repo_path.join("touch_marker.sh"),
        "#!/bin/sh\necho marker >> marker.txt\n",
    )
    .expect("write script");
    let mut perms = fs::metadata(repo_path.join("touch_marker.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("touch_marker.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "marker.txt", "touch_marker.sh"]);
    run_git(&repo_path, &["commit", "-m", "seed marker baseline"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-amend-last-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("f1.txt"), "v1").expect("write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "C1"]);

    // Record tip SHA before the amend
    let (_, head_before) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let head_before = head_before.trim().to_string();

    let (success, stdout, stderr) = run_gg(&repo_path, &["run", "--amend", "./touch_marker.sh"]);
    assert!(
        success,
        "gg run --amend on last commit should succeed: {} / {}",
        stdout, stderr
    );

    // Tip SHA must have changed (amended)
    let (_, head_after) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let head_after = head_after.trim().to_string();
    assert_ne!(
        head_before, head_after,
        "HEAD SHA should have changed after amend"
    );

    // HEAD must be on a branch, not detached
    let (_, symref) = run_git(&repo_path, &["symbolic-ref", "HEAD"]);
    assert!(
        symref.trim().starts_with("refs/heads/"),
        "HEAD should be a branch after gg run, got: {}",
        symref
    );

    // marker.txt must contain the appended marker (amend folded it in)
    let (_, marker_content) = run_git(&repo_path, &["show", "HEAD:marker.txt"]);
    assert!(
        marker_content.contains("marker"),
        "amend should have folded the marker append into the commit, marker.txt={:?}",
        marker_content
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_parallel_enforces_read_only_contract() {
    // Regression test for Bug #2: the parallel path used to mark commits as
    // passed based purely on command exit status, ignoring whether the
    // command dirtied the worktree. The sequential path rejects dirty trees
    // in ReadOnly mode; the parallel path must now do the same so `-j N`
    // and `-j 1` are equivalent in terms of what they accept.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Baseline a TRACKED poison.txt file in the base commit. Both parallel
    // (git status --porcelain) and sequential (is_working_directory_clean,
    // which ignores untracked) code paths must agree that modifying this
    // tracked file counts as "dirty".
    fs::write(repo_path.join("poison.txt"), "").expect("seed poison.txt");
    fs::write(
        repo_path.join("dirty.sh"),
        "#!/bin/sh\n# Command exits 0 but dirties the worktree by modifying a tracked file\n\
         echo poison >> poison.txt\n\
         exit 0\n",
    )
    .expect("write dirty.sh");
    let mut perms = fs::metadata(repo_path.join("dirty.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("dirty.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "poison.txt", "dirty.sh"]);
    run_git(&repo_path, &["commit", "-m", "seed poison baseline"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-parallel-dirty-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Two-commit stack so the parallel path actually has work to parallelize.
    fs::write(repo_path.join("f1.txt"), "v1").expect("write f1");
    run_git(&repo_path, &["add", "f1.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("f2.txt"), "v2").expect("write f2");
    run_git(&repo_path, &["add", "f2.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Parallel run in ReadOnly mode must FAIL because dirty.sh dirties the
    // worktree even though it exits 0.
    let (success_parallel, stdout_p, stderr_p) =
        run_gg(&repo_path, &["run", "-j", "2", "--json", "./dirty.sh"]);
    assert!(
        !success_parallel,
        "gg run -j 2 should fail when a command dirties the worktree: {} / {}",
        stdout_p, stderr_p
    );

    // Parse only the first JSON value — gg prints a second error-object
    // on non-zero exit which would confuse a whole-string parse.
    let mut stream = serde_json::Deserializer::from_str(&stdout_p).into_iter::<Value>();
    let parsed: Value = stream
        .next()
        .expect("expected at least one json object")
        .expect("first json object parse");
    let all_passed = parsed["run"]["all_passed"].as_bool().expect("all_passed");
    assert!(!all_passed, "all_passed should be false");

    let results = parsed["run"]["results"].as_array().expect("results");
    assert!(!results.is_empty(), "results should not be empty");
    for (idx, r) in results.iter().enumerate() {
        assert_eq!(
            r["passed"].as_bool(),
            Some(false),
            "commit at index {} should be marked failed (dirty worktree)",
            idx
        );
    }

    // Sequential parity check: `-j 1` must produce the same verdict so the
    // user can't get conflicting behavior by tweaking --jobs.
    let (success_seq, _, _) = run_gg(&repo_path, &["run", "-j", "1", "./dirty.sh"]);
    assert!(
        !success_seq,
        "sequential gg run must also fail — parallel should match sequential"
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_json_failure_emits_single_object() {
    // Regression: `gg run --json` used to print two JSON documents on failure
    // — first the {"run": ...} payload from execute(), then a
    // {"error": "Some commands failed"} payload from the generic main.rs
    // error path (because the handler converted Ok(false) into Err(...)).
    // Consumers expect a single parseable JSON document, so a failing
    // `gg run --json` must now emit exactly one object.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-json-failure-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("a.txt"), "a").expect("write a.txt");
    run_git(&repo_path, &["add", "a.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    // `false` exits non-zero on every unix, so the run will fail and we hit
    // the not-all-passed path.
    let (success_run, stdout, stderr) = run_gg(&repo_path, &["run", "--json", "false"]);
    assert!(
        !success_run,
        "gg run --json false should fail (exit 1): stdout={} stderr={}",
        stdout, stderr
    );

    // The critical assertion: exactly one JSON object in stdout.
    let mut stream = serde_json::Deserializer::from_str(&stdout).into_iter::<Value>();
    let first = stream
        .next()
        .expect("expected one json object")
        .expect("first json object must parse");
    assert!(
        first.get("run").is_some(),
        "first (and only) object must be the run payload, got: {}",
        first
    );
    let extra = stream.next();
    assert!(
        extra.is_none(),
        "expected exactly one JSON document in stdout, but got a second: {:?}",
        extra
    );

    // And the run payload itself should report the failure so consumers
    // can still distinguish success from failure without the second doc.
    assert_eq!(
        first["run"]["all_passed"].as_bool(),
        Some(false),
        "run.all_passed should be false on failure"
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_discard_resets_staged_index() {
    // Regression: `--discard` used to run `git checkout .` + `git clean -fd`,
    // which reverts tracked files and removes untracked files but does NOT
    // unstage anything the command added to the index. If a command ran
    // `git add`, those staged changes would persist into the next iteration
    // and could contaminate later commits or cause checkout failures.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Seed a tracked `tracked.txt` plus a script that modifies it AND stages
    // the modification — exercising the index path the old code missed.
    fs::write(repo_path.join("tracked.txt"), "original\n").expect("seed tracked.txt");
    fs::write(
        repo_path.join("stage.sh"),
        "#!/bin/sh\n\
         # Dirty the tree AND stage the change so the index carries it.\n\
         echo dirty >> tracked.txt\n\
         git add tracked.txt\n",
    )
    .expect("write stage.sh");
    let mut perms = fs::metadata(repo_path.join("stage.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("stage.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "tracked.txt", "stage.sh"]);
    run_git(&repo_path, &["commit", "-m", "seed tracked + stage script"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-discard-index-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Two commits so the discard happens at least once mid-stack, not just
    // at the final commit.
    fs::write(repo_path.join("f1.txt"), "v1").expect("write f1");
    run_git(&repo_path, &["add", "f1.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);

    fs::write(repo_path.join("f2.txt"), "v2").expect("write f2");
    run_git(&repo_path, &["add", "f2.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    let (success_run, stdout, stderr) = run_gg(&repo_path, &["run", "--discard", "./stage.sh"]);
    assert!(
        success_run,
        "gg run --discard should succeed: stdout={} stderr={}",
        stdout, stderr
    );

    // After discard, the working tree and index must both be clean —
    // `git status --porcelain` (which includes staged entries) should emit
    // nothing. Previously, the staged `tracked.txt` entry would still show.
    let status_out = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repo_path)
        .output()
        .expect("git status");
    assert!(
        status_out.status.success(),
        "git status failed: {}",
        String::from_utf8_lossy(&status_out.stderr)
    );
    assert!(
        status_out.stdout.is_empty(),
        "working tree + index must be clean after --discard, but got:\n{}",
        String::from_utf8_lossy(&status_out.stdout)
    );

    // And tracked.txt content should be back to the committed version.
    let tracked = fs::read_to_string(repo_path.join("tracked.txt")).unwrap();
    assert_eq!(
        tracked, "original\n",
        "tracked.txt must be restored to the committed state after --discard"
    );
}

#[cfg(unix)]
#[test]
fn test_gg_run_parallel_dirty_check_ignores_untracked() {
    // Regression: the parallel path used raw `git status --porcelain` which
    // includes untracked files, while the sequential path uses
    // `git::is_working_directory_clean` (include_untracked=false). A command
    // that created untracked files passed under `-j 1` but failed under
    // `-j N`, so `--jobs` could flip pass/fail for the same command.
    // The fix is to run `git status --porcelain --untracked-files=no` in the
    // parallel worker, matching the sequential semantics.
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Script only creates an untracked file — sequential accepts, parallel
    // must also accept (with the fix), and reject otherwise.
    fs::write(
        repo_path.join("make_untracked.sh"),
        "#!/bin/sh\necho hi > scratch.tmp\nexit 0\n",
    )
    .expect("write make_untracked.sh");
    let mut perms = fs::metadata(repo_path.join("make_untracked.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(repo_path.join("make_untracked.sh"), perms).unwrap();
    run_git(&repo_path, &["add", "make_untracked.sh"]);
    run_git(&repo_path, &["commit", "-m", "seed script"]);

    let (success, _, stderr) = run_gg(&repo_path, &["co", "run-untracked-parity-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Two-commit stack so -j 2 has real work.
    fs::write(repo_path.join("f1.txt"), "v1").expect("write f1");
    run_git(&repo_path, &["add", "f1.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 1"]);
    fs::write(repo_path.join("f2.txt"), "v2").expect("write f2");
    run_git(&repo_path, &["add", "f2.txt"]);
    run_git(&repo_path, &["commit", "-m", "Commit 2"]);

    // Sequential first: must succeed because the sequential dirty check
    // ignores untracked files.
    let (success_seq, stdout_s, stderr_s) =
        run_gg(&repo_path, &["run", "-j", "1", "./make_untracked.sh"]);
    assert!(
        success_seq,
        "sequential gg run must accept untracked-only dirtying: stdout={} stderr={}",
        stdout_s, stderr_s
    );

    // Parallel must match: also succeed.
    let (success_par, stdout_p, stderr_p) =
        run_gg(&repo_path, &["run", "-j", "2", "./make_untracked.sh"]);
    assert!(
        success_par,
        "parallel gg run must match sequential (accept untracked-only dirtying): stdout={} stderr={}",
        stdout_p, stderr_p
    );
}
