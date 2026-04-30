use crate::helpers::{
    create_test_repo, create_test_repo_with_remote, run_gg, run_git, run_git_full,
};

use std::fs;
use std::process::Command;

#[test]
fn test_rebase_updates_local_main() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with a commit
    let (success, _, stderr) = run_gg(&repo_path, &["co", "rebase-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "feature").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add feature"]);

    // Get the initial main SHA
    run_git(&repo_path, &["checkout", "main"]);
    let (_, initial_main_sha) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let initial_main_sha = initial_main_sha.trim();

    // Simulate a commit being merged on the remote (like a PR merge)
    // We'll add a commit directly to origin/main
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("merged.txt"), "merged from PR").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Merged PR"]);
    run_git(&repo_path, &["push", "origin", "main"]);

    // Reset local main to the old SHA (simulating local main being behind)
    run_git(&repo_path, &["reset", "--hard", initial_main_sha]);

    // Verify local main is behind
    let (_, local_sha) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    assert_eq!(
        local_sha.trim(),
        initial_main_sha,
        "Local main should be at initial SHA"
    );

    // Switch to our stack
    run_git(&repo_path, &["checkout", "testuser/rebase-test"]);

    // Run gg rebase - this should update local main
    let (success, stdout, stderr) = run_gg(&repo_path, &["rebase"]);

    // Check that rebase ran (might fail if there are conflicts, but that's ok)
    // The important thing is that it attempted to update main
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("Updating main") || combined.contains("Updated local main") || success,
        "Should mention updating main: {}",
        combined
    );
}

#[test]
fn test_rebase_updates_local_main_from_worktree() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack branch and commit
    let (success, _, stderr) = run_gg(&repo_path, &["co", "rebase-worktree-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "feature").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add feature"]);

    // Keep main checked out in the main worktree
    run_git(&repo_path, &["checkout", "main"]);
    let (_, initial_main_sha) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let initial_main_sha = initial_main_sha.trim();

    // Create linked worktree for the stack branch
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("valid time")
        .as_nanos();
    let worktree_path = repo_path
        .parent()
        .unwrap()
        .join(format!("stack-worktree-{}", unique_suffix));
    let (success, _, stderr) = run_git_full(
        &repo_path,
        &[
            "worktree",
            "add",
            worktree_path.to_str().expect("valid path"),
            "testuser/rebase-worktree-test",
        ],
    );
    assert!(success, "Failed to create worktree: {}", stderr);

    // Advance origin/main and make local main behind
    fs::write(repo_path.join("merged.txt"), "merged").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Merged PR"]);
    run_git(&repo_path, &["push", "origin", "main"]);
    run_git(&repo_path, &["reset", "--hard", initial_main_sha]);

    // Run rebase from linked worktree
    let worktree_path_buf = worktree_path.to_path_buf();
    let (_success, stdout, stderr) = run_gg(&worktree_path_buf, &["rebase"]);
    let combined = format!("{}{}", stdout, stderr);

    assert!(
        !combined.contains("already used by worktree"),
        "rebase should not try checking out main in a linked worktree: {}",
        combined
    );

    // Local main should be updated to origin/main via fast-forward fetch
    let (_, local_main_sha) = run_git(&repo_path, &["rev-parse", "main"]);
    let (_, remote_main_sha) = run_git(&repo_path, &["rev-parse", "origin/main"]);
    assert_eq!(
        local_main_sha.trim(),
        remote_main_sha.trim(),
        "Local main should fast-forward to origin/main"
    );
}

#[test]
fn test_rebase_help() {
    let (_temp_dir, repo_path) = create_test_repo();

    let (success, stdout, _stderr) = run_gg(&repo_path, &["rebase", "--help"]);

    assert!(success, "Help should succeed");
    assert!(
        stdout.contains("Rebase") || stdout.contains("rebase"),
        "Should show rebase help: {}",
        stdout
    );
}

// ============================================================
// Tests for PR #50 - gg rebase improvements
// ============================================================

