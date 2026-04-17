//! `gg restack` — Detect and repair ancestry drift in a stack.
//!
//! After manual git operations (amend, cherry-pick, interactive rebase),
//! the Git parent chain can diverge from the GG-Parent metadata chain.
//! Restack compares the two, builds a plan, and executes a single
//! `git rebase -i` to realign them.

use std::path::Path;

use console::style;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::immutability::{self, ImmutabilityPolicy};
use crate::output::{
    print_json, RestackResponse, RestackResultJson, RestackStepJson, OUTPUT_VERSION,
};
use crate::stack::{self, Stack};

/// Options for the restack command.
#[derive(Debug, Default)]
pub struct RestackOptions {
    /// Show plan without executing.
    pub dry_run: bool,
    /// Starting position/SHA/GG-ID — only repair from this entry upward.
    pub from: Option<String>,
    /// Override the immutability check for merged/base-ancestor commits.
    pub force: bool,
    /// Output as JSON.
    pub json: bool,
}

/// What restack will do (or did) to a single entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestackAction {
    /// Ancestry matches — included in rebase for continuity.
    Ok,
    /// Ancestry broken — rebase will fix.
    Reattach,
}

/// One entry in the restack plan.
#[derive(Debug, Clone)]
pub struct RestackStep {
    pub position: usize,
    pub gg_id: Option<String>,
    pub short_sha: String,
    pub full_sha: String,
    pub title: String,
    pub action: RestackAction,
    pub current_parent: Option<String>,
    pub expected_parent: Option<String>,
}

/// The full restack plan — public so `gg reparent` (Task 5) can reuse it.
#[derive(Debug, Clone)]
pub struct RestackPlan {
    pub stack_name: String,
    pub base_oid: git2::Oid,
    pub steps: Vec<RestackStep>,
}

impl RestackPlan {
    /// Number of entries that need reattaching.
    pub fn reattach_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| s.action == RestackAction::Reattach)
            .count()
    }

    /// True when every entry's ancestry already matches.
    pub fn is_consistent(&self) -> bool {
        self.reattach_count() == 0
    }
}

/// Build a restack plan by comparing GG-Parent trailers to expected values.
///
/// Public so `gg reparent` can reuse the detection + execution pipeline.
pub fn build_plan(
    stack: &Stack,
    from_position: Option<usize>,
    base_oid: git2::Oid,
) -> Result<RestackPlan> {
    let start = from_position.unwrap_or(1);

    let steps: Vec<RestackStep> = stack
        .entries
        .iter()
        .filter(|e| e.position >= start)
        .map(|entry| {
            let expected = stack.expected_parent_gg_id(entry.position);
            let current = entry.gg_parent.as_deref();
            let action = if current == expected {
                RestackAction::Ok
            } else {
                RestackAction::Reattach
            };

            RestackStep {
                position: entry.position,
                gg_id: entry.gg_id.clone(),
                short_sha: entry.short_sha.clone(),
                full_sha: entry.oid.to_string(),
                title: entry.title.clone(),
                action,
                current_parent: current.map(String::from),
                expected_parent: expected.map(String::from),
            }
        })
        .collect();

    // Determine the actual rebase base OID.
    // If --from N and N > 1, base is the OID of entry at position N-1.
    // Otherwise, use the stack base.
    let effective_base = if start > 1 {
        stack
            .get_entry_by_position(start - 1)
            .map(|e| e.oid)
            .unwrap_or(base_oid)
    } else {
        base_oid
    };

    Ok(RestackPlan {
        stack_name: stack.name.clone(),
        base_oid: effective_base,
        steps,
    })
}

/// Execute a restack plan via `git rebase -i` with GIT_SEQUENCE_EDITOR.
fn execute_plan(plan: &RestackPlan, workdir: &Path) -> Result<()> {
    use std::io::Write;

    // Build the rebase todo — one `pick` per step, in order
    let mut rebase_todo = String::new();
    for step in &plan.steps {
        rebase_todo.push_str(&format!("pick {}\n", step.full_sha));
    }

    let unique_id = std::process::id();
    let todo_file = std::env::temp_dir().join(format!("gg-restack-todo-{}", unique_id));
    std::fs::write(&todo_file, &rebase_todo)?;

    let editor_script = format!("#!/bin/sh\ncat {} > \"$1\"", todo_file.display());
    let script_file = std::env::temp_dir().join(format!("gg-restack-editor-{}.sh", unique_id));
    {
        let mut f = std::fs::File::create(&script_file)?;
        f.write_all(editor_script.as_bytes())?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_file)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_file, perms)?;
    }

    let output = std::process::Command::new("git")
        .current_dir(workdir)
        .env("GIT_SEQUENCE_EDITOR", script_file.to_str().unwrap())
        .args(["rebase", "-i", &plan.base_oid.to_string(), "--keep-empty"])
        .output()?;

    let _ = std::fs::remove_file(&todo_file);
    let _ = std::fs::remove_file(&script_file);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("CONFLICT") || stderr.contains("conflict") {
            return Err(GgError::RebaseConflict);
        }
        return Err(GgError::Other(format!("Rebase failed: {}", stderr)));
    }

    Ok(())
}

