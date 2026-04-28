//! `gg unstack` - Split a stack into two independent stacks

use std::collections::{HashMap, HashSet};
use std::process::Command;

use console::style;
use git2::{BranchType, Oid};

use super::unstack_tui::{self, UnstackEntry};
use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::immutability::{self, ImmutabilityPolicy};
use crate::operations::{OperationKind, SnapshotScope};
use crate::output::{
    print_json, UnstackMovedEntryJson, UnstackResponse, UnstackResultJson, OUTPUT_VERSION,
};
use crate::stack::{self, Stack};

/// Options for the unstack command.
#[derive(Debug, Default)]
pub struct UnstackOptions {
    /// First commit to move into the new stack: position, short SHA, or GG-ID.
    pub target: Option<String>,
    /// Name for the new stack.
    pub name: Option<String>,
    /// Disable TUI; require --target.
    pub no_tui: bool,
    /// Override the immutability check for rewritten commits.
    pub force: bool,
    /// Output structured JSON.
    pub json: bool,
}

/// Run the unstack command.
pub fn run(options: UnstackOptions) -> Result<()> {
    let repo = git::open_repo()?;
    let mut config = Config::load_with_global(repo.commondir())?;

    let _lock = git::acquire_operation_lock(&repo, "unstack")?;
    git::require_clean_working_directory(&repo)?;

    if git::is_rebase_in_progress(&repo) {
        return Err(GgError::Other(
            "A rebase is already in progress. Complete or abort it first.".to_string(),
        ));
    }

    let mut source_stack = Stack::load(&repo, &config)?;
    immutability::refresh_mr_state_for_guard(&repo, &mut source_stack);

    if source_stack.len() < 2 {
        return Err(GgError::Other(
            "Need at least 2 commits to unstack.".to_string(),
        ));
    }

    let Some(target_position) = resolve_unstack_target(&source_stack, &options)? else {
        println!("{}", style("Unstack cancelled.").dim());
        return Ok(());
    };
    if target_position == 1 {
        return Err(GgError::Other(
            "Cannot unstack at position 1 because no commits would remain in the original stack."
                .to_string(),
        ));
    }

    let new_stack_name = resolve_new_stack_name(&repo, &config, &source_stack, options.name)?;
    let moved_entries = moved_entries_json(&source_stack, target_position);
    let moved_gg_ids: HashSet<String> = source_stack.entries[(target_position - 1)..]
        .iter()
        .filter_map(|entry| entry.gg_id.clone())
        .collect();

    let policy = ImmutabilityPolicy::for_stack(&repo, &source_stack)?;
    let targets: Vec<usize> = (target_position..=source_stack.len()).collect();
    let report = policy.check_positions(&source_stack, &targets);
    immutability::guard(report, options.force)?;

    let guard = git::begin_recorded_op(
        &repo,
        &config,
        OperationKind::Unstack,
        std::env::args().skip(1).collect(),
        Some(source_stack.name.clone()),
        SnapshotScope::AllUserBranches,
    )?;

    if !options.json {
        println!(
            "{} Unstacking {} commit(s) from {} into {}...",
            style("Unstack").cyan().bold(),
            moved_entries.len(),
            style(&source_stack.name).cyan(),
            style(&new_stack_name).cyan()
        );
    }

    migrate_config(&mut config, &source_stack, &new_stack_name, &moved_gg_ids);

    let old_stack_count = target_position - 1;
    let new_stack_count = source_stack.len() - old_stack_count;

    rewrite_branches(
        &repo,
        &config,
        &source_stack,
        &new_stack_name,
        target_position,
    )?;
    delete_old_entry_branches(&repo, &source_stack, &moved_gg_ids);
    config.save(repo.commondir())?;

    let new_branch = git::format_stack_branch(&source_stack.username, &new_stack_name);
    git::checkout_branch(&repo, &new_branch)?;

    guard.finalize_with_scope(
        &repo,
        &config,
        SnapshotScope::AllUserBranches,
        vec![],
        false,
    )?;

    if options.json {
        print_json(&UnstackResponse {
            version: OUTPUT_VERSION,
            unstack: UnstackResultJson {
                old_stack: source_stack.name,
                new_stack: new_stack_name,
                target_position,
                moved: moved_entries,
                old_stack_count,
                new_stack_count,
            },
        });
    } else {
        println!(
            "{} Created new stack {}",
            style("OK").green().bold(),
            style(&new_stack_name).cyan()
        );
        println!(
            "  Original stack: {} now has {} commit(s)",
            style(&source_stack.name).cyan(),
            old_stack_count
        );
        println!(
            "  New stack: {} has {} commit(s)",
            style(&new_stack_name).cyan(),
            new_stack_count
        );
        println!();
        println!(
            "{}",
            style("Run `gg sync` to push the new stack and update PR/MR targets.").dim()
        );
    }

    Ok(())
}

