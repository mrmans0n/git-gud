//! `gg rebase` - Rebase the stack onto an updated base branch

use console::style;
use git2::Repository;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::immutability::{self, ImmutabilityPolicy};
use crate::operations::{self, OperationKind, SnapshotScope};
use crate::stack::{self, Stack};

/// Run the rebase command
pub fn run(target: Option<String>, force: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;

    // Acquire the operation lock for validation, but defer writing the
    // op-log record until after the immutability guard passes so refused
    // operations never pollute `gg undo --list` (design §4.6).
    let _lock = git::acquire_operation_lock(&repo, "rebase")?;

    // Run validation (fetch + immutability guard). This may mutate refs
    // via the fetch and local-branch fast-forward, but those are harmless
    // and don't need undo coverage.
    let target_branch = prepare_rebase(&repo, &config, target.clone(), false, force)?;

    // All validation passed — now write the Pending op-log record so a
    // failure beyond this point leaves a record the sweep can promote to
    // Interrupted.
    let guard = git::begin_recorded_op(
        &repo,
        &config,
        OperationKind::Rebase,
        std::env::args().skip(1).collect(),
        None,
        SnapshotScope::AllUserBranches,
    )?;

    match execute_rebase(&repo, &target_branch, false) {
        Ok(()) => guard.finalize_with_scope(
            &repo,
            &config,
            SnapshotScope::AllUserBranches,
            vec![],
            false,
        ),
        Err(GgError::RebaseConflict) => {
            let _ = operations::remember_interrupted_rebase_operation(&repo, guard.id());
            Err(GgError::RebaseConflict)
        }
        Err(e) => Err(e),
    }
}

/// Run rebase with an already-open repository (no lock acquisition)
pub fn run_with_repo(
    repo: &Repository,
    target: Option<String>,
    json: bool,
    force: bool,
) -> Result<()> {
    let config = Config::load_with_global(repo.commondir())?;
    let target_branch = prepare_rebase(repo, &config, target, json, force)?;
    execute_rebase(repo, &target_branch, json)
}

/// Validation phase: resolve target, fetch, update local base, run the
/// immutability guard. Returns the resolved target branch on success.
fn prepare_rebase(
    repo: &Repository,
    config: &Config,
    target: Option<String>,
    json: bool,
    force: bool,
) -> Result<String> {
    // Determine target branch. If no target provided, we need to be on a
    // stack to get the base branch.
    let target_branch = if let Some(t) = target {
        t
    } else {
        let stack = Stack::load(repo, config)?;
        stack.base.clone()
    };

    // Remember current branch to return to after updating base
    let current_branch = git::current_branch_name(repo);

    if !json {
        println!(
            "{}",
            style(format!("Updating {} and rebasing stack...", target_branch)).dim()
        );
    }

    // Fetch the latest from remote first. We want fresh origin/<base> for
    // both the immutability guard and the rebase itself — running the guard
    // against stale refs can silently pass on a newly-merged commit and
    // then rewrite it after the fetch updates the ref.
    let fetch_result = git::run_git_command(&["fetch", "origin", "--prune"]);
    let fetch_succeeded = fetch_result.is_ok();
    if let Err(e) = fetch_result {
        if !json {
            println!(
                "{} Could not fetch from origin: {}",
                style("Warning:").yellow(),
                e
            );
        }
    }

    // Update local base branch to match remote (fast-forward)
    // This ensures merged PRs are reflected in the local base
    let update_result = update_local_branch(&target_branch);
    if let Err(e) = update_result {
        if !json {
            println!(
                "{} Could not update local {}: {}",
                style("Warning:").yellow(),
                target_branch,
                e
            );
            println!("  Continuing with rebase onto origin/{}...", target_branch);
        }
    } else if !json {
        println!(
            "{} Updated local {} to latest",
            style("→").cyan(),
            target_branch
        );
    }

    // Return to stack branch if we switched away
    if let Some(ref branch) = current_branch {
        let _ = git::run_git_command(&["checkout", branch]);
    }

    // Immutability pre-flight: rebase rewrites every commit in the stack's
    // parent chain. If any commit is merged or already on the (freshly
    // fetched) base, refuse without --force. Must run *after* the fetch so
    // origin/<base> reflects the latest remote state.
    if let Ok(mut stack) = Stack::load(repo, config) {
        if !stack.is_empty() {
            // Best-effort refresh of mr_state so the guard catches
            // squash-merged PRs (their merge SHA isn't on origin/<base>, so
            // the base-ancestor rule misses them). No-op when offline.
            immutability::refresh_mr_state_for_guard(repo, &mut stack);
            let policy = ImmutabilityPolicy::for_stack(repo, &stack)?;
            let report = policy.check_all(&stack);
            let (report, dropped) = if fetch_succeeded && target_branch == stack.base {
                let pre_filter_count = report.entries.len();
                // Both filters require a fresh fetch: without_base_ancestors
                // needs up-to-date origin/<base> for ancestry checks, and
                // without_bottom_merged_prs needs it to confirm that the
                // contiguous bottom of the stack actually landed on the base.
                let filtered = report.without_bottom_merged_prs().without_base_ancestors();
                let dropped = pre_filter_count - filtered.entries.len();
                (filtered, dropped)
            } else {
                (report, 0)
            };
            if dropped > 0 && !json {
                println!(
                    "{} Skipping {} merged commit(s) already on {}",
                    style("→").cyan(),
                    dropped,
                    policy.base_ref()
                );
            }
            immutability::guard(report, force)?;
        }
    }

    Ok(target_branch)
}

