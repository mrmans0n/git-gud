//! `gg clean` - Clean up merged stacks

use console::style;
use dialoguer::Confirm;
use git2::{BranchType, Repository};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::config::{Config, PendingRemoteBranchDeletion, RemoteBranchDeletionState};
use crate::error::{GgError, Result};
use crate::git;
use crate::operations::{OperationKind, RemoteEffect, SnapshotScope};
use crate::output::{print_json, CleanResponse, CleanResultJson, OUTPUT_VERSION};
use crate::provider::{PrState, Provider};
use crate::stack;

use super::sync::RemoteDeletionCheckpoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteDeletionSegmentState {
    Idle,
    AttemptStarted,
    EffectRecorded,
}

pub(crate) struct RemoteDeletionSegmentOwner<'a> {
    repo: &'a Repository,
    kind: OperationKind,
    label: &'static str,
    args: &'a [String],
    stack_name: Option<&'a str>,
    guard: &'a mut Option<crate::operations::OperationGuard>,
    remote_effects: &'a mut Vec<RemoteEffect>,
    touched_remote: &'a mut bool,
    state: RemoteDeletionSegmentState,
}

impl<'a> RemoteDeletionSegmentOwner<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        repo: &'a Repository,
        kind: OperationKind,
        label: &'static str,
        args: &'a [String],
        stack_name: Option<&'a str>,
        guard: &'a mut Option<crate::operations::OperationGuard>,
        remote_effects: &'a mut Vec<RemoteEffect>,
        touched_remote: &'a mut bool,
    ) -> Self {
        Self {
            repo,
            kind,
            label,
            args,
            stack_name,
            guard,
            remote_effects,
            touched_remote,
            state: RemoteDeletionSegmentState::Idle,
        }
    }

    fn durability_error(&self, action: &str, error: impl std::fmt::Display) -> GgError {
        GgError::DurabilityCheckpoint(format!("failed to {action}: {error}"))
    }

    fn begin_segment(&mut self, config: &Config, action: &str) -> Result<()> {
        *self.guard = Some(
            git::begin_recorded_op(
                self.repo,
                config,
                self.kind,
                self.args.to_vec(),
                self.stack_name.map(str::to_string),
                SnapshotScope::AllUserBranches,
            )
            .map_err(|error| self.durability_error(action, error))?,
        );
        Ok(())
    }

    fn finalize_current(&mut self, config: &Config, action: &str) -> Result<()> {
        let Some(segment) = self.guard.take() else {
            return Ok(());
        };
        segment
            .finalize_with_scope(
                self.repo,
                config,
                SnapshotScope::AllUserBranches,
                std::mem::take(self.remote_effects),
                std::mem::take(self.touched_remote),
            )
            .map_err(|error| self.durability_error(action, error))
    }

    pub(crate) fn checkpoint(
        &mut self,
        config: &mut Config,
        checkpoint: RemoteDeletionCheckpoint,
    ) -> Result<()> {
        match (self.state, checkpoint) {
            (
                RemoteDeletionSegmentState::Idle,
                RemoteDeletionCheckpoint::BeforeAttempt {
                    remote,
                    branch,
                    prior_oid,
                },
            ) => {
                self.finalize_current(
                    config,
                    &format!("finalize {} segment before deletion", self.label),
                )?;
                self.begin_segment(
                    config,
                    &format!("open {} remote deletion segment", self.label),
                )?;
                self.guard
                    .as_mut()
                    .expect("remote deletion segment guard")
                    .mark_touched_remote_before_attempt()
                    .map_err(|error| {
                        self.durability_error(
                            &format!("persist {} remote deletion uncertainty", self.label),
                            error,
                        )
                    })?;
                *self.touched_remote = true;
                if !config
                    .pending_remote_branch_deletions
                    .iter()
                    .any(|intent| intent.remote == remote && intent.branch == branch)
                {
                    let source_operation_id = self
                        .guard
                        .as_ref()
                        .expect("remote deletion segment guard")
                        .id()
                        .to_string();
                    config
                        .pending_remote_branch_deletions
                        .push(PendingRemoteBranchDeletion {
                            remote,
                            branch,
                            expected_oid: prior_oid,
                            source_operation_id,
                            state: RemoteBranchDeletionState::Deleting,
                        });
                    config.save(self.repo.commondir()).map_err(|error| {
                        self.durability_error(
                            &format!("persist {} remote deletion identity", self.label),
                            error,
                        )
                    })?;
                }
                self.state = RemoteDeletionSegmentState::AttemptStarted;
                Ok(())
            }
            (
                RemoteDeletionSegmentState::AttemptStarted,
                RemoteDeletionCheckpoint::Effect(effect),
            ) => {
                self.guard
                    .as_mut()
                    .expect("remote deletion segment guard")
                    .record_remote_effect_durable(effect.clone())
                    .map_err(|error| {
                        self.durability_error(
                            &format!(
                                "durably record exact remote deletion effect for {}",
                                self.label
                            ),
                            error,
                        )
                    })?;
                self.remote_effects.push(effect);
                self.state = RemoteDeletionSegmentState::EffectRecorded;
                Ok(())
            }
            (RemoteDeletionSegmentState::EffectRecorded, RemoteDeletionCheckpoint::Complete) => {
                self.finalize_current(
                    config,
                    &format!("finalize {} deletion segment", self.label),
                )?;
                self.begin_segment(
                    config,
                    &format!("open post-deletion {} local cleanup segment", self.label),
                )?;
                self.state = RemoteDeletionSegmentState::Idle;
                Ok(())
            }
            (
                RemoteDeletionSegmentState::AttemptStarted,
                RemoteDeletionCheckpoint::DeferredBeforeMutation,
            ) => {
                self.guard
                    .as_mut()
                    .expect("remote deletion segment guard")
                    .clear_touched_remote_before_attempt()
                    .map_err(|error| {
                        self.durability_error(
                            &format!("clear deferred {} deletion uncertainty", self.label),
                            error,
                        )
                    })?;
                *self.touched_remote = false;
                self.finalize_current(
                    config,
                    &format!("finalize deferred {} deletion segment", self.label),
                )?;
                self.begin_segment(
                    config,
                    &format!("open post-deferral {} local cleanup segment", self.label),
                )?;
                self.state = RemoteDeletionSegmentState::Idle;
                Ok(())
            }
            (RemoteDeletionSegmentState::Idle, RemoteDeletionCheckpoint::LocalMutationComplete) => {
                self.finalize_current(
                    config,
                    &format!(
                        "finalize {} local mutation before success output",
                        self.label
                    ),
                )?;
                self.begin_segment(
                    config,
                    &format!("open post-output {} local cleanup segment", self.label),
                )?;
                Ok(())
            }
            (state, checkpoint) => Err(GgError::DurabilityCheckpoint(format!(
                "invalid {} remote deletion checkpoint transition: {state:?} -> {checkpoint:?}",
                self.label
            ))),
        }
    }

    pub(crate) fn finish(mut self, config: &Config) -> Result<()> {
        if self.state != RemoteDeletionSegmentState::Idle {
            return Err(GgError::DurabilityCheckpoint(format!(
                "cannot finish {} cleanup with an incomplete remote deletion segment",
                self.label
            )));
        }
        self.finalize_current(config, &format!("finalize {} cleanup", self.label))
    }

    fn ensure_local_segment(&mut self, config: &Config) -> Result<()> {
        if self.state != RemoteDeletionSegmentState::Idle {
            return Err(GgError::DurabilityCheckpoint(format!(
                "cannot open {} local cleanup segment during a remote deletion",
                self.label
            )));
        }
        if self.guard.is_none() {
            self.begin_segment(
                config,
                &format!("open {} local cleanup segment", self.label),
            )?;
        }
        Ok(())
    }

    fn finalize_before_success_output(&mut self, config: &Config) -> Result<()> {
        if self.state != RemoteDeletionSegmentState::Idle {
            return Err(GgError::DurabilityCheckpoint(format!(
                "cannot report {} success with an incomplete remote deletion segment",
                self.label
            )));
        }
        self.finalize_current(
            config,
            &format!("finalize {} cleanup before success output", self.label),
        )
    }
}