fn resolve_unstack_target(stack: &Stack, options: &UnstackOptions) -> Result<Option<usize>> {
    if let Some(target) = &options.target {
        return stack::resolve_target(stack, target).map(Some);
    }

    if options.no_tui {
        return Err(GgError::Other(
            "No unstack target specified. Use --target <position|SHA|GG-ID>.".to_string(),
        ));
    }

    let is_tty = atty::is(atty::Stream::Stdin) && atty::is(atty::Stream::Stdout);
    if !is_tty {
        return Err(GgError::Other(
            "No unstack target specified. Use --target <position|SHA|GG-ID>.".to_string(),
        ));
    }

    let entries: Vec<UnstackEntry> = stack
        .entries
        .iter()
        .map(|entry| UnstackEntry {
            position: entry.position,
            short_sha: entry.short_sha.clone(),
            gg_id: entry.gg_id.clone(),
            title: entry.title.clone(),
        })
        .collect();

    match unstack_tui::unstack_tui(entries)? {
        Some(position) => Ok(Some(position)),
        None => Ok(None),
    }
}

fn resolve_new_stack_name(
    repo: &git2::Repository,
    config: &Config,
    source_stack: &Stack,
    requested_name: Option<String>,
) -> Result<String> {
    if let Some(name) = requested_name {
        let sanitized = git::sanitize_stack_name(&name)?;
        if sanitized != name {
            return Err(GgError::Other(format!(
                "Invalid stack name '{}'. Did you mean '{}'?",
                name, sanitized
            )));
        }
        ensure_stack_name_available(repo, config, &source_stack.username, &sanitized)?;
        return Ok(sanitized);
    }

    for suffix in 2usize.. {
        let candidate = format!("{}-{}", source_stack.name, suffix);
        let Ok(sanitized) = git::sanitize_stack_name(&candidate) else {
            continue;
        };
        if ensure_stack_name_available(repo, config, &source_stack.username, &sanitized).is_ok() {
            return Ok(sanitized);
        }
    }

    unreachable!("unbounded suffix search must return")
}

fn ensure_stack_name_available(
    repo: &git2::Repository,
    config: &Config,
    username: &str,
    stack_name: &str,
) -> Result<()> {
    if config.get_stack(stack_name).is_some() {
        return Err(GgError::Other(format!(
            "Stack '{}' already exists in gg config.",
            stack_name
        )));
    }

    let branch_name = git::format_stack_branch(username, stack_name);
    if repo.find_branch(&branch_name, BranchType::Local).is_ok() {
        return Err(GgError::Other(format!(
            "Stack branch '{}' already exists.",
            branch_name
        )));
    }

    Ok(())
}

fn moved_entries_json(stack: &Stack, target_position: usize) -> Vec<UnstackMovedEntryJson> {
    stack.entries[(target_position - 1)..]
        .iter()
        .map(|entry| UnstackMovedEntryJson {
            old_position: entry.position,
            sha: entry.short_sha.clone(),
            gg_id: entry.gg_id.clone(),
            title: entry.title.clone(),
        })
        .collect()
}