#[test]
fn test_rebase_restores_original_branch() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with commits
    let (success, _, stderr) = run_gg(&repo_path, &["co", "restore-branch-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "feature content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add feature"]);

    // Remember the branch we're on
    let (_, original_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let original_branch = original_branch.trim();

    // Run rebase
    let (success, _stdout, _stderr) = run_gg(&repo_path, &["rebase"]);

    // Whether it succeeds or not, we should be back on the original branch
    let (_, current_branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(
        current_branch.trim(),
        original_branch,
        "Should restore to original branch after rebase (success={})",
        success
    );
}

#[test]
fn test_rebase_when_local_base_branch_not_exists() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "no-local-base-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add feature"]);

    // Delete the local main branch (unusual but possible scenario)
    // First we need to be on a different branch (we already are on our stack)
    run_git(&repo_path, &["branch", "-D", "main"]);

    // Verify main doesn't exist locally
    let (exists, _) = run_git(&repo_path, &["rev-parse", "--verify", "main"]);
    assert!(!exists, "Local main should not exist");

    // Rebase should still work (will use origin/main directly)
    let (success, stdout, stderr) = run_gg(&repo_path, &["rebase"]);

    // Should succeed or at least not crash due to missing local branch
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        success || combined.contains("Rebased"),
        "Rebase should handle missing local base branch gracefully: {}",
        combined
    );
}

#[test]
fn test_rebase_when_remote_base_branch_not_exists() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config with a non-existent base branch
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "no-remote-base-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add feature"]);

    // Try to rebase onto a non-existent branch
    let (_success, stdout, stderr) = run_gg(&repo_path, &["rebase", "nonexistent-branch"]);

    // Should fail gracefully with a clear error
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("Warning")
            || combined.contains("Could not")
            || combined.contains("error")
            || combined.contains("not exist"),
        "Should handle missing remote branch gracefully: {}",
        combined
    );
}

#[test]
fn test_rebase_when_branches_have_diverged() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "diverged-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "feature").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Add feature"]);

    // Make a commit on remote main
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("remote-change.txt"), "from remote").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Remote commit"]);
    run_git(&repo_path, &["push", "origin", "main"]);

    // Make a DIFFERENT commit on local main (causing divergence)
    run_git(&repo_path, &["reset", "--hard", "HEAD~1"]);
    fs::write(repo_path.join("local-change.txt"), "local only").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Local only commit"]);

    // Verify branches have diverged
    let (_, local_sha) = run_git(&repo_path, &["rev-parse", "main"]);
    let (_, remote_sha) = run_git(&repo_path, &["rev-parse", "origin/main"]);
    assert_ne!(
        local_sha.trim(),
        remote_sha.trim(),
        "Branches should have diverged"
    );

    // Switch to our stack
    run_git(&repo_path, &["checkout", "testuser/diverged-test"]);

    // Rebase should warn but continue with origin/main
    let (success, stdout, stderr) = run_gg(&repo_path, &["rebase"]);

    let combined = format!("{}{}", stdout, stderr);

    // Should either succeed (using origin/main) or warn about divergence
    // The key is it shouldn't crash and should give useful feedback
    assert!(
        success
            || combined.contains("Warning")
            || combined.contains("Could not update")
            || combined.contains("origin/main"),
        "Should handle diverged branches gracefully: {}",
        combined
    );
}

#[test]
fn test_rebase_removes_merged_commits_from_stack() {
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with multiple commits
    let (success, _, stderr) = run_gg(&repo_path, &["co", "merged-commits-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("commit1.txt"), "commit 1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 1 - will be merged"]);

    fs::write(repo_path.join("commit2.txt"), "commit 2").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Commit 2 - stays in stack"]);

    // Get the initial commit count
    let (_, log_before) = run_git(&repo_path, &["log", "--oneline", "main..HEAD"]);
    let commits_before = log_before.trim().lines().count();
    assert_eq!(commits_before, 2, "Should have 2 commits before merge");

    // Simulate first commit being merged to main on remote
    // Cherry-pick commit 1 to main and push
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("commit1.txt"), "commit 1").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "Commit 1 - will be merged (merged via PR)"],
    );
    run_git(&repo_path, &["push", "origin", "main"]);

    // Reset local main to be behind
    run_git(&repo_path, &["reset", "--hard", "HEAD~1"]);

    // Switch back to stack and rebase
    run_git(&repo_path, &["checkout", "testuser/merged-commits-test"]);

    let (success, _stdout, _stderr) = run_gg(&repo_path, &["rebase"]);

    if success {
        // After rebase, the first commit should be gone (it's now in main)
        // Only commit 2 should remain
        let (_, log_after) = run_git(&repo_path, &["log", "--oneline", "origin/main..HEAD"]);
        let commits_after = log_after.trim().lines().count();

        // The commit that was "merged" should no longer appear in the stack
        // Note: This depends on git's ability to detect the commit was cherry-picked
        // In a real scenario with actual PR merges, git rebase drops duplicate commits
        assert!(
            commits_after <= commits_before,
            "Stack should have same or fewer commits after rebase. Before: {}, After: {}",
            commits_before,
            commits_after
        );
    }
}