/// Mutation phase: stash uncommitted changes, run `git rebase`, restore
/// stash. Assumes validation (fetch + immutability guard) has already run.
fn execute_rebase(repo: &Repository, target_branch: &str, json: bool) -> Result<()> {
    let current_branch = git::current_branch_name(repo);

    // Auto-stash uncommitted changes if present. Done after the guard so we
    // don't create a stash we'll have to restore if the guard rejects.
    let needs_stash = !git::is_working_directory_clean(repo)?;
    if needs_stash {
        if !json {
            println!("{}", style("Auto-stashing uncommitted changes...").dim());
        }
        git::run_git_command(&["stash", "push", "-m", "gg-rebase-autostash"])?;
    }

    // Perform the rebase
    let rebase_target = format!("origin/{}", target_branch);
    let rebase_result = git::run_git_command(&["rebase", &rebase_target]);

    match rebase_result {
        Ok(_) => {
            // In worktrees, rebase can leave HEAD detached; re-attach it.
            if let Some(ref branch) = current_branch {
                git::ensure_branch_attached(repo, branch)?;
            }

            if !json {
                println!(
                    "{} Rebased stack onto {}",
                    style("OK").green().bold(),
                    target_branch
                );
            }

            // Restore stashed changes if we stashed earlier
            if needs_stash {
                if !json {
                    println!("{}", style("Restoring stashed changes...").dim());
                }
                match git::run_git_command(&["stash", "pop"]) {
                    Ok(_) => {
                        if !json {
                            println!("{} Changes restored", style("→").cyan());
                        }
                    }
                    Err(e) => {
                        if !json {
                            println!(
                                "{} Could not restore stashed changes: {}",
                                style("Warning:").yellow(),
                                e
                            );
                            println!(
                                "  Your changes are in the stash. Run 'git stash pop' manually."
                            );
                        }
                    }
                }
            }

            Ok(())
        }
        Err(e) => {
            let error_str = e.to_string();
            if error_str.contains("CONFLICT") || error_str.contains("conflict") {
                if !json {
                    println!("{} Rebase conflict detected.", style("!").yellow().bold());
                    println!("  Resolve conflicts, then run `gg continue`");
                    println!("  Or run `gg abort` to cancel the rebase");

                    if needs_stash {
                        println!(
                            "  {}",
                            style("Note: Your uncommitted changes are stashed. They will be restored after the rebase completes.").dim()
                        );
                    }
                }

                Err(GgError::RebaseConflict)
            } else {
                // On other errors, try to restore stash
                if needs_stash {
                    if !json {
                        println!(
                            "{}",
                            style("Attempting to restore stashed changes...").dim()
                        );
                    }
                    let _ = git::run_git_command(&["stash", "pop"]);
                }
                Err(e)
            }
        }
    }
}