fn migrate_config(
    config: &mut Config,
    source_stack: &Stack,
    new_stack_name: &str,
    moved_gg_ids: &HashSet<String>,
) {
    let old_stack_name = source_stack.name.clone();
    let mut moved_mrs = HashMap::new();
    let old_base = config
        .get_stack(&old_stack_name)
        .and_then(|stack| stack.base.clone());

    if let Some(old_cfg) = config.stacks.get_mut(&old_stack_name) {
        old_cfg.mrs.retain(|gg_id, mr| {
            if moved_gg_ids.contains(gg_id) {
                moved_mrs.insert(gg_id.clone(), *mr);
                false
            } else {
                true
            }
        });
    }

    let new_cfg = config.stacks.entry(new_stack_name.to_string()).or_default();
    new_cfg.base = old_base;
    new_cfg.mrs.extend(moved_mrs);
    new_cfg.worktree_path = None;
}

fn rewrite_branches(
    repo: &git2::Repository,
    config: &Config,
    source_stack: &Stack,
    new_stack_name: &str,
    target_position: usize,
) -> Result<()> {
    let source_branch = source_stack.branch_name();
    let new_branch = git::format_stack_branch(&source_stack.username, new_stack_name);
    let target_entry = source_stack
        .get_entry_by_position(target_position)
        .ok_or_else(|| GgError::Other(format!("Position {} out of range", target_position)))?;
    let previous_entry = source_stack
        .get_entry_by_position(target_position - 1)
        .ok_or_else(|| GgError::Other(format!("Position {} out of range", target_position - 1)))?;
    let old_tip = source_stack
        .last()
        .ok_or_else(|| GgError::Other("Stack is empty".to_string()))?;
    let old_tip_commit = repo.find_commit(old_tip.oid)?;

    repo.branch(&new_branch, &old_tip_commit, false)?;
    git::checkout_branch(repo, &new_branch)?;

    run_rebase_onto_base(repo, source_stack, previous_entry.oid, &new_branch)?;

    let new_stack = Stack::load(repo, config)?;
    git::normalize_stack_metadata(repo, &new_stack)?;

    set_branch_target(
        repo,
        &source_branch,
        previous_entry.oid,
        "gg unstack: shorten original stack",
    )?;
    git::checkout_branch(repo, &source_branch)?;

    let old_stack = Stack::load(repo, config)?;
    git::normalize_stack_metadata(repo, &old_stack)?;

    let new_branch_tip = repo.revparse_single(&new_branch)?.id();
    if new_branch_tip == target_entry.oid {
        return Err(GgError::Other(
            "Unstack failed to create an independent stack.".to_string(),
        ));
    }

    Ok(())
}

fn run_rebase_onto_base(
    repo: &git2::Repository,
    stack: &Stack,
    previous_oid: Oid,
    new_branch: &str,
) -> Result<()> {
    let base = repo
        .revparse_single(&stack.base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", stack.base)))
        .map_err(|_| GgError::NoBaseBranch)?;
    let base_oid = base.id().to_string();
    let previous_oid = previous_oid.to_string();

    let output = Command::new("git")
        .args(["rebase", "--onto", &base_oid, &previous_oid, new_branch])
        .output()?;

    if output.status.success() {
        git::ensure_branch_attached(repo, new_branch)?;
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("CONFLICT") || stderr.contains("conflict") {
        return Err(GgError::RebaseConflict);
    }

    Err(GgError::Other(format!("Rebase failed: {}", stderr.trim())))
}

fn set_branch_target(
    repo: &git2::Repository,
    branch_name: &str,
    oid: Oid,
    log_message: &str,
) -> Result<()> {
    let mut reference = repo.find_reference(&format!("refs/heads/{}", branch_name))?;
    reference.set_target(oid, log_message)?;
    Ok(())
}

fn delete_old_entry_branches(
    repo: &git2::Repository,
    source_stack: &Stack,
    moved_gg_ids: &HashSet<String>,
) {
    for entry in &source_stack.entries {
        let Some(gg_id) = &entry.gg_id else {
            continue;
        };
        if !moved_gg_ids.contains(gg_id) {
            continue;
        }
        if let Some(branch_name) = source_stack.entry_branch_name(entry) {
            let _ = repo
                .find_branch(&branch_name, BranchType::Local)
                .and_then(|mut branch| branch.delete());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unstack_options_default() {
        let opts = UnstackOptions::default();
        assert!(opts.target.is_none());
        assert!(opts.name.is_none());
        assert!(!opts.no_tui);
        assert!(!opts.force);
        assert!(!opts.json);
    }
}