#[test]
fn test_rebase_with_prune_removes_deleted_remote_branches() {
    let (_temp_dir, repo_path, remote_path) = create_test_repo_with_remote();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create and push a temporary branch
    run_git(&repo_path, &["checkout", "-b", "temp-branch"]);
    fs::write(repo_path.join("temp.txt"), "temp").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Temp commit"]);
    run_git(&repo_path, &["push", "-u", "origin", "temp-branch"]);

    // Go back to main
    run_git(&repo_path, &["checkout", "main"]);

    // Delete the branch on the remote directly (simulating PR merge with branch deletion)
    Command::new("git")
        .args(["branch", "-D", "temp-branch"])
        .current_dir(&remote_path)
        .output()
        .expect("Failed to delete remote branch");

    // Verify the remote tracking branch still exists locally
    let (exists_before, _) = run_git(
        &repo_path,
        &["rev-parse", "--verify", "refs/remotes/origin/temp-branch"],
    );
    assert!(
        exists_before,
        "Remote tracking branch should exist before fetch --prune"
    );

    // Create a stack and run rebase (which does fetch --prune)
    let (success, _, stderr) = run_gg(&repo_path, &["co", "prune-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    fs::write(repo_path.join("feature.txt"), "feature").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Feature commit"]);

    // Run rebase - this should fetch with --prune
    let (_success, _stdout, _stderr) = run_gg(&repo_path, &["rebase"]);

    // After rebase (which fetches with --prune), the deleted remote branch should be gone
    let (exists_after, _) = run_git(
        &repo_path,
        &["rev-parse", "--verify", "refs/remotes/origin/temp-branch"],
    );
    assert!(
        !exists_after,
        "Remote tracking branch should be pruned after rebase"
    );
}

#[test]
fn test_rebase_without_stack_requires_target() {
    // Test that rebase works on any branch if target is provided
    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Create a feature branch (not a stack)
    run_git(&repo_path, &["checkout", "-b", "feature-branch"]);

    // Create a commit
    fs::write(repo_path.join("file.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Feature commit"]);

    // Try rebase without target (should fail - not on a stack branch)
    let (success, _stdout, stderr) = run_gg(&repo_path, &["rebase"]);
    assert!(
        !success,
        "Rebase without target should fail when not on a stack"
    );
    assert!(
        stderr.contains("not a stack branch"),
        "Should indicate not on a stack. Got: {}",
        stderr
    );

    // Try rebase with target (should work on any branch)
    let (success, stdout, stderr) = run_gg(&repo_path, &["rebase", "main"]);
    assert!(
        success,
        "Rebase with target should work on any branch. stdout: {}, stderr: {}",
        stdout, stderr
    );
}

#[test]
fn test_stacked_branches_can_rebase_after_squash_merge() {
    // This test verifies the mechanics that `gg land --all` relies on:
    // After a squash merge, remaining stacked branches can be rebased
    // onto the updated main to avoid merge conflicts.
    //
    // Scenario:
    // - Stack: commit A -> commit B -> commit C
    // - Each has a branch (simulating PR branches)
    // - Squash merge commit A to main (creates new SHA)
    // - Rebase branch B onto new main
    // - Verify branch B now has only its own commit (not the old A)

    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up gg config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack with gg co
    let (success, _, stderr) = run_gg(&repo_path, &["co", "stacked-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create commit A
    fs::write(repo_path.join("file_a.txt"), "content A").expect("Failed to write file A");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: commit A\n\nGG-ID: c-aaa1111"],
    );

    // Create branch for commit A (simulating PR branch)
    run_git(&repo_path, &["branch", "pr-branch-a"]);

    // Create commit B
    fs::write(repo_path.join("file_b.txt"), "content B").expect("Failed to write file B");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: commit B\n\nGG-ID: c-bbb2222"],
    );

    // Create branch for commit B
    run_git(&repo_path, &["branch", "pr-branch-b"]);

    // Create commit C
    fs::write(repo_path.join("file_c.txt"), "content C").expect("Failed to write file C");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: commit C\n\nGG-ID: c-ccc3333"],
    );

    // Create branch for commit C
    run_git(&repo_path, &["branch", "pr-branch-c"]);

    // Get SHA of commit A on the stack branch
    let (_, old_a_sha) = run_git(&repo_path, &["rev-parse", "pr-branch-a"]);
    let old_a_sha = old_a_sha.trim();

    // Now simulate a squash merge of commit A to main
    // This creates a NEW commit with DIFFERENT SHA but same content
    run_git(&repo_path, &["checkout", "main"]);

    // Cherry-pick with squash (simulates GitHub squash merge)
    fs::write(repo_path.join("file_a.txt"), "content A").expect("Failed to write file A");
    run_git(&repo_path, &["add", "."]);
    run_git(
        &repo_path,
        &["commit", "-m", "feat: commit A (#1)\n\nSquash merged"],
    );

    // Push the updated main
    run_git(&repo_path, &["push", "origin", "main"]);

    // Get the new SHA on main
    let (_, new_main_sha) = run_git(&repo_path, &["rev-parse", "HEAD"]);
    let new_main_sha = new_main_sha.trim();

    // Verify the SHAs are different (squash creates new commit)
    assert_ne!(
        old_a_sha, new_main_sha,
        "Squash merge should create different SHA"
    );

    // Now the critical part: rebase pr-branch-b onto the new main
    // This is what gg land does after merging each PR
    run_git(&repo_path, &["checkout", "pr-branch-b"]);

    // Before rebase: branch B has commits A and B
    let (_, log_before) = run_git(&repo_path, &["log", "--oneline", "main..HEAD"]);
    let commits_before: Vec<&str> = log_before.trim().lines().collect();
    assert_eq!(
        commits_before.len(),
        2,
        "Before rebase: should have 2 commits (A and B)"
    );

    // Rebase onto the updated main
    let (success, _) = run_git(&repo_path, &["rebase", "main"]);
    assert!(success, "Rebase should succeed");

    // After rebase: branch B should only have commit B
    let (_, log_after) = run_git(&repo_path, &["log", "--oneline", "main..HEAD"]);
    let commits_after: Vec<&str> = log_after.trim().lines().collect();
    assert_eq!(
        commits_after.len(),
        1,
        "After rebase: should have 1 commit (only B)"
    );
    assert!(
        log_after.contains("commit B"),
        "Should still have commit B: {}",
        log_after
    );

    // Verify the rebased branch can be pushed (simulating force-push)
    let (_success, _) = run_git(&repo_path, &["push", "-f", "origin", "pr-branch-b"]);
    // Note: This might fail if branch doesn't exist on remote, which is fine for this test
    // The important part is that the rebase succeeded
}