/// Run the clean command for a specific stack (used by auto-clean)
#[allow(dead_code)]
pub fn run_for_stack(stack_name: &str, force: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let _lock = git::acquire_operation_lock(&repo, "clean")?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    run_for_stack_with_owned_journal(&repo, stack_name, force, args)
}

fn run_for_stack_with_owned_journal(
    repo: &Repository,
    stack_name: &str,
    force: bool,
    args: Vec<String>,
) -> Result<()> {
    let config = Config::load_with_global(repo.commondir())?;
    let mut guard = Some(git::begin_recorded_op(
        repo,
        &config,
        OperationKind::Clean,
        args.clone(),
        Some(stack_name.to_string()),
        SnapshotScope::AllUserBranches,
    )?);
    let mut remote_effects = Vec::new();
    let mut touched_remote = false;
    let mut segments = RemoteDeletionSegmentOwner::new(
        repo,
        OperationKind::Clean,
        "clean",
        &args,
        Some(stack_name),
        &mut guard,
        &mut remote_effects,
        &mut touched_remote,
    );

    let cleanup_result = run_for_stack_with_repo_options(
        repo,
        stack_name,
        force,
        false,
        false,
        &mut |checkpoint, checkpoint_config| segments.checkpoint(checkpoint_config, checkpoint),
    );

    match cleanup_result {
        Ok(_) => {
            let final_config = Config::load_with_global(repo.commondir())?;
            segments.finish(&final_config)?;
            Ok(())
        }
        Err(error @ GgError::DurabilityCheckpoint(_))
        | Err(error @ GgError::RemoteMutationUncertain(_)) => Err(error),
        Err(error) => {
            let finish_result = Config::load_with_global(repo.commondir())
                .and_then(|final_config| segments.finish(&final_config));
            match finish_result {
                Ok(()) => Err(error),
                Err(finalize_error) => Err(GgError::Other(format!(
                    "{error}; additionally failed to finalize clean operation: {finalize_error}"
                ))),
            }
        }
    }
}

/// Run clean for a stack with an already-open repository.
pub fn run_for_stack_with_repo(repo: &Repository, stack_name: &str, force: bool) -> Result<()> {
    let _lock = git::acquire_operation_lock(repo, "clean")?;
    run_for_stack_with_owned_journal(
        repo,
        stack_name,
        force,
        vec!["clean".to_string(), stack_name.to_string()],
    )
}

/// Run clean after `gg land` has already verified that every PR/MR in the stack
/// was merged via the provider.
///
/// `gg land` removes merged PR/MR mappings from config before auto-clean runs,
/// so a normal clean cannot always re-check provider state. This entry point
/// carries the verification that land just obtained and allows remote entry
/// branch deletion while keeping normal `gg clean` conservative.
pub(crate) fn run_for_stack_with_repo_after_verified_land(
    repo: &Repository,
    stack_name: &str,
    force: bool,
    silent: bool,
    record_remote_effect: &mut dyn FnMut(RemoteDeletionCheckpoint, &mut Config) -> Result<()>,
) -> Result<bool> {
    // Land holds the shared operation lock whenever it calls this helper.
    run_for_stack_with_repo_options(repo, stack_name, force, true, silent, record_remote_effect)
}

fn delete_stack_branch(
    repo: &Repository,
    config: &Config,
    stack_name: &str,
    branch_name: &str,
    silent: bool,
) -> Result<bool> {
    let Ok(mut branch) = repo.find_branch(branch_name, BranchType::Local) else {
        return Ok(true);
    };

    let current = git::current_branch_name(repo);
    if current.as_deref() == Some(branch_name) {
        let base = config
            .get_base_for_stack(stack_name)
            .map(|s| s.to_string())
            .or_else(|| git::find_base_branch(repo).ok())
            .unwrap_or_else(|| "main".to_string());

        if let Err(e) = git::checkout_branch(repo, &base) {
            let msg = e.to_string();
            if msg.contains("current HEAD of a linked") {
                if !silent {
                    println!(
                        "{} '{}' is checked out in another worktree; detaching HEAD before branch deletion.",
                        style("Note:").cyan(),
                        base
                    );
                }
                if let Ok(head) = repo.head() {
                    if let Some(oid) = head.target() {
                        repo.set_head_detached(oid)?;
                    }
                }
            } else {
                return Err(e);
            }
        }
    }

    if let Some(wt_name) = git::is_branch_checked_out_in_worktree(repo, branch_name) {
        if !git::try_prune_worktree(repo, &wt_name) {
            if !silent {
                println!(
                    "{} Branch '{}' is checked out in worktree '{}'. Removing worktree.",
                    style("Note:").cyan(),
                    branch_name,
                    wt_name
                );
            }
            if let Err(e) = git::remove_worktree(&wt_name) {
                if !silent {
                    println!(
                        "{} Could not remove worktree '{}': {}",
                        style("Warning:").yellow(),
                        wt_name,
                        e
                    );
                }
                return Ok(false);
            }
        }
    }

    if let Err(e) = branch.delete() {
        if !silent {
            println!(
                "{} Could not delete local branch '{}': {}",
                style("Warning:").yellow(),
                branch_name,
                e
            );
            println!(
                "  You may need to manually remove the worktree first: git worktree remove <path>"
            );
        }
        return Ok(false);
    }

    Ok(true)
}

fn reconcile_pending_deletions(
    repo: &Repository,
    provider: Option<&Provider>,
    config: &mut Config,
    stack_name: &str,
    username: &str,
    record_remote_effect: &mut dyn FnMut(RemoteDeletionCheckpoint, &mut Config) -> Result<()>,
) -> Result<()> {
    let branch_name = git::format_stack_branch(username, stack_name);
    let stack_tip = repo.refname_to_id(&format!("refs/heads/{branch_name}"));
    let current_branches = if let Ok(tip) = stack_tip {
        let base = config
            .get_base_for_stack(stack_name)
            .map(str::to_owned)
            .or_else(|| git::find_base_branch(repo).ok())
            .ok_or(GgError::NoBaseBranch)?;
        let mut branches = HashSet::new();
        for oid in git::get_stack_commit_oids_from_tip(repo, &base, Some(&branch_name), tip)? {
            if let Some(entry_id) = git::get_gg_id(&repo.find_commit(oid)?) {
                branches.insert(git::format_entry_branch(username, stack_name, &entry_id));
            }
        }
        branches
    } else {
        HashSet::new()
    };

    let deferred = super::sync::reconcile_pending_remote_entry_deletions(
        repo,
        provider,
        username,
        stack_name,
        &current_branches,
        config,
        |checkpoint, config| record_remote_effect(checkpoint, config),
    )?;
    // Keep the stack (and its intents) recoverable instead of stranding them.
    if deferred.is_empty() {
        Ok(())
    } else {
        Err(GgError::Other(deferred.join(" ")))
    }
}

