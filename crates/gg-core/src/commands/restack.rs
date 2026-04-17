//! `gg restack` - Repair stack ancestry after manual history changes

use std::io::Write;

use console::style;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::operations::{OperationKind, SnapshotScope};
use crate::output::{
    print_json, RestackResponse, RestackResultJson, RestackStepJson, OUTPUT_VERSION,
};
use crate::stack::{self, Stack};

/// Options for the restack command
#[derive(Debug, Default)]
pub struct RestackOptions {
    /// Show what would be done without making changes
    pub dry_run: bool,
    /// Repair only from this commit upward (position, SHA, or GG-ID)
    pub from: Option<String>,
    /// Output as JSON
    pub json: bool,
}

/// Action to take for a single stack entry during restack.
#[derive(Debug, Clone, PartialEq)]
pub enum RestackAction {
    /// Entry's GG-Parent already matches the expected parent — no change needed.
    Ok,
    /// Entry's GG-Parent differs from expected — needs rebasing onto correct parent.
    Reattach,
    /// Entry is below the `--from` threshold and was not checked.
    Skip,
}

/// A single step in a restack plan.
#[derive(Debug, Clone)]
pub struct RestackStep {
    /// Position in stack (1-indexed)
    pub position: usize,
    /// GG-ID of the entry
    pub gg_id: String,
    /// Commit title for display
    pub title: String,
    /// Current (possibly wrong) GG-Parent
    pub current_parent: Option<String>,
    /// Expected (correct) GG-Parent
    pub expected_parent: Option<String>,
    /// Whether this entry needs rebasing
    pub action: RestackAction,
}

/// A planned set of ancestry repairs. Public for reuse by `gg reparent` (Task 5).
#[derive(Debug)]
pub struct RestackPlan {
    pub steps: Vec<RestackStep>,
}

impl RestackPlan {
    /// Build a restack plan by comparing each entry's GG-Parent against the expected parent.
    ///
    /// If `from_position` is provided, only entries at or above that position are checked;
    /// entries below are marked `Ok` unconditionally.
    pub fn build(stack: &Stack, from_position: Option<usize>) -> Result<Self> {
        let mut steps = Vec::with_capacity(stack.entries.len());

        for entry in &stack.entries {
            let gg_id = entry.gg_id.as_deref().ok_or_else(|| {
                GgError::Other(format!(
                    "Commit #{} ({}) is missing a GG-ID. Run `gg reconcile` first.",
                    entry.position, entry.short_sha
                ))
            })?;

            let expected = stack.expected_parent_gg_id(entry.position).map(String::from);
            let current = entry.gg_parent.clone();

            let below_from = from_position.is_some_and(|from_pos| entry.position < from_pos);
            let action = if below_from {
                RestackAction::Skip
            } else if current == expected {
                RestackAction::Ok
            } else {
                RestackAction::Reattach
            };

            steps.push(RestackStep {
                position: entry.position,
                gg_id: gg_id.to_string(),
                title: entry.title.clone(),
                current_parent: current,
                expected_parent: expected,
                action,
            });
        }

        Ok(RestackPlan { steps })
    }

    /// Returns true if any step requires rebasing.
    pub fn needs_rebase(&self) -> bool {
        self.steps.iter().any(|s| s.action == RestackAction::Reattach)
    }

    /// Count of entries that need rebasing.
    pub fn reattach_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| s.action == RestackAction::Reattach)
            .count()
    }

    /// Convert plan steps to JSON output structs.
    fn to_json_steps(&self) -> Vec<RestackStepJson> {
        self.steps
            .iter()
            .map(|s| RestackStepJson {
                position: s.position,
                gg_id: s.gg_id.clone(),
                title: s.title.clone(),
                action: match s.action {
                    RestackAction::Ok => "ok".to_string(),
                    RestackAction::Reattach => "reattach".to_string(),
                    RestackAction::Skip => "skip".to_string(),
                },
                current_parent: s.current_parent.clone(),
                expected_parent: s.expected_parent.clone(),
            })
            .collect()
    }
}