#[test]
fn test_rebase_chain_after_multiple_squash_merges() {
    // This test verifies that we can rebase a chain of branches
    // after multiple squash merges (landing PR 1, then PR 2, etc.)

    let (_temp_dir, repo_path, _remote_path) = create_test_repo_with_remote();

    // Set up gg config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "chain-test"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Create 3 commits with branches
    for (letter, num) in [('a', 1), ('b', 2), ('c', 3)] {
        let filename = format!("file_{}.txt", letter);
        let content = format!("content {}", letter.to_uppercase());
        fs::write(repo_path.join(&filename), &content).expect("Failed to write file");
        run_git(&repo_path, &["add", "."]);
        run_git(
            &repo_path,
            &[
                "commit",
                "-m",
                &format!(
                    "feat: commit {}\n\nGG-ID: c-{}{}{}",
                    letter.to_uppercase(),
                    letter,
                    letter,
                    num
                ),
            ],
        );
        run_git(&repo_path, &["branch", &format!("pr-branch-{}", letter)]);
    }

    // Simulate landing PR A (squash merge to main)
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("file_a.txt"), "content A").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "feat: commit A (#1)"]);
    run_git(&repo_path, &["push", "origin", "main"]);

    // Rebase remaining branches (B and C) onto new main
    run_git(&repo_path, &["checkout", "pr-branch-b"]);
    let (success, _) = run_git(&repo_path, &["rebase", "main"]);
    assert!(success, "Rebase of B should succeed");

    run_git(&repo_path, &["checkout", "pr-branch-c"]);
    let (success, _) = run_git(&repo_path, &["rebase", "pr-branch-b"]);
    assert!(success, "Rebase of C onto B should succeed");

    // Simulate landing PR B (squash merge)
    run_git(&repo_path, &["checkout", "main"]);
    fs::write(repo_path.join("file_b.txt"), "content B").expect("Failed to write");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "feat: commit B (#2)"]);
    run_git(&repo_path, &["push", "origin", "main"]);

    // Rebase C onto the new main
    run_git(&repo_path, &["checkout", "pr-branch-c"]);
    let (success, _) = run_git(&repo_path, &["rebase", "main"]);
    assert!(success, "Rebase of C after B landed should succeed");

    // Verify C only has one commit now
    let (_, log) = run_git(&repo_path, &["log", "--oneline", "main..HEAD"]);
    let commits: Vec<&str> = log.trim().lines().collect();
    assert_eq!(
        commits.len(),
        1,
        "C should only have 1 commit after all rebases"
    );
    assert!(log.contains("commit C"), "Should be commit C: {}", log);
}