fn run_for_stack_with_repo_options(
    repo: &Repository,
    stack_name: &str,
    force: bool,
    merge_verified_by_land: bool,
    silent: bool,
    record_remote_effect: &mut dyn FnMut(RemoteDeletionCheckpoint, &mut Config) -> Result<()>,
) -> Result<bool> {
    let git_dir = repo.commondir();
    let mut config = Config::load_with_global(git_dir)?;

    // Detect provider (best-effort)
    let provider = Provider::detect(repo).ok();

    // Get username
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| provider.as_ref().and_then(|p| p.whoami().ok()))
        .unwrap_or_else(|| "unknown".to_string());

    git::validate_branch_username(&username)?;

    // Check if the stack exists
    let branch_name = git::format_stack_branch(&username, stack_name);

    // Check if stack is fully merged
    let merge_status = check_stack_merged(
        repo,
        &config,
        stack_name,
        &username,
        provider.as_ref(),
        silent,
    )?;

    if !merge_status.merged && !force {
        return Err(GgError::Other(format!(
            "Stack '{}' has unmerged commits",
            stack_name
        )));
    }

    reconcile_pending_deletions(
        repo,
        provider.as_ref(),
        &mut config,
        stack_name,
        &username,
        record_remote_effect,
    )?;

    let worktree_cleanup = maybe_remove_configured_worktree(repo, &mut config, stack_name, silent)?;
    if !worktree_cleanup.proceed {
        return Ok(false);
    }
    config.save(git_dir).map_err(|error| {
        GgError::DurabilityCheckpoint(format!(
            "failed to persist configured worktree cleanup for stack '{stack_name}': {error}"
        ))
    })?;
    worktree_cleanup.clear_recovery_marker().map_err(|error| {
        GgError::DurabilityCheckpoint(format!(
            "failed to finalize configured worktree removal intent for stack '{stack_name}': {error}"
        ))
    })?;
    if worktree_cleanup.removed_path.is_some() {
        record_remote_effect(RemoteDeletionCheckpoint::LocalMutationComplete, &mut config)?;
        worktree_cleanup.emit_success(silent);
    }

    let allow_remote_delete = should_delete_remote_branches(merge_status, merge_verified_by_land);
    if !allow_remote_delete && !silent {
        println!(
            "{} Skipping remote branch deletion for '{}' because merge verification is unavailable.",
            style("Warning:").yellow(),
            stack_name
        );
    }

    // Delete entry branches (local and remote when verified)
    delete_entry_branches(
        repo,
        &mut config,
        stack_name,
        &username,
        /*delete_remote=*/ allow_remote_delete,
        /*silent=*/ silent,
        record_remote_effect,
    )?;

    if !delete_stack_branch(repo, &config, stack_name, &branch_name, silent)? {
        return Ok(false);
    }

    // Remove from config
    config.remove_stack(stack_name);

    // Save updated config
    config.save(git_dir).map_err(|error| {
        GgError::DurabilityCheckpoint(format!(
            "failed to persist cleanup of stack '{stack_name}': {error}"
        ))
    })?;

    Ok(true)
}

/// Run the clean command
pub fn run(clean_all: bool, json: bool) -> Result<()> {
    let repo = git::open_repo()?;

    if json && !clean_all {
        crate::output::print_json_error(
            "--json requires --all (cannot show interactive prompts in JSON mode)",
        );
        std::process::exit(1);
    }

    let git_dir = repo.commondir();
    let _lock = git::acquire_operation_lock(&repo, "clean")?;
    let mut config = Config::load_with_global(git_dir)?;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut guard = Some(git::begin_recorded_op(
        &repo,
        &config,
        OperationKind::Clean,
        args.clone(),
        None,
        SnapshotScope::AllUserBranches,
    )?);

    // Detect provider (best-effort).
    // Some repos (e.g. local remotes in tests) won't match GitHub/GitLab.
    let provider = Provider::detect(&repo).ok();

    // Get username
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| provider.as_ref().and_then(|p| p.whoami().ok()))
        .unwrap_or_else(|| "unknown".to_string());

    git::validate_branch_username(&username)?;

    // Get all stacks
    let stacks = stack::list_all_stacks(&repo, &config, &username)?;

    let mut cleaned: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut remote_effects = Vec::new();
    let mut touched_remote = false;
    let mut segments = RemoteDeletionSegmentOwner::new(
        &repo,
        OperationKind::Clean,
        "clean",
        &args,
        None,
        &mut guard,
        &mut remote_effects,
        &mut touched_remote,
    );
    let mut cleanup_error: Option<String> = None;

    if stacks.is_empty() {
        segments.finish(&config)?;
        if json {
            print_json(&CleanResponse {
                version: OUTPUT_VERSION,
                clean: CleanResultJson { cleaned, skipped },
            });
        } else {
            println!("{}", style("No stacks to clean.").dim());
        }
        return Ok(());
    }

    for stack_name in &stacks {
        segments.ensure_local_segment(&config)?;
        // Try to determine if stack is fully merged
        let branch_name = git::format_stack_branch(&username, stack_name);

        // Check if the branch exists
        if repo.find_branch(&branch_name, BranchType::Local).is_err() {
            // Branch doesn't exist: clean LOCAL orphan entry branches and config.
            // Be conservative: do NOT delete remote branches here because we can't
            // reliably verify merge status without the main stack branch.
            if let Err(error) = reconcile_pending_deletions(
                &repo,
                provider.as_ref(),
                &mut config,
                stack_name,
                &username,
                &mut |checkpoint, checkpoint_config| {
                    segments.checkpoint(checkpoint_config, checkpoint)
                },
            ) {
                if matches!(
                    error,
                    GgError::DurabilityCheckpoint(_) | GgError::RemoteMutationUncertain(_)
                ) {
                    return Err(error);
                }
                cleanup_error.get_or_insert_with(|| error.to_string());
                skipped.push(format!("{stack_name} ({error})"));
                continue;
            }
            if let Err(error) = delete_entry_branches(
                &repo,
                &mut config,
                stack_name,
                &username,
                /*delete_remote=*/ false,
                /*silent=*/ json,
                &mut |_, _| Ok(()),
            ) {
                if matches!(
                    error,
                    GgError::DurabilityCheckpoint(_) | GgError::RemoteMutationUncertain(_)
                ) {
                    return Err(error);
                }
                cleanup_error.get_or_insert_with(|| error.to_string());
                skipped.push(format!("{stack_name} ({error})"));
                continue;
            }
            config.remove_stack(stack_name);
            cleaned.push(stack_name.clone());
            continue;
        }

        // Load the stack to check MR status
        let merge_status = check_stack_merged(
            &repo,
            &config,
            stack_name,
            &username,
            provider.as_ref(),
            json,
        )?;

        if merge_status.merged {
            if !clean_all && !json {
                let confirm = Confirm::new()
                    .with_prompt(format!("Delete merged stack '{}'? ", stack_name))
                    .default(true)
                    .interact()
                    .unwrap_or(false);

                if !confirm {
                    skipped.push(stack_name.clone());
                    continue;
                }
            }

            if let Err(error) = reconcile_pending_deletions(
                &repo,
                provider.as_ref(),
                &mut config,
                stack_name,
                &username,
                &mut |checkpoint, checkpoint_config| {
                    segments.checkpoint(checkpoint_config, checkpoint)
                },
            ) {
                if matches!(
                    error,
                    GgError::DurabilityCheckpoint(_) | GgError::RemoteMutationUncertain(_)
                ) {
                    return Err(error);
                }
                cleanup_error.get_or_insert_with(|| error.to_string());
                skipped.push(format!("{stack_name} ({error})"));
                continue;
            }

            let worktree_cleanup =
                maybe_remove_configured_worktree(&repo, &mut config, stack_name, json)?;
            config.save(git_dir).map_err(|error| {
                GgError::DurabilityCheckpoint(format!(
                    "failed to persist configured worktree cleanup for stack '{stack_name}': {error}"
                ))
            })?;
            worktree_cleanup.clear_recovery_marker().map_err(|error| {
                GgError::DurabilityCheckpoint(format!(
                    "failed to finalize configured worktree removal intent for stack '{stack_name}': {error}"
                ))
            })?;
            if worktree_cleanup.removed_path.is_some() {
                segments
                    .checkpoint(&mut config, RemoteDeletionCheckpoint::LocalMutationComplete)?;
                worktree_cleanup.emit_success(json);
            }
            let removed_or_not_configured = worktree_cleanup.proceed;
            if json && !removed_or_not_configured {
                skipped.push(format!(
                    "{} (worktree not removed: confirmation defaults to false in --json mode)",
                    stack_name
                ));
                continue;
            }

            let allow_remote_delete = merge_status.verified;
            if !allow_remote_delete && !json {
                println!(
                    "{} Skipping remote branch deletion for '{}' because merge verification is unavailable.",
                    style("Warning:").yellow(),
                    stack_name
                );
            }

            // Delete entry branches (local and remote when verified)
            if let Err(error) = delete_entry_branches(
                &repo,
                &mut config,
                stack_name,
                &username,
                /*delete_remote=*/ allow_remote_delete,
                /*silent=*/ json,
                &mut |checkpoint, checkpoint_config| {
                    segments.checkpoint(checkpoint_config, checkpoint)
                },
            ) {
                if matches!(
                    error,
                    GgError::DurabilityCheckpoint(_) | GgError::RemoteMutationUncertain(_)
                ) {
                    return Err(error);
                }
                if !json {
                    println!(
                        "{} Could not delete entry branches for '{}': {}",
                        style("Warning:").yellow(),
                        stack_name,
                        error
                    );
                }
                cleanup_error.get_or_insert_with(|| error.to_string());
                skipped.push(format!("{stack_name} ({error})"));
                continue;
            }

            match delete_stack_branch(&repo, &config, stack_name, &branch_name, json) {
                Ok(true) => {}
                Ok(false) => {
                    skipped.push(format!("{stack_name} (branch cleanup could not complete)"));
                    continue;
                }
                Err(error) => {
                    if !json {
                        println!(
                            "{} Could not delete stack branch for '{}': {}",
                            style("Warning:").yellow(),
                            stack_name,
                            error
                        );
                    }
                    cleanup_error.get_or_insert_with(|| error.to_string());
                    skipped.push(format!("{stack_name} ({error})"));
                    continue;
                }
            }

            // Remove from config
            config.remove_stack(stack_name);
            config.save(git_dir).map_err(|error| {
                GgError::DurabilityCheckpoint(format!(
                    "failed to persist cleanup of stack '{stack_name}': {error}"
                ))
            })?;
            segments.finalize_before_success_output(&config)?;
            if !json {
                println!(
                    "{} Deleted stack '{}'",
                    style("OK").green().bold(),
                    stack_name
                );
            }
            cleaned.push(stack_name.clone());
        } else {
            if !json {
                println!(
                    "{} Stack '{}' has unmerged commits, skipping",
                    style("○").yellow(),
                    stack_name
                );
            }
            skipped.push(stack_name.clone());
        }
    }

    // Save updated config
    config.save(git_dir).map_err(|error| {
        GgError::DurabilityCheckpoint(format!(
            "failed to persist final clean configuration: {error}"
        ))
    })?;

    segments.finish(&config)?;

    if json {
        print_json(&CleanResponse {
            version: OUTPUT_VERSION,
            clean: CleanResultJson { cleaned, skipped },
        });
    } else if !cleaned.is_empty() {
        println!();
        println!(
            "{} Cleaned {} stack(s)",
            style("OK").green().bold(),
            cleaned.len()
        );
    } else {
        println!("{}", style("No stacks to clean.").dim());
    }

    if json {
        Ok(())
    } else if let Some(error) = cleanup_error {
        Err(GgError::Other(error))
    } else {
        Ok(())
    }
}