/// Update a local branch to match its remote counterpart (fast-forward only)
fn update_local_branch(branch: &str) -> Result<()> {
    // Check if the local branch exists
    let local_exists = git::run_git_command(&["rev-parse", "--verify", branch]).is_ok();

    if !local_exists {
        // Branch doesn't exist locally, nothing to update
        return Ok(());
    }

    // Check if remote branch exists
    let remote_ref = format!("origin/{}", branch);
    if git::run_git_command(&["rev-parse", "--verify", &remote_ref]).is_err() {
        // Remote branch doesn't exist
        return Ok(());
    }

    // Fast-forward local branch ref without checking it out.
    // This is worktree-safe because it avoids `git checkout <branch>`.
    if git::run_git_command(&["merge-base", "--is-ancestor", branch, &remote_ref]).is_err() {
        return Err(GgError::Other(format!(
            "Local {} has diverged from {}",
            branch, remote_ref
        )));
    }

    let remote_oid = git::run_git_command(&["rev-parse", &remote_ref])?;
    let local_ref = format!("refs/heads/{}", branch);
    git::run_git_command(&["update-ref", &local_ref, remote_oid.trim()])?;

    Ok(())
}

/// Continue a paused rebase
pub fn continue_rebase() -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;

    if !git::is_rebase_in_progress(&repo) {
        return Err(GgError::NoRebaseInProgress);
    }

    let _lock = git::acquire_operation_lock(&repo, "continue")?;

    // Check for unstaged changes before continuing
    let statuses = repo.statuses(None)?;
    let has_unstaged = statuses.iter().any(|s| {
        let flags = s.status();
        // Check for modified/deleted files that aren't staged
        flags.is_wt_modified() || flags.is_wt_deleted()
    });

    if has_unstaged {
        return Err(GgError::Other(
            "You have unstaged changes. Stage them with `git add` before running `gg continue`."
                .to_string(),
        ));
    }

    // Check for unresolved conflicts
    let has_conflicts = statuses.iter().any(|s| {
        let flags = s.status();
        flags.is_conflicted()
    });

    if has_conflicts {
        return Err(GgError::Other(
            "You have unresolved conflicts. Resolve them and stage with `git add` before running `gg continue`.".to_string()
        ));
    }

    let continued_operation = operations::interrupted_rebase_operation(&repo)?;

    match git::rebase_continue() {
        Ok(_) => {
            // If the paused rebase was a mid-stack integration (`gg restack`
            // folding in a detached commit), finish the integration-specific
            // cleanup that the conflict short-circuited: normalize GG metadata,
            // land HEAD back on the inserted commit, and rewrite the nav context.
            if let Some((branch_name, head_oid)) =
                crate::stack::read_pending_integration(repo.path())
            {
                let config = Config::load_with_global(repo.commondir())?;
                let (new_head_oid, stack_name) =
                    crate::commands::restack::finalize_detached_integration(
                        &repo,
                        &config,
                        &branch_name,
                        head_oid,
                    )?;
                crate::stack::clear_pending_integration(repo.path())?;

                let short = repo
                    .find_object(new_head_oid, None)?
                    .short_id()?
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                println!(
                    "{} Integrated mid-stack commit into stack {:?}; HEAD stays on {}",
                    style("OK").green().bold(),
                    stack_name,
                    style(&short).yellow()
                );
                println!(
                    "  {}",
                    style("Run `gg sync` to push the updated stack.").dim()
                );
                finalize_continued_operation(&repo, &config, continued_operation)?;
                return Ok(());
            }

            finalize_continued_operation(&repo, &config, continued_operation)?;
            println!(
                "{} Rebase continued successfully",
                style("OK").green().bold()
            );
            Ok(())
        }
        Err(e) => {
            let error_str = e.to_string();
            if error_str.contains("CONFLICT") || error_str.contains("conflict") {
                println!(
                    "{} More conflicts detected. Resolve and run `gg continue` again.",
                    style("!").yellow().bold()
                );
                Err(GgError::RebaseConflict)
            } else {
                // Provide more helpful error message
                eprintln!("{} Failed to continue rebase", style("Error:").red().bold());
                eprintln!("  {}", error_str);
                eprintln!();
                eprintln!("{}", style("You are still in rebase state.").yellow());
                eprintln!("  • Resolve any remaining issues");
                eprintln!("  • Run `git rebase --continue` manually to continue");
                eprintln!("  • Or run `gg abort` to cancel the rebase");
                eprintln!();
                eprintln!("  Hint: Run `git status` to see the current state");
                Err(e)
            }
        }
    }
}