/// Run the restack command.
pub fn run(options: RestackOptions) -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;

    // Acquire operation lock (recording deferred until after validation)
    let _lock = git::acquire_operation_lock(&repo, "restack")?;
    // The op-log record is written just before mutation (below) so that
    // rejected/dry-run invocations do not pollute the operation log.

    // Check for rebase-in-progress
    if git::is_rebase_in_progress(&repo) {
        return Err(GgError::Other(
            "A rebase is already in progress. Complete or abort it first.".to_string(),
        ));
    }

    // Require clean working directory
    git::require_clean_working_directory(&repo)?;

    // Load stack
    let stack = Stack::load(&repo, &config)?;

    if stack.is_empty() {
        return Err(GgError::Other("Stack is empty".to_string()));
    }

    // Resolve --from target
    let from_position = match &options.from {
        Some(target) => Some(stack::resolve_target(&stack, target)?),
        None => None,
    };

    // Build plan
    let plan = RestackPlan::build(&stack, from_position)?;

    let stack_name = stack.name.clone();
    let total = stack.entries.len();
    let reattach_count = plan.reattach_count();
    let skip_count = plan.steps.iter().filter(|s| s.action == RestackAction::Skip).count();
    let ok_count = total - reattach_count - skip_count;

    // No-op: stack is already consistent
    if !plan.needs_rebase() {
        if options.json {
            print_json(&RestackResponse {
                version: OUTPUT_VERSION,
                restack: RestackResultJson {
                    stack_name,
                    total_entries: total,
                    entries_restacked: 0,
                    entries_ok: total,
                    dry_run: options.dry_run,
                    steps: plan.to_json_steps(),
                },
            });
        } else {
            println!(
                "{} Stack is already consistent ({} commits, no ancestry drift)",
                style("✓").green().bold(),
                total
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
                    stack_name,
                    total_entries: total,
                    entries_restacked: reattach_count,
                    entries_ok: ok_count,
                    dry_run: true,
                    steps: plan.to_json_steps(),
                },
            });
        } else {
            println!(
                "{} Restack plan for stack {:?} ({} commits):",
                style("→").cyan().bold(),
                stack_name,
                total
            );
            for step in &plan.steps {
                let action_str = match step.action {
                    RestackAction::Skip => style("skip").dim().to_string(),
                    RestackAction::Ok => style("ok").green().to_string(),
                    RestackAction::Reattach => {
                        let cur = step.current_parent.as_deref().unwrap_or("(none)");
                        let exp = step.expected_parent.as_deref().unwrap_or("(none)");
                        format!(
                            "{}    {} → {}",
                            style("reattach").yellow(),
                            cur,
                            exp
                        )
                    }
                };
                println!(
                    "  {} {} {}",
                    style(format!("#{}", step.position)).dim(),
                    style(&step.title).white(),
                    action_str
                );
            }
            println!();
            println!(
                "{} commits need restacking. Run without --dry-run to execute.",
                reattach_count
            );
        }
        return Ok(());
    }

    // All validation passed — write the Pending op-log record immediately
    // before the actual rebase.
    let guard = git::begin_recorded_op(
        &repo,
        &config,
        OperationKind::Restack,
        std::env::args().skip(1).collect(),
        None,
        SnapshotScope::AllUserBranches,
    )?;

    // Execute: single git rebase -i
    // Determine base ref for the rebase
    let base_oid = if let Some(from_pos) = from_position {
        if from_pos <= 1 {
            // --from 1 is equivalent to full restack
            let base_ref = repo
                .revparse_single(&stack.base)
                .or_else(|_| repo.revparse_single(&format!("origin/{}", stack.base)))?;
            base_ref.id()
        } else {
            // Use the commit at from_pos - 1 as the base
            let base_entry = stack.get_entry_by_position(from_pos - 1).ok_or_else(|| {
                GgError::Other(format!("Position {} out of range", from_pos - 1))
            })?;
            base_entry.oid
        }
    } else {
        let base_ref = repo
            .revparse_single(&stack.base)
            .or_else(|_| repo.revparse_single(&format!("origin/{}", stack.base)))?;
        base_ref.id()
    };

    // Build rebase todo: pick all entries from the rebase start point upward
    let rebase_start = from_position.unwrap_or(1);
    let mut rebase_todo = String::new();
    for entry in &stack.entries {
        if entry.position >= rebase_start {
            rebase_todo.push_str(&format!("pick {}\n", entry.oid));
        }
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
        .env("GIT_SEQUENCE_EDITOR", script_file.to_str().unwrap())
        .args(["rebase", "-i", &base_oid.to_string()])
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

    // Normalize GG metadata after rebase. This normalizes the full stack
    // (not just from --from) because entries below --from are only rewritten
    // if their metadata is genuinely stale — correct entries are left as-is.
    let rewritten_stack = Stack::load(&repo, &config)?;
    git::normalize_stack_metadata(&repo, &rewritten_stack)?;

    // Finalize the op record with post-mutation refs. Restack is purely
    // local; no remote effects.
    guard.finalize_with_scope(
        &repo,
        &config,
        SnapshotScope::AllUserBranches,
        vec![],
        false,
    )?;

    // Output result
    if options.json {
        print_json(&RestackResponse {
            version: OUTPUT_VERSION,
            restack: RestackResultJson {
                stack_name,
                total_entries: total,
                entries_restacked: reattach_count,
                entries_ok: ok_count,
                dry_run: false,
                steps: plan.to_json_steps(),
            },
        });
    } else {
        println!(
            "{} Restacked {} commit(s) in stack {:?}",
            style("✓").green().bold(),
            reattach_count,
            stack_name
        );
        println!(
            "  {}",
            style("Hint: Run `gg sync` to push the repaired stack.").dim()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restack_options_default() {
        let opts = RestackOptions::default();
        assert!(!opts.dry_run);
        assert!(opts.from.is_none());
        assert!(!opts.json);
    }

    #[test]
    fn test_restack_action_equality() {
        assert_eq!(RestackAction::Ok, RestackAction::Ok);
        assert_eq!(RestackAction::Reattach, RestackAction::Reattach);
        assert_eq!(RestackAction::Skip, RestackAction::Skip);
        assert_ne!(RestackAction::Ok, RestackAction::Reattach);
        assert_ne!(RestackAction::Ok, RestackAction::Skip);
    }
}