/// Run the restack command.
pub fn run(options: RestackOptions) -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;

    // Acquire operation lock
    let _lock = git::acquire_operation_lock(&repo, "restack")?;

    // Guard: rebase already in progress
    let git_dir = repo.path();
    if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        return Err(GgError::Other(
            "A rebase is already in progress. Run `gg continue` or `gg abort` first.".to_string(),
        ));
    }

    // Require clean working directory
    git::require_clean_working_directory(&repo)?;

    // Load stack
    let mut stack = Stack::load(&repo, &config)?;
    // Best-effort refresh of mr_state for the immutability guard (catches
    // squash-merged PRs that base-ancestor would otherwise miss).
    immutability::refresh_mr_state_for_guard(&repo, &mut stack);

    if stack.is_empty() {
        if options.json {
            print_json(&RestackResponse {
                version: OUTPUT_VERSION,
                restack: RestackResultJson {
                    stack_name: stack.name.clone(),
                    total_entries: 0,
                    entries_restacked: 0,
                    entries_ok: 0,
                    dry_run: options.dry_run,
                    steps: vec![],
                },
            });
        } else {
            println!("Stack is empty — nothing to restack.");
        }
        return Ok(());
    }

    // Resolve --from target
    let from_position = match &options.from {
        Some(target) => Some(stack::resolve_target(&stack, target)?),
        None => None,
    };

    // Resolve base OID
    let base_ref = repo
        .revparse_single(&stack.base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", stack.base)))?;
    let base_oid = base_ref.id();

    // Build plan
    let plan = build_plan(&stack, from_position, base_oid)?;

    // Build JSON steps (used for both dry-run and post-execution output)
    let json_steps: Vec<RestackStepJson> = plan
        .steps
        .iter()
        .map(|s| RestackStepJson {
            position: s.position,
            gg_id: s.gg_id.clone(),
            title: s.title.clone(),
            action: match s.action {
                RestackAction::Ok => "ok".to_string(),
                RestackAction::Reattach => "reattach".to_string(),
            },
            current_parent: s.current_parent.clone(),
            expected_parent: s.expected_parent.clone(),
        })
        .collect();

    // Early exit: already consistent
    if plan.is_consistent() {
        if options.json {
            print_json(&RestackResponse {
                version: OUTPUT_VERSION,
                restack: RestackResultJson {
                    stack_name: plan.stack_name.clone(),
                    total_entries: plan.steps.len(),
                    entries_restacked: 0,
                    entries_ok: plan.steps.len(),
                    dry_run: options.dry_run,
                    steps: json_steps,
                },
            });
        } else {
            println!(
                "{} Stack {} is already consistent.",
                style("✓").green().bold(),
                style(&plan.stack_name).cyan()
            );
        }
        return Ok(());
    }

    // Dry-run: display plan and exit
    if options.dry_run {
        if options.json {
            print_json(&RestackResponse {
                version: OUTPUT_VERSION,
                restack: RestackResultJson {
                    stack_name: plan.stack_name.clone(),
                    total_entries: plan.steps.len(),
                    entries_restacked: plan.reattach_count(),
                    entries_ok: plan.steps.len() - plan.reattach_count(),
                    dry_run: true,
                    steps: json_steps,
                },
            });
        } else {
            println!(
                "{} Restack plan for {}:",
                style("dry-run").yellow().bold(),
                style(&plan.stack_name).cyan()
            );
            for step in &plan.steps {
                let marker = match step.action {
                    RestackAction::Ok => style("  ok").dim(),
                    RestackAction::Reattach => style("  ✗ reattach").red().bold(),
                };
                println!(
                    "  {} {} {} {}",
                    style(format!("#{}", step.position)).dim(),
                    step.short_sha,
                    step.title,
                    marker
                );
            }
            println!(
                "\n  {} entries need repair, {} already consistent.",
                plan.reattach_count(),
                plan.steps.len() - plan.reattach_count()
            );
        }
        return Ok(());
    }

    // Immutability pre-flight: restack rewrites every entry in the plan,
    // so check the affected range for merged/base-ancestor commits.
    {
        let targets: Vec<usize> = plan.steps.iter().map(|s| s.position).collect();
        let policy = ImmutabilityPolicy::for_stack(&repo, &stack)?;
        let report = policy.check_positions(&stack, &targets);
        immutability::guard(report, options.force)?;
    }

    // Execute
    let workdir = repo
        .workdir()
        .ok_or_else(|| GgError::Other("Cannot restack in a bare repository.".to_string()))?;
    execute_plan(&plan, workdir)?;

    // Normalize metadata post-rebase, scoped to the affected range.
    // When --from is used, only normalize entries at or above from_position
    // to avoid rewriting history below the boundary. We pass the predecessor's
    // GG-ID so the first restacked entry's GG-Parent is set correctly.
    let mut rewritten_stack = Stack::load(&repo, &config)?;
    let predecessor_gg_id = from_position.and_then(|from_pos| {
        rewritten_stack
            .get_entry_by_position(from_pos.saturating_sub(1))
            .and_then(|e| e.gg_id.clone())
    });
    if let Some(from_pos) = from_position {
        rewritten_stack.entries.retain(|e| e.position >= from_pos);
    }
    git::normalize_stack_metadata_with_predecessor(
        &repo,
        &rewritten_stack,
        predecessor_gg_id.as_deref(),
    )?;

    // Output
    let reattach_count = plan.reattach_count();
    let ok_count = plan.steps.len() - reattach_count;

    if options.json {
        print_json(&RestackResponse {
            version: OUTPUT_VERSION,
            restack: RestackResultJson {
                stack_name: plan.stack_name.clone(),
                total_entries: plan.steps.len(),
                entries_restacked: reattach_count,
                entries_ok: ok_count,
                dry_run: false,
                steps: json_steps,
            },
        });
    } else {
        println!(
            "{} Restacked {} — {} {} repaired, {} already consistent.",
            style("✓").green().bold(),
            style(&plan.stack_name).cyan(),
            reattach_count,
            if reattach_count == 1 {
                "entry"
            } else {
                "entries"
            },
            ok_count,
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::StackEntry;

    fn make_entry(pos: usize, gg_id: Option<&str>, gg_parent: Option<&str>) -> StackEntry {
        StackEntry {
            oid: git2::Oid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            short_sha: format!("aaa{}", pos),
            title: format!("commit {}", pos),
            gg_id: gg_id.map(String::from),
            gg_parent: gg_parent.map(String::from),
            mr_number: None,
            mr_state: None,
            approved: false,
            ci_status: None,
            position: pos,
            in_merge_train: false,
            merge_train_position: None,
        }
    }

    fn make_stack(entries: Vec<StackEntry>) -> Stack {
        Stack {
            name: "test-stack".to_string(),
            username: "test".to_string(),
            base: "main".to_string(),
            entries,
            current_position: Some(1),
        }
    }

    #[test]
    fn test_build_plan_all_consistent() {
        let entries = vec![
            make_entry(1, Some("c-aaa1111"), None),
            make_entry(2, Some("c-bbb2222"), Some("c-aaa1111")),
            make_entry(3, Some("c-ccc3333"), Some("c-bbb2222")),
        ];
        let stack = make_stack(entries);
        let base_oid = git2::Oid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();

        let plan = build_plan(&stack, None, base_oid).unwrap();

        assert!(plan.is_consistent());
        assert_eq!(plan.steps.len(), 3);
        assert!(plan.steps.iter().all(|s| s.action == RestackAction::Ok));
    }

    #[test]
    fn test_build_plan_detects_mismatch() {
        let entries = vec![
            make_entry(1, Some("c-aaa1111"), None),
            make_entry(2, Some("c-bbb2222"), Some("c-aaa1111")),
            // Entry 3 has wrong parent — points to id-1 instead of id-2
            make_entry(3, Some("c-ccc3333"), Some("c-aaa1111")),
        ];
        let stack = make_stack(entries);
        let base_oid = git2::Oid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();

        let plan = build_plan(&stack, None, base_oid).unwrap();

        assert!(!plan.is_consistent());
        assert_eq!(plan.reattach_count(), 1);
        assert_eq!(plan.steps[0].action, RestackAction::Ok);
        assert_eq!(plan.steps[1].action, RestackAction::Ok);
        assert_eq!(plan.steps[2].action, RestackAction::Reattach);
        assert_eq!(plan.steps[2].current_parent.as_deref(), Some("c-aaa1111"));
        assert_eq!(plan.steps[2].expected_parent.as_deref(), Some("c-bbb2222"));
    }

    #[test]
    fn test_build_plan_from_position() {
        let entries = vec![
            make_entry(1, Some("c-aaa1111"), None),
            // Entry 2 has wrong parent (None instead of id-1) — but --from 3 skips it
            make_entry(2, Some("c-bbb2222"), None),
            make_entry(3, Some("c-ccc3333"), Some("c-bbb2222")),
        ];
        let stack = make_stack(entries);
        let base_oid = git2::Oid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();

        // --from 3: only entry 3 is in the plan
        let plan = build_plan(&stack, Some(3), base_oid).unwrap();

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].position, 3);
        assert_eq!(plan.steps[0].action, RestackAction::Ok); // entry 3's parent IS id-2
    }
}