/// Abort a paused rebase
pub fn abort_rebase() -> Result<()> {
    let repo = git::open_repo()?;

    if !git::is_rebase_in_progress(&repo) {
        return Err(GgError::NoRebaseInProgress);
    }

    git::rebase_abort()?;
    operations::clear_interrupted_rebase_operation(&repo)?;

    // Discard any pending mid-stack integration: the fold-in is cancelled.
    crate::stack::clear_pending_integration(repo.path())?;

    println!("{} Rebase aborted", style("OK").green().bold());

    Ok(())
}

fn finalize_continued_operation(
    repo: &Repository,
    config: &Config,
    operation: Option<operations::OperationRecord>,
) -> Result<()> {
    let Some(operation) = operation else {
        return Ok(());
    };

    if operation_needs_metadata_normalization_after_continue(operation.kind) {
        let rewritten_stack = Stack::load(repo, config)?;
        git::normalize_stack_metadata(repo, &rewritten_stack)?;
    }
    if operation.kind == OperationKind::Drop {
        cleanup_continued_drop_branches(repo, &operation);
    }
    if operation.kind == OperationKind::Split {
        restore_continued_split_navigation(repo, config, &operation)?;
    }
    if operation.kind == OperationKind::Squash {
        restore_continued_squash_navigation(repo, config, &operation)?;
    }

    operations::finalize_operation_by_id(
        repo,
        config,
        &operation.id,
        SnapshotScope::AllUserBranches,
        vec![],
        false,
    )?;
    operations::clear_interrupted_rebase_operation(repo)?;
    Ok(())
}

fn operation_needs_metadata_normalization_after_continue(kind: OperationKind) -> bool {
    matches!(
        kind,
        OperationKind::Drop
            | OperationKind::Reorder
            | OperationKind::Restack
            | OperationKind::Split
    )
}

fn restore_continued_split_navigation(
    repo: &Repository,
    config: &Config,
    operation: &operations::OperationRecord,
) -> Result<()> {
    let Some(split_plan) = operation
        .pending_plan
        .as_ref()
        .and_then(|plan| plan.get("split"))
    else {
        return Ok(());
    };

    let branch_name = split_plan
        .get("branch_name")
        .and_then(|branch| branch.as_str())
        .map(ToString::to_string);

    if let Some(branch_name) = &branch_name {
        git::ensure_branch_attached(repo, branch_name)?;
        git::checkout_branch(repo, branch_name)?;
    }

    let stack = Stack::load(repo, config)?;
    let Some(entry) = continued_split_target_entry(&stack, split_plan) else {
        return Ok(());
    };

    let branch_name = branch_name.unwrap_or_else(|| stack.branch_name());
    let entry_position = entry.position;
    let entry_oid = entry.oid;
    stack::save_nav_context(repo.path(), &branch_name, entry_position - 1, entry_oid)?;
    let commit = repo.find_commit(entry_oid)?;
    git::checkout_commit(repo, &commit)?;
    Ok(())
}