struct ConfiguredWorktreeCleanup {
    proceed: bool,
    removed_path: Option<String>,
    recovery_marker: Option<PathBuf>,
}

impl ConfiguredWorktreeCleanup {
    fn proceed() -> Self {
        Self {
            proceed: true,
            removed_path: None,
            recovery_marker: None,
        }
    }

    fn stop() -> Self {
        Self {
            proceed: false,
            removed_path: None,
            recovery_marker: None,
        }
    }

    fn removed(path: String, recovery_marker: PathBuf) -> Self {
        Self {
            proceed: true,
            removed_path: Some(path),
            recovery_marker: Some(recovery_marker),
        }
    }

    fn clear_recovery_marker(&self) -> Result<()> {
        let Some(marker) = &self.recovery_marker else {
            return Ok(());
        };
        match fs::remove_file(marker) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        #[cfg(unix)]
        if let Some(parent) = marker.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    }

    fn emit_success(&self, silent: bool) {
        emit_removed_worktree_success(self.removed_path.as_deref(), silent);
    }
}

pub(crate) fn emit_removed_worktree_success(path: Option<&str>, silent: bool) {
    if let Some(path) = path.filter(|_| !silent) {
        println!(
            "{} Removed worktree '{}'.",
            style("OK").green().bold(),
            path
        );
    }
}

#[cfg(debug_assertions)]
fn test_worktree_removal_confirmed() -> bool {
    std::env::var_os("GG_TEST_CONFIRM_WORKTREE_REMOVAL").is_some()
}

#[cfg(not(debug_assertions))]
fn test_worktree_removal_confirmed() -> bool {
    false
}