fn restore_continued_squash_navigation(
    repo: &Repository,
    config: &Config,
    operation: &operations::OperationRecord,
) -> Result<()> {
    let Some(squash_plan) = operation
        .pending_plan
        .as_ref()
        .and_then(|plan| plan.get("squash"))
    else {
        return Ok(());
    };

    let branch_name = squash_plan
        .get("branch_name")
        .and_then(|branch| branch.as_str())
        .map(ToString::to_string);

    if let Some(branch_name) = &branch_name {
        git::ensure_branch_attached(repo, branch_name)?;
        git::checkout_branch(repo, branch_name)?;
    }

    let stack = Stack::load(repo, config)?;
    let Some(entry) = continued_squash_target_entry(&stack, squash_plan) else {
        return Ok(());
    };

    let branch_name = branch_name.unwrap_or_else(|| stack.branch_name());
    let entry_position = entry.position;
    let entry_oid = entry.oid;
    stack::save_nav_context(repo.path(), &branch_name, entry_position - 1, entry_oid)?;
    let commit = repo.find_commit(entry_oid)?;
    git::checkout_commit(repo, &commit)?;
    Ok(())
}

fn continued_split_target_entry<'a>(
    stack: &'a Stack,
    split_plan: &serde_json::Value,
) -> Option<&'a stack::StackEntry> {
    continued_plan_target_entry(stack, split_plan, "remainder_gg_id", "remainder_position")
}

fn continued_squash_target_entry<'a>(
    stack: &'a Stack,
    squash_plan: &serde_json::Value,
) -> Option<&'a stack::StackEntry> {
    continued_plan_target_entry(stack, squash_plan, "target_gg_id", "target_position")
}

fn continued_plan_target_entry<'a>(
    stack: &'a Stack,
    plan: &serde_json::Value,
    gg_id_key: &str,
    position_key: &str,
) -> Option<&'a stack::StackEntry> {
    plan.get(gg_id_key)
        .and_then(|gg_id| gg_id.as_str())
        .and_then(|gg_id| stack.get_entry_by_gg_id(gg_id))
        .or_else(|| {
            plan.get(position_key)
                .and_then(|position| position.as_u64())
                .and_then(|position| usize::try_from(position).ok())
                .and_then(|position| stack.get_entry_by_position(position))
        })
}