fn normalize_worktree_path(repo: &Repository, configured_path: &str) -> PathBuf {
    let path = Path::new(configured_path);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo.workdir()
            .unwrap_or_else(|| repo.commondir())
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn worktree_removal_marker_path(repo: &Repository, stack_name: &str) -> PathBuf {
    let encoded_name = stack_name
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Config::config_path(repo.commondir())
        .parent()
        .expect("config path has a parent")
        .join(format!("worktree-removal-{encoded_name}.json"))
}

fn persist_worktree_removal_marker(marker: &Path, worktree_path: &str) -> Result<()> {
    let temp = marker.with_extension("tmp");
    let contents = serde_json::to_vec(worktree_path)?;
    {
        let mut file = File::create(&temp)?;
        file.write_all(&contents)?;
        file.sync_all()?;
    }
    fs::rename(&temp, marker)?;
    #[cfg(unix)]
    if let Some(parent) = marker.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn worktree_removal_marker_matches(marker: &Path, worktree_path: &str) -> Result<bool> {
    match fs::read(marker) {
        Ok(contents) => Ok(serde_json::from_slice::<String>(&contents)? == worktree_path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn maybe_remove_configured_worktree(
    repo: &Repository,
    config: &mut Config,
    stack_name: &str,
    silent: bool,
) -> Result<ConfiguredWorktreeCleanup> {
    let Some(stack_cfg) = config.get_stack(stack_name) else {
        return Ok(ConfiguredWorktreeCleanup::proceed());
    };
    let Some(worktree_path) = stack_cfg.worktree_path.clone() else {
        return Ok(ConfiguredWorktreeCleanup::proceed());
    };

    let repo_root = repo
        .workdir()
        .ok_or_else(|| GgError::Other("Repository has no working directory".to_string()))?;

    let recovery_marker = worktree_removal_marker_path(repo, stack_name);
    let normalized_worktree_path = normalize_worktree_path(repo, &worktree_path);
    if !normalized_worktree_path.exists() {
        let output = std::process::Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(repo_root)
            .output()?;
        if !output.status.success() {
            return Err(GgError::Other(format!(
                "Failed to inspect worktree '{}': {}",
                worktree_path,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let registered = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .any(|path| normalize_worktree_path(repo, path) == normalized_worktree_path);
        if !registered && worktree_removal_marker_matches(&recovery_marker, &worktree_path)? {
            if let Some(stack_cfg_mut) = config.stacks.get_mut(stack_name) {
                stack_cfg_mut.worktree_path = None;
            }
            return Ok(ConfiguredWorktreeCleanup::removed(
                worktree_path,
                recovery_marker,
            ));
        }
    }

    let confirm = if test_worktree_removal_confirmed() {
        true
    } else if silent {
        false
    } else {
        let prompt = format!(
            "Stack '{}' has an associated worktree at '{}'. Remove it?",
            stack_name, worktree_path
        );
        Confirm::new()
            .with_prompt(prompt)
            .default(false)
            .interact()
            .unwrap_or(false)
    };

    if !confirm {
        if !silent {
            println!(
                "{} Keeping worktree '{}'.",
                style("Note:").cyan(),
                worktree_path
            );
        }
        return Ok(ConfiguredWorktreeCleanup::stop());
    }

    persist_worktree_removal_marker(&recovery_marker, &worktree_path).map_err(|error| {
        GgError::DurabilityCheckpoint(format!(
            "failed to persist configured worktree removal intent for stack '{stack_name}': {error}"
        ))
    })?;

    let output = std::process::Command::new("git")
        .arg("worktree")
        .arg("remove")
        .arg(&worktree_path)
        .arg("--force")
        .current_dir(repo_root)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to remove worktree '{}': {}",
            worktree_path,
            stderr.trim()
        )));
    }

    if let Some(stack_cfg_mut) = config.stacks.get_mut(stack_name) {
        stack_cfg_mut.worktree_path = None;
    }

    let removed = !Path::new(&worktree_path).exists();
    Ok(if removed {
        ConfiguredWorktreeCleanup::removed(worktree_path, recovery_marker)
    } else {
        ConfiguredWorktreeCleanup::proceed()
    })
}

#[derive(Debug, Clone, Copy)]
struct MergeStatus {
    merged: bool,
    /// True when merge state was verified via provider checks.
    verified: bool,
}

fn should_delete_remote_branches(merge_status: MergeStatus, merge_verified_by_land: bool) -> bool {
    merge_status.verified || (merge_verified_by_land && merge_status.merged)
}

/// Check if a stack is fully merged
fn check_stack_merged(
    repo: &git2::Repository,
    config: &Config,
    stack_name: &str,
    username: &str,
    provider: Option<&Provider>,
    silent: bool,
) -> Result<MergeStatus> {
    // Track whether any provider API call failed - if so, we cannot trust
    // verification and must be conservative about remote branch deletion.
    let mut had_provider_error = false;

    // Track whether we actually consulted the provider API. If we have a provider
    // configured but never called it (e.g., no MRs tracked), we cannot claim
    // verified=true based on provider existence alone.
    let mut provider_was_consulted = false;

    // Track MRs that are Closed (not Merged). These need ancestry fallback
    // because the commits may have been landed manually/directly.
    let mut any_explicitly_open = false;

    // Primary method: check if all MRs are merged.
    // This works correctly with squash and rebase merges.
    if let Some(stack_config) = config.get_stack(stack_name) {
        if !stack_config.mrs.is_empty() {
            if let Some(provider) = provider {
                let mut all_provider_merged = true;

                for (gg_id, mr_num) in &stack_config.mrs {
                    match provider.get_pr_info(*mr_num) {
                        Ok(info) => {
                            provider_was_consulted = true;
                            match info.state {
                                PrState::Merged => {
                                    // Explicitly merged - good
                                }
                                PrState::Open | PrState::Draft => {
                                    // Explicitly open/draft - NOT merged
                                    any_explicitly_open = true;
                                    all_provider_merged = false;
                                }
                                PrState::Closed => {
                                    // Closed but not merged. This can happen when:
                                    // - MR was closed without merging (genuinely unmerged)
                                    // - Commits were landed manually (cherry-pick, direct push)
                                    //   and MR was auto-closed or manually closed after.
                                    //
                                    // We cannot distinguish these cases from provider state alone.
                                    // Mark as not-provider-merged but allow ancestry fallback.
                                    all_provider_merged = false;
                                }
                            }
                        }
                        Err(e) => {
                            // PR/MR might be deleted or inaccessible.
                            // Log the error for debugging but continue checking other MRs.
                            if !silent {
                                eprintln!(
                                    "{} Could not fetch MR #{} ({}): {}",
                                    console::style("Debug:").dim(),
                                    mr_num,
                                    gg_id,
                                    e
                                );
                            }
                            had_provider_error = true;
                            provider_was_consulted = true; // We tried, it failed
                        }
                    }
                }

                // If we found any MR that is explicitly Open/Draft, the stack is not merged.
                // (Closed MRs are ambiguous and need ancestry check.)
                if any_explicitly_open {
                    return Ok(MergeStatus {
                        merged: false,
                        verified: true,
                    });
                }

                // Note: we intentionally do NOT call verify_commits_reachable here.
                // With squash or rebase merges, the local commit SHAs will differ from
                // what's on the base branch, so a revwalk check would incorrectly report
                // unmerged commits. The provider API confirmation is authoritative.

                // If all checked MRs are Merged (and NO provider errors occurred)
                if all_provider_merged && !had_provider_error {
                    return Ok(MergeStatus {
                        merged: true,
                        verified: true,
                    });
                }

                // If some MRs are Closed or had errors, fall through to ancestor check.
                // Closed MRs may have had their commits landed manually.
            }
        }
    }

    // Fallback: check if stack branch is ancestor of base.
    // This supports:
    // - Local/manual merge simulations when provider checks are unavailable
    // - Closed MRs where commits were landed manually (cherry-pick, direct push)
    // - Stacks with no tracked MRs
    let local_merged = is_stack_branch_ancestor_of_base(repo, config, stack_name, username)?;

    if !local_merged {
        return Ok(MergeStatus {
            merged: false,
            verified: false,
        });
    }

    // Stack is locally merged (ancestor of base). Now determine verification status.

    // SAFETY: If we had ANY provider error while checking MRs, we must NOT
    // allow verified=true. This prevents remote branch deletion when the
    // provider state is uncertain (network issues, stale refs, etc.).
    if had_provider_error {
        return Ok(MergeStatus {
            merged: true,
            verified: false,
        });
    }

    // SAFETY: If provider exists but was never consulted (no MRs tracked),
    // we cannot claim verification. The provider might have info we didn't check.
    // Remote branch deletion requires explicit provider confirmation OR
    // ancestry verification after successful provider consultation.
    if provider.is_some() && !provider_was_consulted {
        return Ok(MergeStatus {
            merged: true,
            verified: false,
        });
    }

    // Local check passed and either:
    // - No provider configured (local-only workflow)
    // - Provider was consulted without errors (but some MRs were Closed, not Merged)
    //
    // For the Closed MR case: if ancestry check passed, the commits ARE in base,
    // so they were landed somehow. Safe to clean up.
    //
    // For additional safety with a provider, also verify against remote base branch.
    // This provides confidence that the merge is complete on the server side.
    // IMPORTANT: For stacked PRs, we must verify that ALL entry branches are merged,
    // not just the stack tip. Otherwise we could delete remote branches for MRs that
    // are still open. This includes orphan entry branches discovered via pattern scan.
    let remote_verified = if provider.is_some() && provider_was_consulted {
        let stack_merged =
            is_stack_branch_ancestor_of_remote_base(repo, config, stack_name, username)
                .unwrap_or(false);
        let all_entries_merged =
            are_all_entry_branches_merged_to_remote_base(repo, config, stack_name, username)
                .unwrap_or(false);
        stack_merged && all_entries_merged
    } else {
        false
    };

    Ok(MergeStatus {
        merged: true,
        verified: remote_verified,
    })
}

fn is_stack_branch_ancestor_of_base(
    repo: &git2::Repository,
    config: &Config,
    stack_name: &str,
    username: &str,
) -> Result<bool> {
    let branch_name = git::format_stack_branch(username, stack_name);

    // Get base branch
    let base = config
        .get_base_for_stack(stack_name)
        .map(|s| s.to_string())
        .or_else(|| git::find_base_branch(repo).ok())
        .ok_or(GgError::NoBaseBranch)?;

    // Check if stack branch is ancestor of base (fully merged)
    let stack_ref = repo.revparse_single(&branch_name)?;
    let base_ref = repo
        .revparse_single(&base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", base)))?;

    let stack_oid = stack_ref.id();
    let base_oid = base_ref.id();

    Ok(repo.merge_base(stack_oid, base_oid)? == stack_oid)
}

/// Check if stack branch is an ancestor of the remote base branch (origin/<base>).
/// This provides stronger verification that the merge is complete on the server.
fn is_stack_branch_ancestor_of_remote_base(
    repo: &git2::Repository,
    config: &Config,
    stack_name: &str,
    username: &str,
) -> Result<bool> {
    let branch_name = git::format_stack_branch(username, stack_name);

    // Get base branch
    let base = config
        .get_base_for_stack(stack_name)
        .map(|s| s.to_string())
        .or_else(|| git::find_base_branch(repo).ok())
        .ok_or(GgError::NoBaseBranch)?;

    // Get stack branch commit
    let stack_ref = repo.revparse_single(&branch_name)?;
    let stack_oid = stack_ref.id();

    // Try to get origin/<base> - if it doesn't exist, we can't verify
    let remote_base = format!("origin/{}", base);
    let remote_ref = match repo.revparse_single(&remote_base) {
        Ok(r) => r,
        Err(_) => {
            // Remote tracking branch doesn't exist - can't verify
            return Ok(false);
        }
    };
    let remote_oid = remote_ref.id();

    // Check if stack branch is an ancestor of origin/<base>
    Ok(repo.merge_base(stack_oid, remote_oid)? == stack_oid)
}

/// Collect all entry branches for a stack that would be targeted for deletion.
/// This includes both branches from config.mrs AND orphan branches discovered via pattern scan.
/// This mirrors the discovery logic in `delete_entry_branches`.
fn collect_all_entry_branches(
    repo: &git2::Repository,
    config: &Config,
    stack_name: &str,
    username: &str,
) -> Vec<String> {
    let mut branches = Vec::new();

    // Collect entry branches from config
    if let Some(stack_config) = config.get_stack(stack_name) {
        for entry_id in stack_config.mrs.keys() {
            if let Some(normalized_id) = git::normalize_gg_id(entry_id) {
                let entry_branch = git::format_entry_branch(username, stack_name, &normalized_id);
                branches.push(entry_branch);
            }
        }
    }

    // Also scan for orphan entry branches matching this stack (pattern scan)
    // This mirrors the discovery logic in delete_entry_branches
    if let Ok(branch_iter) = repo.branches(Some(BranchType::Local)) {
        for branch_result in branch_iter.flatten() {
            if let Ok(Some(name)) = branch_result.0.name() {
                // Match branches like "username/stack-name--c-XXXXX"
                if let Some((branch_user, branch_stack, _)) = git::parse_entry_branch(name) {
                    if branch_user == username && branch_stack == *stack_name {
                        let name_string = name.to_string();
                        if !branches.contains(&name_string) {
                            branches.push(name_string);
                        }
                    }
                }
            }
        }
    }

    branches
}

/// Check if ALL entry branches in the stack are ancestors of the remote base branch.
/// This is crucial for stacked PRs: the stack tip being merged doesn't mean all
/// intermediate entry branches' MRs were merged. We must verify each one.
/// This includes orphan entry branches discovered via pattern scan (same as delete_entry_branches).
fn are_all_entry_branches_merged_to_remote_base(
    repo: &git2::Repository,
    config: &Config,
    stack_name: &str,
    username: &str,
) -> Result<bool> {
    // Get base branch
    let base = config
        .get_base_for_stack(stack_name)
        .map(|s| s.to_string())
        .or_else(|| git::find_base_branch(repo).ok())
        .ok_or(GgError::NoBaseBranch)?;

    // Get remote base commit
    let remote_base = format!("origin/{}", base);
    let remote_ref = match repo.revparse_single(&remote_base) {
        Ok(r) => r,
        Err(_) => {
            // Remote tracking branch doesn't exist - can't verify
            return Ok(false);
        }
    };
    let remote_oid = remote_ref.id();

    // Collect ALL entry branches (from config + orphan pattern scan)
    let entry_branches = collect_all_entry_branches(repo, config, stack_name, username);

    // Verify each entry branch is an ancestor of remote base
    for entry_branch in entry_branches {
        // Try to get the entry branch - if it doesn't exist locally, it's already been deleted
        // (which is fine - it means it was already cleaned up)
        let entry_ref = match repo.revparse_single(&entry_branch) {
            Ok(r) => r,
            Err(_) => {
                // Entry branch doesn't exist locally - skip (already deleted)
                continue;
            }
        };
        let entry_oid = entry_ref.id();

        // Check if this entry branch is an ancestor of remote base
        let merge_base = repo.merge_base(entry_oid, remote_oid)?;
        if merge_base != entry_oid {
            // This entry branch is NOT merged to remote base - can't verify
            return Ok(false);
        }
    }

    // All entry branches are either merged or don't exist locally
    Ok(true)
}

#[allow(dead_code)]
/// Verify that all commits in the stack are reachable from the base branch
/// This provides additional safety before deleting remote branches
fn verify_commits_reachable(
    repo: &git2::Repository,
    config: &Config,
    stack_name: &str,
    username: &str,
) -> Result<()> {
    let branch_name = git::format_stack_branch(username, stack_name);

    // Get base branch
    let base = config
        .get_base_for_stack(stack_name)
        .map(|s| s.to_string())
        .or_else(|| git::find_base_branch(repo).ok())
        .ok_or(GgError::NoBaseBranch)?;

    // Get base commit (prefer origin/<base> for most up-to-date state)
    let base_ref = repo
        .revparse_single(&format!("origin/{}", base))
        .or_else(|_| repo.revparse_single(&base))?;
    let base_commit = base_ref.peel_to_commit()?;

    // Get stack branch commit
    let stack_ref = repo.revparse_single(&branch_name)?;
    let stack_commit = stack_ref.peel_to_commit()?;

    // Walk commits from stack to base
    let mut revwalk = repo.revwalk()?;
    revwalk.push(stack_commit.id())?;
    revwalk.hide(base_commit.id())?;

    // If there are any commits not reachable from base, it's not fully merged
    if let Some(oid) = revwalk.next() {
        let _oid = oid?;
        // If we get here, there are commits in stack not in base
        return Err(GgError::Other(format!(
            "Stack '{}' has commits not reachable from {}",
            stack_name, base
        )));
    }

    Ok(())
}

/// Delete all entry branches for a stack (both local and remote)
fn delete_entry_branches(
    repo: &git2::Repository,
    config: &mut Config,
    stack_name: &str,
    username: &str,
    delete_remote: bool,
    silent: bool,
    record_remote_effect: &mut dyn FnMut(RemoteDeletionCheckpoint, &mut Config) -> Result<()>,
) -> Result<()> {
    // First, delete entry branches from config (if any)
    let configured_entry_ids = config
        .get_stack(stack_name)
        .map(|stack_config| stack_config.mrs.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    for entry_id in configured_entry_ids {
        let entry_id = match git::normalize_gg_id(&entry_id) {
            Some(id) => id,
            None => {
                if !silent {
                    println!(
                        "{} Skipping invalid GG-ID '{}' in config.",
                        style("Warning:").yellow(),
                        entry_id
                    );
                }
                continue;
            }
        };
        let entry_branch = git::format_entry_branch(username, stack_name, &entry_id);
        // Delete remote entry branch first so a remote failure leaves the local ref
        // discoverable for a retry.
        if delete_remote {
            if let Some(effect) =
                delete_remote_branch(repo, config, &entry_branch, record_remote_effect)?
            {
                complete_remote_deletion(repo, config, effect, record_remote_effect)?;
            }
        }
        // Delete local entry branch
        if let Ok(mut branch) = repo.find_branch(&entry_branch, BranchType::Local) {
            // Check if branch is HEAD of a worktree
            if let Some(wt_name) = git::is_branch_checked_out_in_worktree(repo, &entry_branch) {
                // Try to prune if stale
                if !git::try_prune_worktree(repo, &wt_name) {
                    // Worktree still exists - try to remove it silently
                    let _ = git::remove_worktree(&wt_name);
                }
            }
            // Try to delete, ignore errors (best effort for entry branches)
            let _ = branch.delete();
        }
    }

    // Also scan for any orphaned entry branches matching this stack
    // (in case config is out of sync with actual branches)
    let branches: Vec<String> = repo
        .branches(Some(BranchType::Local))
        .ok()
        .map(|branches| {
            branches
                .filter_map(|b| b.ok())
                .filter_map(|(branch, _)| branch.name().ok().flatten().map(String::from))
                .filter(|name| {
                    // Match branches like "username/stack-name--c-XXXXX"
                    if let Some((branch_user, branch_stack, _)) = git::parse_entry_branch(name) {
                        branch_user == username && branch_stack == *stack_name
                    } else {
                        false
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    for branch_name in branches {
        // Delete remote first so a remote failure leaves the local orphan ref
        // discoverable for a retry.
        if delete_remote {
            if let Some(effect) =
                delete_remote_branch(repo, config, &branch_name, record_remote_effect)?
            {
                complete_remote_deletion(repo, config, effect, record_remote_effect)?;
            }
        }
        if let Ok(mut branch) = repo.find_branch(&branch_name, BranchType::Local) {
            // Check if branch is HEAD of a worktree
            if let Some(wt_name) = git::is_branch_checked_out_in_worktree(repo, &branch_name) {
                // Try to prune if stale
                if !git::try_prune_worktree(repo, &wt_name) {
                    // Worktree still exists - try to remove it silently
                    let _ = git::remove_worktree(&wt_name);
                }
            }
            // Try to delete, ignore errors (best effort for entry branches)
            let _ = branch.delete();
        }
    }
    Ok(())
}

fn complete_remote_deletion(
    repo: &Repository,
    config: &mut Config,
    effect: RemoteEffect,
    record_remote_effect: &mut dyn FnMut(RemoteDeletionCheckpoint, &mut Config) -> Result<()>,
) -> Result<()> {
    let RemoteEffect::BranchDeleted { remote, branch, .. } = &effect else {
        return Err(GgError::DurabilityCheckpoint(
            "remote deletion completed without a branch_deleted effect".to_string(),
        ));
    };
    let remote = remote.clone();
    let branch = branch.clone();
    record_remote_effect(RemoteDeletionCheckpoint::Effect(effect), config)?;
    config
        .pending_remote_branch_deletions
        .retain(|intent| intent.remote != remote || intent.branch != branch);
    config.save(repo.commondir()).map_err(|error| {
        GgError::DurabilityCheckpoint(format!(
            "failed to persist completed remote deletion for '{branch}': {error}"
        ))
    })?;
    record_remote_effect(RemoteDeletionCheckpoint::Complete, config)
}

fn delete_remote_branch(
    repo: &Repository,
    config: &mut Config,
    branch: &str,
    record_remote_effect: &mut dyn FnMut(RemoteDeletionCheckpoint, &mut Config) -> Result<()>,
) -> Result<Option<RemoteEffect>> {
    let Some(prior_oid) = git::get_live_remote_branch_oid(repo, "origin", branch)? else {
        return Ok(None);
    };
    record_remote_effect(
        RemoteDeletionCheckpoint::BeforeAttempt {
            remote: "origin".to_string(),
            branch: branch.to_string(),
            prior_oid: prior_oid.to_string(),
        },
        config,
    )?;
    if let Err(delete_error) = git::delete_remote_branch_at_oid(repo, "origin", branch, prior_oid) {
        return Err(GgError::RemoteMutationUncertain(delete_error.to_string()));
    }
    Ok(Some(RemoteEffect::BranchDeleted {
        remote: "origin".to_string(),
        branch: branch.to_string(),
        prior_oid: Some(prior_oid.to_string()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_deletion_segment_rejects_complete_before_exact_effect() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let repo = Repository::init(temp.path()).expect("init repo");
        let mut config = Config::default();
        let args = vec!["clean".to_string()];
        let mut guard = Some(
            git::begin_recorded_op(
                &repo,
                &config,
                OperationKind::Clean,
                args.clone(),
                Some("stack".to_string()),
                SnapshotScope::AllUserBranches,
            )
            .expect("begin initial clean segment"),
        );
        let mut remote_effects = Vec::new();
        let mut touched_remote = false;
        let mut owner = RemoteDeletionSegmentOwner::new(
            &repo,
            OperationKind::Clean,
            "clean",
            &args,
            Some("stack"),
            &mut guard,
            &mut remote_effects,
            &mut touched_remote,
        );
        owner
            .checkpoint(
                &mut config,
                RemoteDeletionCheckpoint::BeforeAttempt {
                    remote: "origin".to_string(),
                    branch: "user/stack--c-1111111".to_string(),
                    prior_oid: "1111111111111111111111111111111111111111".to_string(),
                },
            )
            .expect("persist pre-attempt uncertainty");

        let error = owner
            .checkpoint(&mut config, RemoteDeletionCheckpoint::Complete)
            .expect_err("complete without an exact effect must be rejected");

        assert!(
            error.to_string().contains(
                "invalid clean remote deletion checkpoint transition: AttemptStarted -> Complete"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn normal_clean_deletes_remote_branches_only_when_verified() {
        assert!(should_delete_remote_branches(
            MergeStatus {
                merged: true,
                verified: true,
            },
            false,
        ));

        assert!(!should_delete_remote_branches(
            MergeStatus {
                merged: true,
                verified: false,
            },
            false,
        ));
    }

    #[test]
    fn verified_land_allows_remote_branch_deletion_after_config_mappings_are_removed() {
        assert!(should_delete_remote_branches(
            MergeStatus {
                merged: true,
                verified: false,
            },
            true,
        ));
    }

    #[test]
    fn verified_land_does_not_allow_remote_branch_deletion_for_unmerged_stack() {
        assert!(!should_delete_remote_branches(
            MergeStatus {
                merged: false,
                verified: false,
            },
            true,
        ));
    }

    #[test]
    fn delete_remote_branch_reports_git_errors() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let repo = Repository::init(temp.path()).expect("init repo");

        let mut config = Config::default();
        let error =
            delete_remote_branch(&repo, &mut config, "u/cleanup--c-1111111", &mut |_, _| {
                Ok(())
            })
            .unwrap_err();

        assert!(
            error.to_string().contains("git ls-remote failed"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn verified_land_keeps_stack_retryable_when_remote_entry_cleanup_fails() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let repo_path = temp.path().join("repo");
        std::fs::create_dir(&repo_path).expect("create repo dir");

        let init = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&repo_path)
            .output()
            .expect("git init");
        assert!(
            init.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&repo_path)
            .output()
            .expect("git config email");
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&repo_path)
            .output()
            .expect("git config name");
        std::fs::write(repo_path.join("README.md"), "test\n").expect("write readme");
        std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&repo_path)
            .output()
            .expect("git add");
        let commit = std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&repo_path)
            .output()
            .expect("git commit");
        assert!(
            commit.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        );
        std::process::Command::new("git")
            .args(["branch", "u/cleanup"])
            .current_dir(&repo_path)
            .output()
            .expect("git branch");

        let gg_dir = repo_path.join(".git/gg");
        std::fs::create_dir_all(&gg_dir).expect("create gg dir");
        std::fs::write(
            gg_dir.join("config.json"),
            serde_json::json!({
                "defaults": {"branch_username": "u", "base": "main"},
                "stacks": {"cleanup": {"mrs": {"c-1111111": 1}}}
            })
            .to_string(),
        )
        .expect("write config");

        let repo = Repository::open(&repo_path).expect("open repo");
        let result = run_for_stack_with_repo_after_verified_land(
            &repo,
            "cleanup",
            true,
            true,
            &mut |_, _| Ok(()),
        );

        assert!(result.is_err(), "missing origin should fail remote cleanup");
        assert!(
            repo.find_branch("u/cleanup", BranchType::Local).is_ok(),
            "stack branch must remain retryable"
        );
        assert!(
            Config::load_with_global(repo.commondir())
                .expect("reload config")
                .get_stack("cleanup")
                .is_some(),
            "stack config must remain retryable"
        );
    }

    #[test]
    fn verified_land_keeps_orphan_entry_retryable_when_remote_cleanup_fails() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let repo_path = temp.path().join("repo");
        std::fs::create_dir(&repo_path).expect("create repo dir");

        let init = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&repo_path)
            .output()
            .expect("git init");
        assert!(
            init.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&repo_path)
            .output()
            .expect("git config email");
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&repo_path)
            .output()
            .expect("git config name");
        std::fs::write(repo_path.join("README.md"), "test\n").expect("write readme");
        std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&repo_path)
            .output()
            .expect("git add");
        let commit = std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&repo_path)
            .output()
            .expect("git commit");
        assert!(
            commit.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        );
        std::process::Command::new("git")
            .args(["branch", "u/cleanup"])
            .current_dir(&repo_path)
            .output()
            .expect("git branch");
        std::process::Command::new("git")
            .args(["branch", "u/cleanup--c-1111111"])
            .current_dir(&repo_path)
            .output()
            .expect("git branch");

        let gg_dir = repo_path.join(".git/gg");
        std::fs::create_dir_all(&gg_dir).expect("create gg dir");
        std::fs::write(
            gg_dir.join("config.json"),
            serde_json::json!({
                "defaults": {"branch_username": "u", "base": "main"},
                "stacks": {"cleanup": {}}
            })
            .to_string(),
        )
        .expect("write config");

        let repo = Repository::open(&repo_path).expect("open repo");
        let result = run_for_stack_with_repo_after_verified_land(
            &repo,
            "cleanup",
            true,
            true,
            &mut |_, _| Ok(()),
        );

        assert!(result.is_err(), "missing origin should fail remote cleanup");
        assert!(
            repo.find_branch("u/cleanup--c-1111111", BranchType::Local)
                .is_ok(),
            "orphan entry branch must remain retryable"
        );
    }

    #[test]
    fn verified_land_validates_before_removing_configured_worktree() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let repo_path = temp.path().join("repo");
        std::fs::create_dir(&repo_path).expect("create repo dir");

        let init = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&repo_path)
            .output()
            .expect("git init");
        assert!(
            init.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );

        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&repo_path)
            .output()
            .expect("git config email");
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&repo_path)
            .output()
            .expect("git config name");
        std::fs::write(repo_path.join("README.md"), "test\n").expect("write readme");
        std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&repo_path)
            .output()
            .expect("git add");
        let commit = std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&repo_path)
            .output()
            .expect("git commit");
        assert!(
            commit.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        );

        let worktree_path = temp.path().join("configured-worktree");
        let worktree = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "--detach",
                worktree_path.to_str().unwrap(),
                "HEAD",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("git worktree add");
        assert!(
            worktree.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&worktree.stderr)
        );

        let gg_dir = repo_path.join(".git/gg");
        std::fs::create_dir_all(&gg_dir).expect("create gg dir");
        std::fs::write(
            gg_dir.join("config.json"),
            serde_json::json!({
                "defaults": {"branch_username": "bad/name", "base": "main"},
                "stacks": {"cleanup": {"worktree_path": worktree_path.display().to_string()}}
            })
            .to_string(),
        )
        .expect("write config");

        let repo = Repository::open(&repo_path).expect("open repo");
        let result = run_for_stack_with_repo_after_verified_land(
            &repo,
            "cleanup",
            true,
            false,
            &mut |_, _| Ok(()),
        );

        assert!(result.is_err(), "invalid username should fail validation");
        assert!(
            worktree_path.exists(),
            "configured worktree must not be removed before validation"
        );
        let config = Config::load_with_global(repo.commondir()).expect("reload config");
        assert_eq!(
            config
                .get_stack("cleanup")
                .and_then(|stack| stack.worktree_path.as_deref()),
            Some(worktree_path.to_str().unwrap())
        );
    }

    #[test]
    fn verified_land_reports_incomplete_branch_cleanup() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let repo_path = temp.path().join("repo");
        std::fs::create_dir(&repo_path).expect("create repo dir");

        let init = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&repo_path)
            .output()
            .expect("git init");
        assert!(
            init.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&repo_path)
            .output()
            .expect("git config email");
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&repo_path)
            .output()
            .expect("git config name");
        std::fs::write(repo_path.join("README.md"), "test\n").expect("write readme");
        std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&repo_path)
            .output()
            .expect("git add");
        let commit = std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&repo_path)
            .output()
            .expect("git commit");
        assert!(
            commit.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        );
        std::process::Command::new("git")
            .args(["branch", "u/cleanup"])
            .current_dir(&repo_path)
            .output()
            .expect("git branch");

        let worktree_path = temp.path().join("stack-worktree");
        let worktree = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                worktree_path.to_str().unwrap(),
                "u/cleanup",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("git worktree add");
        assert!(
            worktree.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&worktree.stderr)
        );
        let lock = std::process::Command::new("git")
            .args(["worktree", "lock", worktree_path.to_str().unwrap()])
            .current_dir(&repo_path)
            .output()
            .expect("git worktree lock");
        assert!(
            lock.status.success(),
            "git worktree lock failed: {}",
            String::from_utf8_lossy(&lock.stderr)
        );

        let gg_dir = repo_path.join(".git/gg");
        std::fs::create_dir_all(&gg_dir).expect("create gg dir");
        std::fs::write(
            gg_dir.join("config.json"),
            serde_json::json!({
                "defaults": {"branch_username": "u", "base": "main"},
                "stacks": {"cleanup": {}}
            })
            .to_string(),
        )
        .expect("write config");

        let repo = Repository::open(&repo_path).expect("open repo");
        let result = run_for_stack_with_repo_after_verified_land(
            &repo,
            "cleanup",
            true,
            true,
            &mut |_, _| Ok(()),
        );

        assert!(!result.expect("cleanup should not error"));
        assert!(
            Config::load_with_global(repo.commondir())
                .expect("reload config")
                .get_stack("cleanup")
                .is_some(),
            "incomplete cleanup must not remove stack config"
        );
    }
}