fn cleanup_continued_drop_branches(repo: &Repository, operation: &operations::OperationRecord) {
    let Some(branches) = operation
        .pending_plan
        .as_ref()
        .and_then(|plan| plan.get("drop"))
        .and_then(|drop| drop.get("entry_branches"))
        .and_then(|branches| branches.as_array())
    else {
        return;
    };

    for branch_name in branches.iter().filter_map(|branch| branch.as_str()) {
        let _ = repo
            .find_branch(branch_name, git2::BranchType::Local)
            .and_then(|mut branch| branch.delete());
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::fs;
    use std::process::Command;

    use super::*;

    fn run_git_command(repo_path: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout={}\nstderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn fake_entry(position: usize, gg_id: Option<&str>) -> stack::StackEntry {
        stack::StackEntry {
            oid: git2::Oid::from_str(&format!("{position:040x}")).unwrap(),
            short_sha: format!("{position:07x}"),
            title: format!("Commit {position}"),
            gg_id: gg_id.map(ToString::to_string),
            gg_parent: None,
            mr_number: None,
            mr_state: None,
            approved: false,
            changes_requested: false,
            mergeable: false,
            ci_status: None,
            position,
            in_merge_train: false,
            merge_train_position: None,
        }
    }

    fn fake_stack() -> Stack {
        Stack {
            name: "test".to_string(),
            username: "user".to_string(),
            base: "main".to_string(),
            entries: vec![
                fake_entry(1, Some("c-one111")),
                fake_entry(2, Some("c-two222")),
                fake_entry(3, Some("c-three3")),
            ],
            current_position: Some(2),
        }
    }

    #[test]
    fn continued_split_navigation_reattaches_branch_before_loading_stack() {
        let temp_dir = tempfile::tempdir().unwrap();
        let repo_path = temp_dir.path().join("repo");
        fs::create_dir(&repo_path).unwrap();

        run_git_command(&repo_path, &["init", "--initial-branch=main"]);
        run_git_command(&repo_path, &["config", "user.email", "test@test.com"]);
        run_git_command(&repo_path, &["config", "user.name", "Test User"]);
        fs::write(repo_path.join("README.md"), "base\n").unwrap();
        run_git_command(&repo_path, &["add", "."]);
        run_git_command(&repo_path, &["commit", "-m", "Initial"]);
        run_git_command(&repo_path, &["checkout", "-b", "testuser/split-wt"]);

        fs::write(repo_path.join("one.txt"), "one\n").unwrap();
        run_git_command(&repo_path, &["add", "."]);
        run_git_command(&repo_path, &["commit", "-m", "Commit 1\n\nGG-ID: c-one111"]);
        fs::write(repo_path.join("two.txt"), "two\n").unwrap();
        run_git_command(&repo_path, &["add", "."]);
        run_git_command(&repo_path, &["commit", "-m", "Commit 2\n\nGG-ID: c-two222"]);

        let repo = Repository::open(&repo_path).unwrap();
        let branch_tip = repo
            .revparse_single("refs/heads/testuser/split-wt")
            .unwrap()
            .peel_to_commit()
            .unwrap();
        repo.set_head_detached(branch_tip.id()).unwrap();
        assert!(repo.head_detached().unwrap());

        let mut config = Config::default();
        config.defaults.base = Some("main".to_string());
        config.defaults.branch_username = Some("testuser".to_string());
        let operation = operations::OperationRecord {
            id: operations::new_id(),
            schema_version: operations::SCHEMA_VERSION,
            kind: OperationKind::Split,
            status: operations::OperationStatus::Pending,
            created_at_ms: operations::now_ms(),
            args: vec!["split".to_string()],
            stack_name: Some("split-wt".to_string()),
            refs_before: vec![],
            refs_after: vec![],
            remote_effects: vec![],
            touched_remote: false,
            undoes: None,
            pending_plan: Some(json!({
                "split": {
                    "branch_name": "testuser/split-wt",
                    "remainder_position": 2,
                    "remainder_gg_id": "c-two222",
                }
            })),
        };

        let previous_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&repo_path).unwrap();
        let result = restore_continued_split_navigation(&repo, &config, &operation);
        std::env::set_current_dir(previous_dir).unwrap();

        result.unwrap();
        let (branch_name, position, oid) = stack::read_nav_context(repo.path()).unwrap();
        assert_eq!(branch_name, "testuser/split-wt");
        assert_eq!(position, 1);
        assert_eq!(oid, branch_tip.id());
    }

    #[test]
    fn continued_split_target_prefers_remainder_gg_id() {
        let stack = fake_stack();
        let split_plan = json!({
            "remainder_position": 2,
            "remainder_gg_id": "c-three3",
        });

        let entry = continued_split_target_entry(&stack, &split_plan).unwrap();

        assert_eq!(entry.position, 3);
    }

    #[test]
    fn continued_split_target_falls_back_to_remainder_position() {
        let stack = fake_stack();
        let split_plan = json!({
            "remainder_position": 2,
            "remainder_gg_id": null,
        });

        let entry = continued_split_target_entry(&stack, &split_plan).unwrap();

        assert_eq!(entry.gg_id.as_deref(), Some("c-two222"));
    }

    #[test]
    fn continued_squash_target_prefers_target_gg_id() {
        let stack = fake_stack();
        let squash_plan = json!({
            "target_position": 2,
            "target_gg_id": "c-three3",
        });

        let entry = continued_squash_target_entry(&stack, &squash_plan).unwrap();

        assert_eq!(entry.position, 3);
    }

    #[test]
    fn continued_squash_target_falls_back_to_target_position() {
        let stack = fake_stack();
        let squash_plan = json!({
            "target_position": 2,
            "target_gg_id": null,
        });

        let entry = continued_squash_target_entry(&stack, &squash_plan).unwrap();

        assert_eq!(entry.gg_id.as_deref(), Some("c-two222"));
    }
}
