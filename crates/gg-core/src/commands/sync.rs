//! `gg sync` - Sync stack with remote provider (push branches and create/update PRs/MRs)

mod after_land;

use console::style;
use dialoguer::Confirm;
use git2::Repository;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::collections::HashSet;
use std::time::Duration;

use crate::config::{Config, GithubStacksIntegration, RemoteBranchDeletionState};
use crate::error::{GgError, Result};
use crate::git::{self, get_commit_description, strip_gg_id_from_message};
use crate::github_stacks::{
    reconcile_with_gh_before_mutation, GithubStackAction, GithubStackReason, GithubStackSyncResult,
};
use crate::managed_body;
use crate::operations::{self, OperationGuard, OperationKind, RemoteEffect, SnapshotScope};
use crate::output::{
    print_json, StreamingJson, SyncEntryResultJson, SyncMetadataJson, SyncResponse, SyncResultJson,
    SyncStreamingEvent, SyncStreamingResponse, OUTPUT_VERSION,
};
use crate::provider::Provider;
use crate::stack::{resolve_target, Stack};
use crate::stack_nav;
use crate::template::{self, TemplateContext};

/// Per-entry state captured during the main sync loop that the nav-comment
/// reconcile pass needs. Populated only for entries whose PR exists.
struct NavEntrySnapshot {
    pr_number: u64,
    pr_state: stack_nav::PrEntryState,
    /// Index into `json_entries` so we can attach the nav action result.
    json_index: usize,
}

struct SyncSegment {
    guard: Option<OperationGuard>,
    remote_effects: Vec<RemoteEffect>,
    touched_remote: bool,
}

impl SyncSegment {
    fn new() -> Self {
        Self {
            guard: None,
            remote_effects: Vec::new(),
            touched_remote: false,
        }
    }

    fn ensure_started(&mut self, repo: &Repository, config: &Config) -> Result<()> {
        if self.guard.is_none() {
            self.guard = Some(git::begin_recorded_op(
                repo,
                config,
                OperationKind::Sync,
                std::env::args().skip(1).collect(),
                None,
                SnapshotScope::AllUserBranches,
            )?);
        }
        Ok(())
    }

    fn guard_mut(&mut self) -> &mut OperationGuard {
        self.guard.as_mut().expect("sync segment must be started")
    }

    fn mark_remote_before_attempt(&mut self) -> Result<()> {
        self.guard_mut().mark_touched_remote_before_attempt()?;
        self.touched_remote = true;
        Ok(())
    }

    fn mark_remote(&mut self) {
        self.guard_mut().mark_touched_remote();
        self.touched_remote = true;
    }

    fn record_remote_effect(&mut self, effect: RemoteEffect) {
        self.guard_mut().record_remote_effect(effect.clone());
        self.remote_effects.push(effect);
        self.touched_remote = true;
    }

    fn record_remote_effect_durable(&mut self, effect: RemoteEffect) -> Result<()> {
        self.guard_mut()
            .record_remote_effect_durable(effect.clone())?;
        self.remote_effects.push(effect);
        self.touched_remote = true;
        Ok(())
    }

    fn finish(&mut self, repo: &Repository, config: &Config) -> Result<()> {
        let Some(guard) = self.guard.take() else {
            return Ok(());
        };
        guard.finalize_with_scope(
            repo,
            config,
            SnapshotScope::AllUserBranches,
            std::mem::take(&mut self.remote_effects),
            std::mem::take(&mut self.touched_remote),
        )
    }

    fn finish_result<T>(
        &mut self,
        repo: &Repository,
        config: &Config,
        result: Result<T>,
    ) -> Result<T> {
        match (result, self.finish(repo, config)) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(finalize_error)) => Err(GgError::Other(format!(
                "{error}; additionally failed to finalize sync operation: {finalize_error}"
            ))),
        }
    }

    fn finish_on_error<T, E>(
        &mut self,
        repo: &Repository,
        config: &Config,
        result: std::result::Result<T, E>,
    ) -> Result<T>
    where
        E: Into<GgError>,
    {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                let error = error.into();
                if matches!(error, GgError::RebaseConflict) || git::is_rebase_in_progress(repo) {
                    let guard = self.guard.as_ref().expect(
                        "an interrupted sync rebase must belong to the active sync operation",
                    );
                    if let Err(marker_error) =
                        operations::remember_interrupted_rebase_operation(repo, guard.id())
                    {
                        return Err(GgError::Other(format!(
                            "{error}; additionally failed to remember interrupted sync operation {}: {marker_error}",
                            guard.id()
                        )));
                    }
                    Err(error)
                } else {
                    self.finish_result(repo, config, Err(error))
                }
            }
        }
    }
}

fn github_stack_preflight_skip(
    mode: GithubStacksIntegration,
    until: Option<&str>,
    active_pr_numbers: &[u64],
    has_unresolved_active_pr: bool,
) -> Option<GithubStackSyncResult> {
    let reason = if mode == GithubStacksIntegration::Off {
        GithubStackReason::Disabled
    } else if until.is_some() {
        GithubStackReason::PartialSync
    } else if has_unresolved_active_pr {
        GithubStackReason::UnresolvedPrs
    } else if active_pr_numbers.len() < 2 {
        GithubStackReason::InsufficientPrs
    } else {
        return None;
    };

    Some(GithubStackSyncResult::skipped(
        mode,
        reason,
        active_pr_numbers.to_vec(),
    ))
}

fn sync_progress_bar(len: u64, draw_target: ProgressDrawTarget) -> ProgressBar {
    let progress_bar = ProgressBar::with_draw_target(Some(len), draw_target);
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("=>-"),
    );
    progress_bar.enable_steady_tick(Duration::from_millis(100));
    progress_bar
}

fn suspend_sync_progress<R>(progress_bar: &ProgressBar, action: impl FnOnce() -> R) -> R {
    progress_bar.suspend(action)
}

/// Apply the configured threshold and prompt to ordinary behind-base stacks.
fn maybe_rebase_if_base_is_behind(
    repo: &Repository,
    config: &Config,
    base_branch: &str,
    json: bool,
    jsonl: bool,
    rebase: impl FnOnce() -> Result<()>,
) -> Result<bool> {
    let threshold = config.get_sync_behind_threshold();
    if threshold == 0 {
        return Ok(false);
    }

    // Use merge-base to find how many commits on origin/<base_branch> are not
    // reachable from HEAD. This correctly detects when a branch needs rebasing
    // regardless of what local <base_branch> looks like.
    let behind =
        match git::count_branch_behind_upstream(repo, "HEAD", &format!("origin/{}", base_branch)) {
            Ok(count) => count,
            Err(_) => return Ok(false),
        };

    if behind < threshold {
        return Ok(false);
    }

    let prs_label = Provider::detect(repo)
        .ok()
        .map(|provider| format!("{}s", provider.pr_label()))
        .unwrap_or_else(|| "PRs/MRs".to_string());

    if config.get_sync_auto_rebase() {
        if !json && !jsonl {
            println!(
                "{} Your stack is {} commits behind origin/{}. {} may show unrelated changes. Auto-rebasing...",
                style("⚠").yellow().bold(),
                behind,
                base_branch,
                prs_label
            );
        }
        // Internal auto-rebase during sync: the user hasn't been asked to
        // --force, so respect the immutability guard rather than silently
        // bypassing it.
        match rebase() {
            Ok(()) => return Ok(true),
            Err(GgError::ImmutableTargets(msg)) => {
                return Err(GgError::ImmutableTargetsDuringSync(msg));
            }
            Err(e) => return Err(e),
        }
    }

    if !json && !jsonl {
        println!(
            "{} Your stack is {} commits behind origin/{}. {} may show unrelated changes. Run 'gg rebase' first to update.",
            style("⚠").yellow().bold(),
            behind,
            base_branch,
            prs_label
        );
    }

    if json || jsonl {
        return Ok(false);
    }

    let should_rebase = Confirm::new()
        .with_prompt("Rebase before syncing?")
        .default(true)
        .interact()
        .unwrap_or(true);

    if should_rebase {
        match rebase() {
            Ok(()) => return Ok(true),
            Err(GgError::ImmutableTargets(msg)) => {
                return Err(GgError::ImmutableTargetsDuringSync(msg));
            }
            Err(e) => return Err(e),
        }
    }

    Ok(false)
}

/// Format and display a push error with helpful context
fn format_push_error(error: &GgError, branch_name: &str) {
    match error {
        GgError::PushFailed {
            branch,
            hook_error,
            git_error,
        } => {
            println!();
            println!(
                "{} Push failed for {}",
                style("✗").red().bold(),
                style(branch).cyan()
            );
            println!();

            // Display hook error if present
            if let Some(hook_msg) = hook_error {
                println!("{}", style("Pre-push hook failed:").yellow().bold());

                // Indent the hook error output
                for line in hook_msg.lines() {
                    println!("  {}", line);
                }
                println!();

                println!("{}", style("Suggestion:").cyan().bold());
                println!("  Fix the issue, then retry {}", style("`gg sync`").green());
            }

            // Display git error if present (and different from hook error)
            if let Some(git_msg) = git_error {
                if hook_error.is_none() {
                    // No hook error, so this is the main issue
                    println!("{}", style("Git error:").red().bold());
                    for line in git_msg.lines() {
                        println!("  {}", line);
                    }
                    println!();
                }
            }

            // If no specific errors were captured, show generic message
            if hook_error.is_none() && git_error.is_none() {
                println!("  The push command failed without a clear error message.");
                println!("  This might be due to network issues or server-side hooks.");
                println!();
            }
        }
        _ => {
            // For other error types, show the error as-is
            println!();
            println!(
                "{} Push failed for {}: {}",
                style("✗").red().bold(),
                style(branch_name).cyan(),
                error
            );
            println!();
        }
    }
}

/// Compute the target branch for entry at position `i` in a sync loop.
///
/// Walks backwards through `entry_is_closed` to find the nearest predecessor
/// that is not merged/closed. If all predecessors are merged, returns `base`.
/// This ensures downstream MRs are retargeted away from merged intermediate
/// branches (fixes GitLab stacked MR retargeting — see #297).
fn compute_target_branch(
    i: usize,
    base: &str,
    entries: &[crate::stack::StackEntry],
    entry_is_closed: &[bool],
    stack: &Stack,
) -> String {
    if i == 0 {
        return base.to_string();
    }
    for j in (0..i).rev() {
        if !entry_is_closed[j] {
            return stack.entry_branch_name(&entries[j]).unwrap();
        }
    }
    base.to_string()
}

/// Return the targets an already-published review may currently have.
///
/// These targets describe the published topology independently of the current
/// local order. The desired target is handled separately by
/// `compute_target_branch`.
fn expected_current_target_branches(
    repo: &Repository,
    config: &Config,
    i: usize,
    base: &str,
    entries: &[crate::stack::StackEntry],
    stack: &Stack,
    previous_review: Option<&crate::provider::PrInfo>,
) -> Vec<String> {
    let entry_branch = stack
        .entry_branch_name(&entries[i])
        .expect("synced entries have GG IDs");
    let mut targets = Vec::new();
    let mut add_target = |target: String| {
        if target != entry_branch && !targets.contains(&target) {
            targets.push(target);
        }
    };

    // A retained review can still target a dropped predecessor until this sync
    // retargets it. Reconstruct that predecessor only from the published
    // branch's exact parent and a pending deletion for this stack.
    let published_oid = git::get_remote_branch_oid(repo, &entry_branch);
    let published_parent = published_oid
        .and_then(|oid| repo.find_commit(oid).ok())
        .and_then(|commit| commit.parent_id(0).ok());
    if let Some(branch) = published_parent.and_then(|parent| {
        config
            .pending_remote_branch_deletions
            .iter()
            .find(|intent| {
                intent.remote == "origin"
                    && intent.expected_oid.parse::<git2::Oid>().ok() == Some(parent)
                    && matches!(
                        git::parse_entry_branch(&intent.branch),
                        Some((ref user, ref name, ref entry_id))
                            if user == &stack.username
                                && name == &stack.name
                                && git::normalize_gg_id(entry_id).is_some()
                    )
            })
            .map(|intent| intent.branch.clone())
    }) {
        add_target(branch);
    }

    // A previous sync may have pushed the rewritten survivor but failed before
    // retargeting its review. The exact remote/local source match confirms that
    // push landed; in that state, durable same-stack deletion intents whose
    // freshly fetched branch tips still match their authorized OIDs recover the
    // old target even though it is no longer the source commit's parent.
    // Other stacks, malformed entry branches, changed deletion targets, and
    // stale source tips remain unauthorized.
    if published_oid == Some(entries[i].oid) {
        for intent in &config.pending_remote_branch_deletions {
            if intent.remote == "origin"
                && intent
                    .expected_oid
                    .parse::<git2::Oid>()
                    .ok()
                    .is_some_and(|expected_oid| {
                        git::get_remote_branch_oid(repo, &intent.branch) == Some(expected_oid)
                    })
                && matches!(
                    git::parse_entry_branch(&intent.branch),
                    Some((ref user, ref name, ref entry_id))
                        if user == &stack.username
                            && name == &stack.name
                            && git::normalize_gg_id(entry_id).is_some()
                )
            {
                add_target(intent.branch.clone());
            }
        }
    }

    // The configured base and every other mapped current entry are the only
    // targets that can belong to the previously published stack after a local
    // reorder. Unmapped or unrelated branches remain unauthorized.
    add_target(base.to_string());
    for candidate in &stack.entries {
        let Some(gg_id) = candidate.gg_id.as_deref() else {
            continue;
        };
        if config.get_mr_for_entry(&stack.name, gg_id).is_some() {
            if let Some(branch) = stack.entry_branch_name(candidate) {
                add_target(branch);
            }
        }
    }

    // Preserve renamed-head replacement topology: a following review may
    // still target the exact source observed for the previously classified
    // mapped review until replacements are created.
    if let Some(branch) = previous_review.and_then(|info| info.head_branch.clone()) {
        add_target(branch);
    }

    targets
}

/// Delete remote entry branches authorized by earlier Drops.
///
/// Returns one message per branch whose deletion was deferred because an open
/// PR/MR still targets it, or because that could not be verified. Deferred
/// intents stay pending for a later sync or clean.
pub(crate) fn reconcile_pending_remote_entry_deletions(
    repo: &Repository,
    provider: Option<&Provider>,
    username: &str,
    stack_name: &str,
    current_branches: &HashSet<String>,
    config: &mut Config,
    mut record_remote_effect: impl FnMut(Option<RemoteEffect>) -> Result<()>,
) -> Result<Vec<String>> {
    let before = config.pending_remote_branch_deletions.len();
    config
        .pending_remote_branch_deletions
        .retain(|intent| !current_branches.contains(&intent.branch));
    if config.pending_remote_branch_deletions.len() != before {
        config.save(repo.commondir())?;
    }

    let intents = config
        .pending_remote_branch_deletions
        .iter()
        .filter(|intent| {
            matches!(
                git::parse_entry_branch(&intent.branch),
                Some((ref user, ref name, ref entry_id))
                    if user == username
                        && name == stack_name
                        && git::normalize_gg_id(entry_id).is_some()
            )
        })
        .cloned()
        .collect::<Vec<_>>();

    let mut deferred = Vec::new();
    for intent in intents {
        if intent.state == RemoteBranchDeletionState::Deleting {
            return Err(GgError::Other(format!(
                "Remote deletion of '{}' has an uncertain prior outcome. Inspect '{}' and remove or reset its pending_remote_branch_deletions entry before retrying.",
                intent.branch,
                Config::config_path(repo.commondir()).display()
            )));
        }

        let expected_oid = git2::Oid::from_str(&intent.expected_oid)?;
        let live_oid = git::get_live_remote_branch_oid(repo, &intent.remote, &intent.branch)?;
        let Some(live_oid) = live_oid else {
            config.pending_remote_branch_deletions.retain(|candidate| {
                candidate.remote != intent.remote || candidate.branch != intent.branch
            });
            config.save(repo.commondir())?;
            continue;
        };
        if live_oid != expected_oid {
            return Err(GgError::Other(format!(
                "Refusing to delete remote branch '{}': it changed after the local drop (authorized {}, now {}). Keep the branch or drop its current version explicitly.",
                intent.branch, expected_oid, live_oid
            )));
        }

        // Deleting a branch that an open review still targets would close or
        // break that review. Keep the intent until nothing targets it.
        let dependents = match provider {
            Some(provider) => provider.list_prs_targeting_branch(&intent.branch),
            None => Err(GgError::Other("no provider detected".to_string())),
        };
        match dependents {
            Ok(numbers) if numbers.is_empty() => {}
            Ok(numbers) => {
                let provider = provider.expect("dependents were listed by a provider");
                let numbers = numbers
                    .iter()
                    .map(|n| format!("{}{n}", provider.pr_number_prefix()))
                    .collect::<Vec<_>>()
                    .join(", ");
                deferred.push(format!(
                    "Kept remote branch '{}': open {} {} still targets it. Retarget them (a full `gg sync` does), then sync again.",
                    intent.branch,
                    provider.pr_label(),
                    numbers
                ));
                continue;
            }
            Err(error) => {
                deferred.push(format!(
                    "Kept remote branch '{}': could not check for open reviews targeting it ({error}).",
                    intent.branch
                ));
                continue;
            }
        }

        let stored = config
            .pending_remote_branch_deletions
            .iter_mut()
            .find(|candidate| {
                candidate.remote == intent.remote && candidate.branch == intent.branch
            })
            .expect("cloned pending deletion must still exist");
        stored.state = RemoteBranchDeletionState::Deleting;
        config.save(repo.commondir())?;

        // Any unsuccessful push is ambiguous: the server may have applied the
        // deletion before the response was lost. Persist remote uncertainty
        // before attempting it, and leave the intent in `deleting` on error.
        record_remote_effect(None)?;
        git::delete_remote_branch_at_oid(repo, &intent.remote, &intent.branch, expected_oid)?;

        record_remote_effect(Some(RemoteEffect::BranchDeleted {
            remote: intent.remote.clone(),
            branch: intent.branch.clone(),
            prior_oid: Some(intent.expected_oid.clone()),
        }))?;
        config.pending_remote_branch_deletions.retain(|candidate| {
            candidate.remote != intent.remote || candidate.branch != intent.branch
        });
        config.save(repo.commondir())?;
    }

    Ok(deferred)
}

fn print_prune_deferrals(deferrals: &[String]) {
    for message in deferrals {
        println!("{} {}", style("Warning:").yellow(), message);
    }
}

fn has_pending_remote_entry_deletion(config: &Config, stack: &Stack) -> bool {
    config.pending_remote_branch_deletions.iter().any(|intent| {
        matches!(
            git::parse_entry_branch(&intent.branch),
            Some((ref user, ref name, ref entry_id))
                if user == &stack.username
                    && name == &stack.name
                    && git::normalize_gg_id(entry_id).is_some()
        )
    })
}

/// Prune remote branches of dropped entries, appending deferrals to `warnings`.
#[allow(clippy::too_many_arguments)]
fn prune_stale_remote_entry_branches_for_operation(
    repo: &Repository,
    provider: Option<&Provider>,
    stack: &Stack,
    config: &mut Config,
    segment: &mut SyncSegment,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let current_branches = stack
        .entries
        .iter()
        .filter_map(|entry| stack.entry_branch_name(entry))
        .collect();
    let deferred = reconcile_pending_remote_entry_deletions(
        repo,
        provider,
        &stack.username,
        &stack.name,
        &current_branches,
        config,
        |effect| {
            if let Some(effect) = effect {
                segment.record_remote_effect_durable(effect)
            } else {
                segment.mark_remote_before_attempt()
            }
        },
    )?;
    warnings.extend(deferred);
    Ok(())
}

/// Run the sync command
#[allow(clippy::too_many_arguments)]
pub fn run(
    draft: bool,
    json: bool,
    jsonl: bool,
    no_rebase_check: bool,
    rebase_if_needed: bool,
    force: bool,
    update_descriptions: bool,
    update_title: bool,
    run_lint: bool,
    until: Option<String>,
    no_verify: bool,
) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.commondir();
    git::signal_sync_test_before_lock()?;
    let _lock = git::acquire_operation_lock(&repo, "sync")?;
    if git::is_rebase_in_progress(&repo) {
        return Err(GgError::Other(
            "A rebase is in progress. Run `gg continue` to continue or `gg abort` to cancel."
                .to_string(),
        ));
    }
    git::wait_for_sync_test_release()?;
    let mut config = Config::load_with_global(git_dir)?;

    // Apply config defaults to CLI flags:
    // - draft: CLI flag OR config setting (either one enables drafts)
    // - update_descriptions: CLI flag OR config setting (either one enables updates;
    //   set sync_update_descriptions: false in config to opt out)
    let draft = draft || config.get_sync_draft();
    let update_descriptions = update_descriptions || config.get_sync_update_descriptions();
    let update_title = update_title || config.get_sync_update_title();

    // Load stack early to validate --until and decide whether sync can remain a
    // provider-free no-op.
    let initial_stack = Stack::load(&repo, &config)?;
    // Keep the caller's target identity from the stack they could see before
    // fetching. Do not interpret a position a second time on a shorter stack.
    let selected = until
        .as_deref()
        .map(|target| resolve_target(&initial_stack, target))
        .transpose()?
        .map(|pos| &initial_stack.entries[pos - 1]);
    let has_pending_deletion = has_pending_remote_entry_deletion(&config, &initial_stack);
    let provider = if initial_stack.is_empty() && !has_pending_deletion {
        Provider::detect(&repo).ok()
    } else {
        let provider = Provider::detect(&repo)?;
        provider.check_installed()?;
        provider.check_auth()?;
        Some(provider)
    };
    let mut segment = SyncSegment::new();
    if !initial_stack.is_empty() || has_pending_deletion {
        segment.ensure_started(&repo, &config)?;
    }
    let mut warnings: Vec<String> = initial_stack
        .prefix_mismatch(&config)
        .map(|mismatch| mismatch.warning_message())
        .into_iter()
        .collect();
    if !json && !jsonl {
        for warning in &warnings {
            println!("{} {}", style("Warning:").yellow(), warning);
        }
    }
    if initial_stack.is_empty() {
        let warnings_before_prune = warnings.len();
        if has_pending_deletion {
            if let Err(error) = prune_stale_remote_entry_branches_for_operation(
                &repo,
                provider.as_ref(),
                &initial_stack,
                &mut config,
                &mut segment,
                &mut warnings,
            ) {
                return segment.finish_result(&repo, &config, Err(error));
            }
        }
        segment.finish(&repo, &config)?;
        if !json && !jsonl {
            print_prune_deferrals(&warnings[warnings_before_prune..]);
        }
        if json {
            print_json(&SyncResponse {
                version: OUTPUT_VERSION,
                sync: SyncResultJson {
                    stack: initial_stack.name.clone(),
                    base: initial_stack.base.clone(),
                    rebased_before_sync: false,
                    warnings: warnings.clone(),
                    github_stack: None,
                    metadata: SyncMetadataJson::default(),
                    entries: vec![],
                },
            });
        } else if !jsonl {
            println!("{}", style("Stack is empty. Nothing to sync.").dim());
        }
        if jsonl {
            let mut streamer = StreamingJson::new();
            streamer.emit(&SyncStreamingResponse {
                version: OUTPUT_VERSION,
                command: "sync".to_string(),
                event: SyncStreamingEvent::Summary {
                    stack: initial_stack.name,
                    base: initial_stack.base,
                    rebased_before_sync: false,
                    warnings,
                    github_stack: None,
                    metadata: SyncMetadataJson::default(),
                    entries: vec![],
                },
            });
        }
        return Ok(());
    }

    let provider = provider.expect("non-empty sync has a provider");
    let conditional_base = if rebase_if_needed {
        segment.finish_on_error(&repo, &config, git::fetch_and_prune())?;
        let oid = segment.finish_on_error(
            &repo,
            &config,
            repo.refname_to_id(&format!("refs/remotes/origin/{}", initial_stack.base)),
        )?;
        segment.finish_on_error(
            &repo,
            &config,
            git::validate_sync_base(&repo, &initial_stack, oid),
        )?;
        Some(oid)
    } else {
        None
    };

    if conditional_base.is_none() {
        segment.finish_on_error(&repo, &config, git::fetch_and_prune())?;
    }
    let landed = segment
        .finish_on_error(
            &repo,
            &config,
            conditional_base
                .map(|base| after_land::landed_prefix(&repo, &initial_stack, &provider, base))
                .transpose(),
        )?
        .flatten();
    let skipped = landed.as_ref().map_or(0, |p| p.len);
    let mut rebased_before_sync = false;
    if let (Some(base), Some(prefix)) = (conditional_base, &landed) {
        if prefix.rewritten {
            segment.finish_on_error(
                &repo,
                &config,
                after_land::rebase_survivors(
                    &repo,
                    &initial_stack,
                    &provider,
                    base,
                    prefix.len,
                    || Ok(()),
                ),
            )?;
            rebased_before_sync = true;
        } else {
            segment.finish_on_error(
                &repo,
                &config,
                git::record_sync_base(&repo, &initial_stack, base),
            )?;
        }
    } else if !no_rebase_check {
        rebased_before_sync = segment.finish_on_error(
            &repo,
            &config,
            maybe_rebase_if_base_is_behind(
                &repo,
                &config,
                &initial_stack.base,
                json,
                jsonl,
                || match conditional_base {
                    Some(base) => after_land::rebase_survivors(
                        &repo,
                        &initial_stack,
                        &provider,
                        base,
                        0,
                        || Ok(()),
                    ),
                    None => {
                        crate::commands::rebase::run_with_repo(&repo, None, json || jsonl, false)
                    }
                },
            ),
        )?;
    }

    let streamer = if jsonl {
        Some(StreamingJson::new())
    } else {
        None
    };
    let mut streamer = streamer;

    let mut start_emitted = false;
    let mut lint_before_sync = |current_stack: &Stack, end_pos: usize| -> Result<()> {
        if !run_lint || end_pos == 0 {
            return Ok(());
        }
        let sync_start_branch = git::current_branch_name(&repo);
        let sync_start_head = repo.head()?.peel_to_commit()?.id();
        if !json && !jsonl {
            println!("{}", console::style("Running lint before sync...").dim());
        }
        // Emit Start before lint so streaming clients know the stack and
        // entry count up front instead of waiting for lint to finish.
        if let Some(s) = streamer.as_mut().filter(|_| !start_emitted) {
            s.emit(&SyncStreamingResponse {
                version: OUTPUT_VERSION,
                command: "sync".to_string(),
                event: SyncStreamingEvent::Start {
                    stack: current_stack.name.clone(),
                    base: current_stack.base.clone(),
                    total_entries: end_pos,
                },
            });
            start_emitted = true;
        }
        // If the client closes the stream mid-lint, stop emitting but let lint
        // finish so the checkout is restored (a broken pipe in `emit` would
        // exit the process while HEAD is detached on a linted commit).
        let stream_closed = std::rc::Rc::new(std::cell::Cell::new(false));
        let progress: Option<crate::commands::run::RunProgressFn> = if jsonl {
            let closed = stream_closed.clone();
            Some(Box::new(move |event| {
                if closed.get() {
                    return;
                }
                let result = StreamingJson::new().try_emit(&SyncStreamingResponse {
                    version: OUTPUT_VERSION,
                    command: "sync".to_string(),
                    event: lint_progress_event(event),
                });
                if result.is_err() {
                    closed.set(true);
                }
            }))
        } else {
            None
        };
        let lint_result =
            crate::commands::lint::run_for_sync(Some(end_pos), json || jsonl, progress)?;
        if !lint_result.all_passed {
            restore_sync_start_position(
                &repo,
                sync_start_branch.as_deref(),
                sync_start_head,
                json,
                jsonl,
            )?;
            if !stream_closed.get() {
                return Err(GgError::Other(lint_failure_message(&lint_result.results)));
            }
        }
        if stream_closed.get() {
            return Err(GgError::Other(
                "Output stream closed during lint; sync aborted.".to_string(),
            ));
        }
        if !json && !jsonl {
            println!();
        }
        Ok(())
    };

    let end_position =
        |stack: &Stack| sync_end_position(stack, selected, skipped, initial_stack.len());
    let (mut stack, mut end_pos) = segment.finish_on_error(
        &repo,
        &config,
        Stack::load(&repo, &config).and_then(|stack| {
            let end_pos = end_position(&stack)?;
            Ok((stack, end_pos))
        }),
    )?;
    if landed.is_some() && !rebased_before_sync && end_pos > 0 {
        segment.finish_on_error(
            &repo,
            &config,
            after_land::check_preserved_metadata(&repo, &stack),
        )?;
    }
    if let Err(error) = lint_before_sync(&stack, end_pos) {
        return segment.finish_on_error(&repo, &config, Err(error));
    }
    if run_lint {
        (stack, end_pos) = segment.finish_on_error(
            &repo,
            &config,
            Stack::load(&repo, &config).and_then(|stack| {
                let end_pos = end_position(&stack)?;
                Ok((stack, end_pos))
            }),
        )?;
    }
    let mut retargeted_pr = None;
    let mut retarget_started = false;
    if let (Some(base), Some(prefix)) = (conditional_base, &landed) {
        if !prefix.rewritten && end_pos > 0 {
            let first = &initial_stack.entries[skipped];
            let needs_rebase_result = after_land::needs_rebase(
                &repo,
                &stack,
                first,
                prefix.previous_target.as_deref(),
                &provider,
                || {
                    segment.mark_remote_before_attempt()?;
                    retarget_started = true;
                    retargeted_pr = first.mr_number;
                    Ok(())
                },
            );
            let needs_rebase = match needs_rebase_result {
                Ok(needs_rebase) => {
                    if retarget_started {
                        segment.finish(&repo, &config)?;
                    }
                    needs_rebase
                }
                Err(error) => return segment.finish_result(&repo, &config, Err(error)),
            };
            if needs_rebase || stack.entries[0].oid != first.oid {
                // Lint may have amended commits. Replay the current survivors,
                // then lint the exact commits that will be published.
                let replay =
                    after_land::rebase_survivors(&repo, &stack, &provider, base, 0, || {
                        segment.ensure_started(&repo, &config)
                    });
                segment.finish_on_error(&repo, &config, replay)?;
                rebased_before_sync = true;
                (stack, end_pos) = segment.finish_on_error(
                    &repo,
                    &config,
                    Stack::load(&repo, &config).and_then(|stack| {
                        let end_pos = end_position(&stack)?;
                        Ok((stack, end_pos))
                    }),
                )?;
                if let Err(error) = lint_before_sync(&stack, end_pos) {
                    return segment.finish_on_error(&repo, &config, Err(error));
                }
                if run_lint {
                    (stack, end_pos) = segment.finish_on_error(
                        &repo,
                        &config,
                        Stack::load(&repo, &config).and_then(|stack| {
                            let end_pos = end_position(&stack)?;
                            Ok((stack, end_pos))
                        }),
                    )?;
                }
            }
        }
    }

    if end_pos == 0 {
        let warnings_before_prune = warnings.len();
        if has_pending_remote_entry_deletion(&config, &stack) {
            segment.ensure_started(&repo, &config)?;
            if let Err(error) = prune_stale_remote_entry_branches_for_operation(
                &repo,
                Some(&provider),
                &stack,
                &mut config,
                &mut segment,
                &mut warnings,
            ) {
                return segment.finish_result(&repo, &config, Err(error));
            }
        }
        segment.finish(&repo, &config)?;
        if !json && !jsonl {
            print_prune_deferrals(&warnings[warnings_before_prune..]);
        }
        if json {
            print_json(&SyncResponse {
                version: OUTPUT_VERSION,
                sync: SyncResultJson {
                    stack: stack.name.clone(),
                    base: stack.base.clone(),
                    rebased_before_sync,
                    warnings: warnings.clone(),
                    github_stack: None,
                    metadata: SyncMetadataJson::default(),
                    entries: vec![],
                },
            });
        } else if !jsonl {
            println!("{}", style("Stack is empty. Nothing to sync.").dim());
        }
        if let Some(s) = streamer.as_mut() {
            s.emit(&SyncStreamingResponse {
                version: OUTPUT_VERSION,
                command: "sync".to_string(),
                event: SyncStreamingEvent::Summary {
                    stack: stack.name,
                    base: stack.base,
                    rebased_before_sync,
                    warnings: warnings.clone(),
                    github_stack: None,
                    metadata: SyncMetadataJson::default(),
                    entries: vec![],
                },
            });
        }
        return Ok(());
    }

    segment.ensure_started(&repo, &config)?;
    if !json && !jsonl {
        println!("{}", style("Normalizing GG metadata...").dim());
    }
    // Intentional: sync always enforces GG-ID / GG-Parent invariants for the
    // stack so branch/PR mappings stay stable, even if auto_add_gg_ids is false.
    let metadata_counts =
        segment.finish_on_error(&repo, &config, git::normalize_stack_metadata(&repo, &stack))?;
    stack = segment.finish_on_error(&repo, &config, Stack::load(&repo, &config))?;

    // Metadata normalization and lint amendments preserve entry order.
    let entries_to_sync = &stack.entries[..end_pos];

    // Resolve every mapped review before the main loop can mutate anything.
    // Later mutation authorization still performs its own fresh provider read.
    let mut classified_reviews = Vec::with_capacity(entries_to_sync.len());
    let mut classified_entry_is_closed = Vec::with_capacity(entries_to_sync.len());
    for (i, entry) in entries_to_sync.iter().enumerate() {
        let gg_id = entry.gg_id.as_ref().unwrap();
        let expected_head = stack.entry_branch_name(entry).unwrap();
        let expected_current_bases = expected_current_target_branches(
            &repo,
            &config,
            i,
            &stack.base,
            entries_to_sync,
            &stack,
            classified_reviews.last().and_then(Option::as_ref),
        );
        let desired_base = compute_target_branch(
            i,
            &stack.base,
            entries_to_sync,
            &classified_entry_is_closed,
            &stack,
        );
        let existing_pr = config.get_mr_for_entry(&stack.name, gg_id);
        let active_previous_target = (i == 0 && landed.as_ref().is_some_and(|p| p.rewritten))
            .then(|| landed.as_ref()?.previous_target.as_deref())
            .flatten();
        let classified = match existing_pr {
            Some(number) => Some(segment.finish_on_error(
                &repo,
                &config,
                classify_review_for_sync(
                    &provider,
                    number,
                    ReviewSyncTopology {
                        repo: &repo,
                        config: &config,
                        expected_head: &expected_head,
                        expected_current_bases: &expected_current_bases,
                        desired_base: &desired_base,
                        active_previous_base: active_previous_target,
                    },
                ),
            )?),
            None => None,
        };
        classified_entry_is_closed.push(classified.as_ref().is_some_and(|info| {
            matches!(
                info.state,
                crate::provider::PrState::Merged | crate::provider::PrState::Closed
            )
        }));
        classified_reviews.push(classified);
    }

    // Load optional PR template
    let pr_template = template::load_template(git_dir);

    // Sync progress
    let pb = if json || jsonl {
        ProgressBar::hidden()
    } else if atty::is(atty::Stream::Stderr) {
        sync_progress_bar(entries_to_sync.len() as u64, ProgressDrawTarget::stderr())
    } else {
        ProgressBar::hidden()
    };

    // Process each entry
    // If a commit title starts with "WIP:" or "Draft:" (case-insensitive),
    // that PR and all subsequent PRs should be drafts.
    let mut force_draft = draft;
    let mut json_entries: Vec<SyncEntryResultJson> = Vec::new();
    let mut nav_snapshots: Vec<Option<NavEntrySnapshot>> = Vec::new();
    let mut branch_push_failed = false;
    // Track which entries are closed/merged so downstream entries can skip them
    // when computing their target branch (walk-back algorithm for stacked MRs).
    let mut entry_is_closed: Vec<bool> = Vec::with_capacity(entries_to_sync.len());
    let mut entry_has_unresolved_pr_state: Vec<bool> = Vec::with_capacity(entries_to_sync.len());

    if !start_emitted {
        if let Some(s) = streamer.as_mut() {
            s.emit(&SyncStreamingResponse {
                version: OUTPUT_VERSION,
                command: "sync".to_string(),
                event: SyncStreamingEvent::Start {
                    stack: stack.name.clone(),
                    base: stack.base.clone(),
                    total_entries: entries_to_sync.len(),
                },
            });
        }
    }

    for (i, entry) in entries_to_sync.iter().enumerate() {
        let gg_id = entry.gg_id.as_ref().unwrap();
        let entry_branch = stack.entry_branch_name(entry).unwrap();
        let commit = segment.finish_on_error(&repo, &config, repo.find_commit(entry.oid))?;
        let raw_title = strip_gg_id_from_message(&entry.title);

        if let Some(s) = streamer.as_mut() {
            s.emit(&SyncStreamingResponse {
                version: OUTPUT_VERSION,
                command: "sync".to_string(),
                event: SyncStreamingEvent::EntryStarted {
                    position: entry.position,
                    sha: entry.short_sha.clone(),
                    title: entry.title.clone(),
                    gg_id: gg_id.clone(),
                    branch: entry_branch.clone(),
                },
            });
        }

        if !force_draft && is_wip_or_draft_prefix(&raw_title) {
            force_draft = true;
        }
        let entry_draft = force_draft;

        let title = clean_title(&raw_title);

        let mut action = "up_to_date".to_string();
        let mut pr_number: Option<u64> = None;
        let mut pr_url: Option<String> = None;
        let mut pushed = false;
        let mut entry_error: Option<String> = None;
        let mut pr_state_cached: Option<crate::stack_nav::PrEntryState> = None;
        let mut effective_draft = entry_draft;
        let mut is_entry_closed = false;
        let mut pr_state_unresolved = false;

        let (title, description) = build_pr_payload(
            &title,
            get_commit_description(&commit),
            &stack.name,
            &entry.short_sha,
            pr_template.as_deref(),
        );

        pb.set_message(format!("Processing {}...", entry.short_sha));

        // Classify a mapped review before creating or pushing its branch. A
        // terminal review is safe to skip only when it still names this exact
        // stack entry and target.
        let target_branch =
            compute_target_branch(i, &stack.base, entries_to_sync, &entry_is_closed, &stack);
        let existing_pr = config.get_mr_for_entry(&stack.name, gg_id);
        let classified_pr_info = classified_reviews[i].clone();
        let review_is_terminal = classified_pr_info.as_ref().is_some_and(|info| {
            matches!(
                info.state,
                crate::provider::PrState::Merged | crate::provider::PrState::Closed
            )
        });

        // Check if remote branch exists and has the same OID as local
        let remote_oid = git::get_remote_branch_oid(&repo, &entry_branch);
        let needs_push = remote_oid != Some(entry.oid);
        if !review_is_terminal && (needs_push || !rebase_if_needed) {
            segment.finish_on_error(
                &repo,
                &config,
                create_entry_branch(&repo, &stack, entry, &entry_branch),
            )?;
        }

        // Only push if the remote is different or doesn't exist
        if needs_push && !review_is_terminal {
            if let Some(s) = streamer.as_mut() {
                s.emit(&SyncStreamingResponse {
                    version: OUTPUT_VERSION,
                    command: "sync".to_string(),
                    event: SyncStreamingEvent::PushStarted {
                        position: entry.position,
                        branch: entry_branch.clone(),
                    },
                });
            }
            pushed = true;
            // Push the branch (always force-push with lease because rebases change commit SHAs)
            // This is safe because each entry branch is owned by this stack
            // If --force is passed, use hard force as an escape hatch
            let mut before_push_attempt_failed = false;
            let push_result = git::push_branch_with_confirmation(
                &entry_branch,
                true,
                force,
                no_verify,
                |branch_name| suspend_sync_progress(&pb, || git::confirm_force_push(branch_name)),
                || {
                    let result = (|| {
                        if let Some(info) = classified_pr_info.as_ref() {
                            let expected_head = info
                                .head_branch
                                .as_deref()
                                .expect("classified active review has a source");
                            let expected_base = info
                                .base_branch
                                .as_deref()
                                .expect("classified active review has a target");
                            authorize_review_mutation(
                                &provider,
                                info.number,
                                expected_head,
                                expected_base,
                            )?;
                        }
                        segment.mark_remote_before_attempt()
                    })();
                    before_push_attempt_failed |= result.is_err();
                    result
                },
            );
            if let Err(e) = push_result {
                if before_push_attempt_failed {
                    return segment.finish_result(&repo, &config, Err(e));
                }
                pb.finish_and_clear();
                branch_push_failed = true;
                if json || jsonl {
                    if let Some(s) = streamer.as_mut() {
                        s.emit(&SyncStreamingResponse {
                            version: OUTPUT_VERSION,
                            command: "sync".to_string(),
                            event: SyncStreamingEvent::PushError {
                                position: entry.position,
                                branch: entry_branch.clone(),
                                error: e.to_string(),
                            },
                        });
                    }
                    action = "error".to_string();
                    entry_error = Some(e.to_string());

                    json_entries.push(SyncEntryResultJson {
                        position: entry.position,
                        sha: entry.short_sha.clone(),
                        title: entry.title.clone(),
                        gg_id: gg_id.clone(),
                        branch: entry_branch,
                        action,
                        pr_number,
                        pr_url,
                        draft: entry_draft,
                        pushed,
                        error: entry_error,
                        nav_comment_action: None,
                    });
                    nav_snapshots.push(None);
                    entry_is_closed.push(false);
                    entry_has_unresolved_pr_state.push(true);
                    continue;
                }

                format_push_error(&e, &entry_branch);
                return segment.finish_result(&repo, &config, Err(e));
            }

            if let Some(s) = streamer.as_mut() {
                s.emit(&SyncStreamingResponse {
                    version: OUTPUT_VERSION,
                    command: "sync".to_string(),
                    event: SyncStreamingEvent::PushDone {
                        position: entry.position,
                        branch: entry_branch.clone(),
                        forced: force,
                    },
                });
            }

            // Record the push as a remote effect. `sync` always pushes with
            // force-with-lease because rebases rewrite entry-branch history;
            // the `force` field here reflects the hard --force escape hatch.
            let effect = RemoteEffect::Pushed {
                remote: "origin".to_string(),
                branch: entry_branch.clone(),
                force,
            };
            segment.record_remote_effect(effect);
        }

        // Create or update PR
        match existing_pr {
            Some(pr_num) => {
                pr_number = Some(pr_num);
                let pr_info = classified_pr_info;
                pr_url = pr_info.as_ref().map(|info| info.url.clone());
                // Cache the state so the nav reconcile pass can reuse it without
                // a second network round-trip.
                pr_state_cached = pr_info.as_ref().map(|info| match info.state {
                    crate::provider::PrState::Open => crate::stack_nav::PrEntryState::Open,
                    crate::provider::PrState::Draft => crate::stack_nav::PrEntryState::Draft,
                    crate::provider::PrState::Merged => crate::stack_nav::PrEntryState::Merged,
                    crate::provider::PrState::Closed => crate::stack_nav::PrEntryState::Closed,
                });
                let is_closed = pr_info
                    .as_ref()
                    .map(|info| {
                        matches!(
                            info.state,
                            crate::provider::PrState::Merged | crate::provider::PrState::Closed
                        )
                    })
                    .unwrap_or(false);
                is_entry_closed = is_closed;

                if is_closed {
                    action = "skipped_closed".to_string();
                    if let Some(s) = streamer.as_mut() {
                        s.emit(&SyncStreamingResponse {
                            version: OUTPUT_VERSION,
                            command: "sync".to_string(),
                            event: SyncStreamingEvent::PrSkippedClosed {
                                position: entry.position,
                                pr_number: pr_num,
                            },
                        });
                    }
                    // Skip updating closed/merged PRs
                    if !json && !jsonl {
                        pb.println(format!(
                            "{} {} {}{} already closed/merged, skipping",
                            style("○").dim(),
                            provider.pr_label(),
                            provider.pr_number_prefix(),
                            pr_num
                        ));
                    }
                } else if let Some(old_head_branch) =
                    mismatched_pr_head_branch(pr_info.as_ref(), &entry_branch)
                {
                    let old_base_branch = pr_info
                        .as_ref()
                        .and_then(|info| info.base_branch.as_deref())
                        .expect("classified active review has a target");
                    let replacement_draft = pr_info
                        .as_ref()
                        .map(|info| info.draft)
                        .unwrap_or(entry_draft);
                    effective_draft = replacement_draft;
                    let replacement_description = managed_body::wrap(
                        &description_with_replacement_note(&description, &provider, pr_num),
                    );

                    segment.finish_on_error(
                        &repo,
                        &config,
                        authorize_review_mutation(
                            &provider,
                            pr_num,
                            old_head_branch,
                            old_base_branch,
                        ),
                    )?;
                    match provider.create_pr(
                        &entry_branch,
                        &target_branch,
                        &title,
                        &replacement_description,
                        replacement_draft,
                    ) {
                        Ok(result) => {
                            config.set_mr_for_entry(&stack.name, gg_id, result.number);
                            let save_result = config.save(repo.commondir());
                            segment.finish_on_error(&repo, &config, save_result)?;
                            if let Some(s) = streamer.as_mut() {
                                s.emit(&SyncStreamingResponse {
                                    version: OUTPUT_VERSION,
                                    command: "sync".to_string(),
                                    event: SyncStreamingEvent::PrCreated {
                                        position: entry.position,
                                        pr_number: result.number,
                                        pr_url: if result.url.is_empty() {
                                            None
                                        } else {
                                            Some(result.url.clone())
                                        },
                                        draft: replacement_draft,
                                    },
                                });
                            }
                            pr_number = Some(result.number);
                            pr_url = if result.url.is_empty() {
                                None
                            } else {
                                Some(result.url.clone())
                            };
                            pr_state_cached = Some(if replacement_draft {
                                stack_nav::PrEntryState::Draft
                            } else {
                                stack_nav::PrEntryState::Open
                            });
                            action = "recreated".to_string();

                            let created_effect = RemoteEffect::PrCreated {
                                number: result.number,
                                url: result.url.clone(),
                            };
                            segment.record_remote_effect(created_effect);

                            let close_comment = replacement_closing_comment(
                                &provider,
                                old_head_branch,
                                &entry_branch,
                                result.number,
                                &result.url,
                            );
                            segment.finish_on_error(
                                &repo,
                                &config,
                                authorize_review_mutation(
                                    &provider,
                                    pr_num,
                                    old_head_branch,
                                    old_base_branch,
                                ),
                            )?;
                            if let Err(e) = provider.create_pr_comment(pr_num, &close_comment) {
                                if !json && !jsonl {
                                    pb.println(format!(
                                        "{} Could not comment on old {} {}{}: {}",
                                        style("Warning:").yellow(),
                                        provider.pr_label(),
                                        provider.pr_number_prefix(),
                                        pr_num,
                                        e
                                    ));
                                }
                                if entry_error.is_none() {
                                    entry_error = Some(format!(
                                        "Recreated, but could not comment on old {}: {e}",
                                        provider.pr_label()
                                    ));
                                }
                            } else {
                                segment.mark_remote();
                            }

                            segment.finish_on_error(
                                &repo,
                                &config,
                                authorize_review_mutation(
                                    &provider,
                                    pr_num,
                                    old_head_branch,
                                    old_base_branch,
                                ),
                            )?;
                            match provider.close_pr(pr_num) {
                                Ok(()) => {
                                    let closed_effect = RemoteEffect::PrClosed {
                                        number: pr_num,
                                        url: pr_info
                                            .as_ref()
                                            .map(|info| info.url.clone())
                                            .unwrap_or_default(),
                                    };
                                    segment.record_remote_effect(closed_effect);
                                }
                                Err(e) => {
                                    if !json && !jsonl {
                                        pb.println(format!(
                                            "{} Could not close old {} {}{}: {}",
                                            style("Warning:").yellow(),
                                            provider.pr_label(),
                                            provider.pr_number_prefix(),
                                            pr_num,
                                            e
                                        ));
                                    }
                                    if entry_error.is_none() {
                                        entry_error = Some(format!(
                                            "Recreated, but could not close old {}: {e}",
                                            provider.pr_label()
                                        ));
                                    }
                                }
                            }

                            if !json && !jsonl {
                                let status_msg = if needs_push { "Pushed" } else { "Up to date" };
                                pb.println(format!(
                                    "{} {} {} -> {} {}{} (recreated from {}{})",
                                    style("OK").green().bold(),
                                    status_msg,
                                    style(&entry_branch).cyan(),
                                    provider.pr_label(),
                                    provider.pr_number_prefix(),
                                    result.number,
                                    provider.pr_number_prefix(),
                                    pr_num
                                ));
                                if !result.url.is_empty() {
                                    pb.println(format!(
                                        "   {}",
                                        style(&result.url).underlined().blue()
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            // The old PR still exists, but its head branch no
                            // longer represents this local entry. If creating
                            // the replacement fails, skip native GitHub stack
                            // reconciliation instead of linking the stale PR
                            // into the native stack.
                            pr_state_unresolved = true;
                            action = "error".to_string();
                            entry_error = Some(e.to_string());
                            if !json && !jsonl {
                                pb.println(format!(
                                    "{} Failed to recreate {} for {} after source branch changed from {}: {}",
                                    style("Error:").red().bold(),
                                    provider.pr_label(),
                                    entry_branch,
                                    old_head_branch,
                                    e
                                ));
                            }
                        }
                    }
                } else {
                    let mut review_changed = retargeted_pr == Some(pr_num);
                    let current_base = pr_info
                        .as_ref()
                        .and_then(|info| info.base_branch.as_deref())
                        .expect("classified active review has a target");
                    if update_title
                        && (!rebase_if_needed
                            || pr_info.as_ref().is_none_or(|info| info.title != title))
                    {
                        segment.finish_on_error(
                            &repo,
                            &config,
                            authorize_review_mutation(
                                &provider,
                                pr_num,
                                &entry_branch,
                                current_base,
                            ),
                        )?;
                        let checkpoint = segment.mark_remote_before_attempt();
                        segment.finish_on_error(&repo, &config, checkpoint)?;
                        if let Err(e) = provider.update_pr_title(pr_num, &title) {
                            if !json && !jsonl {
                                pb.println(format!(
                                    "{} Could not update {} {}{} title: {}",
                                    style("Warning:").yellow(),
                                    provider.pr_label(),
                                    provider.pr_number_prefix(),
                                    pr_num,
                                    e
                                ));
                            }
                            if entry_error.is_none() {
                                entry_error = Some(format!("Could not update title: {e}"));
                            }
                        } else {
                            review_changed = true;
                        }
                    }

                    if update_descriptions {
                        // Existing PRs keep their current draft/ready state.
                        // --draft only applies when creating NEW PRs/MRs.

                        // Fetch current remote body and merge only the managed block,
                        // preserving user edits outside the markers.
                        match provider.get_pr_body(pr_num) {
                            Ok(remote_body) => {
                                let merged =
                                    managed_body::replace_managed(&remote_body, &description);
                                let new_body = match merged {
                                    Some(body) => body,
                                    None => {
                                        // Legacy PR without managed markers — skip body update
                                        let skip_msg = format!(
                                            "{} {}{} has no managed markers, skipping body update",
                                            provider.pr_label(),
                                            provider.pr_number_prefix(),
                                            pr_num
                                        );
                                        if !json && !jsonl {
                                            pb.println(format!(
                                                "{} {}",
                                                style("Warning:").yellow(),
                                                skip_msg
                                            ));
                                        }
                                        if entry_error.is_none() {
                                            entry_error = Some(skip_msg);
                                        }
                                        String::new()
                                    }
                                };
                                if !new_body.is_empty()
                                    && (!rebase_if_needed || new_body != remote_body)
                                {
                                    segment.finish_on_error(
                                        &repo,
                                        &config,
                                        authorize_review_mutation(
                                            &provider,
                                            pr_num,
                                            &entry_branch,
                                            current_base,
                                        ),
                                    )?;
                                    let checkpoint = segment.mark_remote_before_attempt();
                                    segment.finish_on_error(&repo, &config, checkpoint)?;
                                    if let Err(e) =
                                        provider.update_pr_description(pr_num, &new_body)
                                    {
                                        if !json && !jsonl {
                                            pb.println(format!(
                                                "{} Could not update {} {}{} description: {}",
                                                style("Warning:").yellow(),
                                                provider.pr_label(),
                                                provider.pr_number_prefix(),
                                                pr_num,
                                                e
                                            ));
                                        }
                                        if entry_error.is_none() {
                                            entry_error =
                                                Some(format!("Could not update description: {e}"));
                                        }
                                    } else {
                                        review_changed = true;
                                    }
                                }
                            }
                            Err(e) => {
                                // Could not read remote body — skip update to be safe
                                if !json && !jsonl {
                                    pb.println(format!(
                                        "{} Could not read {} {}{} body, skipping description update: {}",
                                        style("Warning:").yellow(),
                                        provider.pr_label(),
                                        provider.pr_number_prefix(),
                                        pr_num,
                                        e
                                    ));
                                }
                                if entry_error.is_none() {
                                    entry_error = Some(format!("Could not read remote body: {e}"));
                                }
                            }
                        }
                    }

                    let base_changed = pr_info
                        .as_ref()
                        .and_then(|info| info.base_branch.as_deref())
                        != Some(target_branch.as_str());
                    if !rebase_if_needed || base_changed {
                        segment.finish_on_error(
                            &repo,
                            &config,
                            authorize_review_mutation(
                                &provider,
                                pr_num,
                                &entry_branch,
                                current_base,
                            ),
                        )?;
                        let checkpoint = segment.mark_remote_before_attempt();
                        segment.finish_on_error(&repo, &config, checkpoint)?;
                        match provider.update_pr_base(pr_num, &target_branch) {
                            Ok(()) => {
                                review_changed |= base_changed;
                            }
                            Err(e) => {
                                // Native GitHub stack reconciliation must reflect
                                // the ordinary PR base chain. If the provider
                                // rejects that base update, skip the later native
                                // stack mutation rather than linking a topology
                                // `gg sync` did not actually establish.
                                pr_state_unresolved = true;
                                if !json && !jsonl {
                                    pb.println(format!(
                                        "{} Could not update {} {}{}: {}",
                                        style("Warning:").yellow(),
                                        provider.pr_label(),
                                        provider.pr_number_prefix(),
                                        pr_num,
                                        e
                                    ));
                                }
                                if entry_error.is_none() {
                                    entry_error = Some(format!("Could not update base: {e}"));
                                }
                            }
                        }
                    }

                    // Show appropriate message based on whether we pushed
                    let status_msg = if needs_push {
                        "Force-pushed"
                    } else {
                        "Up to date"
                    };
                    if !json && !jsonl {
                        pb.println(format!(
                            "{} {} {} -> {} {}{}",
                            style("OK").green().bold(),
                            status_msg,
                            style(&entry_branch).cyan(),
                            provider.pr_label(),
                            provider.pr_number_prefix(),
                            pr_num
                        ));
                    }
                    if needs_push
                        || review_changed
                        || (!rebase_if_needed && (update_descriptions || update_title))
                    {
                        action = "updated".to_string();
                    }
                    if let Some(s) = streamer.as_mut() {
                        s.emit(&SyncStreamingResponse {
                            version: OUTPUT_VERSION,
                            command: "sync".to_string(),
                            event: SyncStreamingEvent::PrUpdated {
                                position: entry.position,
                                pr_number: pr_num,
                                action: action.clone(),
                            },
                        });
                    }
                }
            }
            None => {
                // Create new PR/MR — wrap description in managed markers
                let wrapped_description = managed_body::wrap(&description);
                match provider.create_pr(
                    &entry_branch,
                    &target_branch,
                    &title,
                    &wrapped_description,
                    entry_draft,
                ) {
                    Ok(result) => {
                        config.set_mr_for_entry(&stack.name, gg_id, result.number);
                        let save_result = config.save(repo.commondir());
                        segment.finish_on_error(&repo, &config, save_result)?;
                        if let Some(s) = streamer.as_mut() {
                            s.emit(&SyncStreamingResponse {
                                version: OUTPUT_VERSION,
                                command: "sync".to_string(),
                                event: SyncStreamingEvent::PrCreated {
                                    position: entry.position,
                                    pr_number: result.number,
                                    pr_url: if result.url.is_empty() {
                                        None
                                    } else {
                                        Some(result.url.clone())
                                    },
                                    draft: entry_draft,
                                },
                            });
                        }
                        pr_number = Some(result.number);
                        pr_url = if result.url.is_empty() {
                            None
                        } else {
                            Some(result.url.clone())
                        };
                        action = "created".to_string();

                        // Record the PR creation as a remote effect so `gg undo`
                        // can surface a provider-specific revert hint. Persist
                        // immediately so a mid-sequence failure still leaves an
                        // accurate record on disk.
                        let effect = RemoteEffect::PrCreated {
                            number: result.number,
                            url: result.url.clone(),
                        };
                        segment.record_remote_effect(effect);

                        if !json && !jsonl {
                            let draft_label = if entry_draft { " (draft)" } else { "" };
                            let status_msg = if needs_push { "Pushed" } else { "Up to date" };
                            pb.println(format!(
                                "{} {} {} -> {} {}{}{}",
                                style("OK").green().bold(),
                                status_msg,
                                style(&entry_branch).cyan(),
                                provider.pr_label(),
                                provider.pr_number_prefix(),
                                result.number,
                                draft_label
                            ));
                            // Show clickable URL for new PRs/MRs
                            if !result.url.is_empty() {
                                pb.println(format!(
                                    "   {}",
                                    style(&result.url).underlined().blue()
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        action = "error".to_string();
                        entry_error = Some(e.to_string());
                        if !json && !jsonl {
                            pb.println(format!(
                                "{} Failed to create {} for {}: {}",
                                style("Error:").red().bold(),
                                provider.pr_label(),
                                entry_branch,
                                e
                            ));
                        }
                    }
                }
            }
        }

        if json || jsonl {
            json_entries.push(SyncEntryResultJson {
                position: entry.position,
                sha: entry.short_sha.clone(),
                title: entry.title.clone(),
                gg_id: gg_id.clone(),
                branch: entry_branch,
                action,
                pr_number,
                pr_url,
                draft: effective_draft,
                pushed,
                error: entry_error,
                nav_comment_action: None,
            });
        }

        // Capture state for nav reconcile pass after the main loop.
        let nav_snapshot: Option<NavEntrySnapshot> = if let Some(num) = pr_number {
            // Reuse state cached during the main loop if available (existing PRs);
            // fall back to a fresh fetch only for newly created PRs.
            let state = if let Some(cached) = pr_state_cached {
                cached
            } else {
                match provider.get_pr_info(num).map(|info| info.state).ok() {
                    Some(crate::provider::PrState::Open) => stack_nav::PrEntryState::Open,
                    Some(crate::provider::PrState::Draft) => stack_nav::PrEntryState::Draft,
                    Some(crate::provider::PrState::Merged) => stack_nav::PrEntryState::Merged,
                    Some(crate::provider::PrState::Closed) => stack_nav::PrEntryState::Closed,
                    // Default to Open when the API is unreachable — we know the
                    // PR exists (just created or from config). Treating it as Open
                    // means the reconcile pass will attempt to upsert/delete the
                    // nav comment; if the API is truly down, that will fail with a
                    // non-fatal warning rather than silently dropping the entry
                    // from all nav comments.
                    None => stack_nav::PrEntryState::Open,
                }
            };
            // json_entries.len() - 1 = the index of the entry we just pushed (JSON mode).
            // In non-JSON mode, json_entries is empty — use a sentinel index.
            let json_index = if json || jsonl {
                json_entries.len().saturating_sub(1)
            } else {
                0
            };
            Some(NavEntrySnapshot {
                pr_number: num,
                pr_state: state,
                json_index,
            })
        } else {
            None
        };
        nav_snapshots.push(nav_snapshot);
        entry_is_closed.push(is_entry_closed);
        entry_has_unresolved_pr_state.push(pr_state_unresolved);

        pb.inc(1);
    }

    let warnings_before_prune = warnings.len();
    let prune_result = if branch_push_failed {
        Ok(())
    } else {
        prune_stale_remote_entry_branches_for_operation(
            &repo,
            Some(&provider),
            &stack,
            &mut config,
            &mut segment,
            &mut warnings,
        )
    };
    if let Err(error) = prune_result {
        pb.finish_and_clear();
        return segment.finish_result(&repo, &config, Err(error));
    }

    if !json && !jsonl {
        pb.finish_with_message("Done!");
        print_prune_deferrals(&warnings[warnings_before_prune..]);
    }

    let active_review_specs: Vec<(u64, String, String)> = entries_to_sync
        .iter()
        .enumerate()
        .zip(&nav_snapshots)
        .zip(&entry_is_closed)
        .filter_map(|(((i, entry), snapshot), is_closed)| {
            if *is_closed {
                None
            } else {
                snapshot.as_ref().map(|value| {
                    (
                        value.pr_number,
                        stack
                            .entry_branch_name(entry)
                            .expect("synced entries have GG IDs"),
                        compute_target_branch(
                            i,
                            &stack.base,
                            entries_to_sync,
                            &entry_is_closed,
                            &stack,
                        ),
                    )
                })
            }
        })
        .collect();
    let active_pr_numbers: Vec<u64> = active_review_specs
        .iter()
        .map(|(number, _, _)| *number)
        .collect();
    let has_unresolved_active_pr = nav_snapshots
        .iter()
        .zip(&entry_is_closed)
        .zip(&entry_has_unresolved_pr_state)
        .any(|((snapshot, is_closed), unresolved)| {
            !*is_closed && (snapshot.is_none() || *unresolved)
        });

    let github_stack_result = match provider {
        Provider::GitHub => {
            let mode = config.get_github_stacks_integration();
            let result = if let Some(result) = github_stack_preflight_skip(
                mode,
                until.as_deref(),
                &active_pr_numbers,
                has_unresolved_active_pr,
            ) {
                result
            } else {
                let outcome = reconcile_with_gh_before_mutation(
                    mode,
                    &stack.base,
                    &active_pr_numbers,
                    || {
                        for (number, expected_head, expected_base) in &active_review_specs {
                            authorize_review_mutation(
                                &provider,
                                *number,
                                expected_head,
                                expected_base,
                            )?;
                        }
                        segment.mark_remote_before_attempt()
                    },
                );
                match outcome {
                    Ok(outcome) => outcome.result,
                    Err(error) => return segment.finish_result(&repo, &config, Err(error)),
                }
            };

            let warning_message = result.is_warning().then(|| {
                result.message.clone().unwrap_or_else(|| {
                    let reason = match result.reason {
                        Some(GithubStackReason::Disabled) => "integration disabled",
                        Some(GithubStackReason::PartialSync) => "partial sync",
                        Some(GithubStackReason::InsufficientPrs) => {
                            "insufficient active pull requests"
                        }
                        Some(GithubStackReason::UnresolvedPrs) => "unresolved active pull requests",
                        Some(GithubStackReason::MissingExtension) => "missing gh stack extension",
                        Some(GithubStackReason::OutdatedExtension) => "outdated gh stack extension",
                        Some(GithubStackReason::UnsupportedRepository) => {
                            "repository does not support GitHub Stacked PRs"
                        }
                        Some(GithubStackReason::Diverged) => "native stack diverged",
                        Some(GithubStackReason::BackendFailed) => "gh stack backend failed",
                        None => "unknown reason",
                    };
                    format!("GitHub Stacked PR reconciliation warning: {reason}")
                })
            });
            if let Some(message) = &warning_message {
                warnings.push(message.clone());
            }

            if !json && !jsonl {
                match result.action {
                    GithubStackAction::Created => match result.stack_number {
                        Some(number) => println!(
                            "{} Created GitHub stack #{}",
                            style("OK").green().bold(),
                            number
                        ),
                        None => println!("{} Created GitHub stack", style("OK").green().bold()),
                    },
                    GithubStackAction::Appended => match result.stack_number {
                        Some(number) => println!(
                            "{} Updated GitHub stack #{}",
                            style("OK").green().bold(),
                            number
                        ),
                        None => println!("{} Updated GitHub stack", style("OK").green().bold()),
                    },
                    GithubStackAction::Unchanged => match result.stack_number {
                        Some(number) => println!(
                            "{}",
                            style(format!("GitHub stack #{} is already up to date", number)).dim()
                        ),
                        None => println!("{}", style("GitHub stack is already up to date").dim()),
                    },
                    GithubStackAction::Warning => println!(
                        "{} {}",
                        style("Warning:").yellow(),
                        warning_message
                            .as_deref()
                            .expect("warning actions have a warning message")
                    ),
                    GithubStackAction::Skipped => {
                        if result.reason == Some(GithubStackReason::UnresolvedPrs) {
                            println!(
                                "{}",
                                style(
                                    "GitHub stack reconciliation skipped because some active pull requests could not be resolved"
                                )
                                .dim()
                            );
                        }
                    }
                }
            }

            if let Some(s) = streamer.as_mut() {
                s.emit(&SyncStreamingResponse {
                    version: OUTPUT_VERSION,
                    command: "sync".to_string(),
                    event: SyncStreamingEvent::GithubStack {
                        result: result.clone(),
                    },
                });
            }

            Some(result)
        }
        Provider::GitLab => None,
    };

    // --- Nav-comment reconcile pass ---
    //
    // Skipped under --until to avoid inconsistent nav comments: a partial sync
    // cannot vouch for all PRs in the stack, and the single-entry skip rule
    // would misfire for partial subsets. Full `gg sync` (no --until) will
    // reconcile navigation across the whole stack.
    // Skip nav reconcile if any entry failed during the sync — a partial set of
    // PR numbers would produce truncated stack navigation on every other PR in
    // the stack. The next full successful sync will reconcile.
    if until.is_none() && nav_snapshots.iter().all(|s| s.is_some()) {
        // For each synced entry whose PR exists and is reachable, decide whether
        // to create/update/delete the managed nav comment based on:
        //   - the stack_nav_comments setting
        //   - the total stack size
        //   - the PR's state (open/draft vs merged/closed)
        //
        // We render the nav body with `is_current = true` on the entry being
        // processed, so each PR's comment highlights the reader's location.
        let setting_enabled = config.get_stack_nav_comments();
        let stack_entry_count = entries_to_sync.len();
        let number_prefix = provider.pr_number_prefix();

        // Collect the (pr_number, index) pairs once — used to render each
        // per-PR body with a different `is_current` flag.
        let all_entries: Vec<(u64, usize)> = nav_snapshots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|snap| (snap.pr_number, i)))
            .collect();

        for (i, snap) in nav_snapshots.iter().enumerate() {
            let snap = match snap {
                Some(s) => s,
                None => continue,
            };

            // Check for an existing managed comment once per PR.
            let existing = match provider.find_managed_comment(snap.pr_number) {
                Ok(v) => v,
                Err(e) => {
                    if !json && !jsonl {
                        println!(
                            "{} Could not list comments on {} {}{}: {}",
                            style("Warning:").yellow(),
                            provider.pr_label(),
                            number_prefix,
                            snap.pr_number,
                            e
                        );
                    }
                    if json || jsonl {
                        if let Some(entry_json) = json_entries.get_mut(snap.json_index) {
                            entry_json.nav_comment_action = Some("error".to_string());
                        }
                    }
                    continue;
                }
            };

            let decision = stack_nav::decide_action(stack_nav::NavDecisionInput {
                setting_enabled,
                stack_entry_count,
                pr_state: snap.pr_state,
                has_existing_comment: existing.is_some(),
            });
            let expected_head = stack
                .entry_branch_name(&entries_to_sync[i])
                .expect("synced entries have GG IDs");
            let expected_base =
                compute_target_branch(i, &stack.base, entries_to_sync, &entry_is_closed, &stack);

            let action_result: Option<&str> = match decision {
                stack_nav::NavAction::Skip => None,
                stack_nav::NavAction::Upsert => {
                    // Render body with `is_current = true` for this entry's position.
                    let nav_entries: Vec<stack_nav::StackNavEntry> = all_entries
                        .iter()
                        .map(|(n, j)| stack_nav::StackNavEntry {
                            pr_number: *n,
                            is_current: *j == i,
                        })
                        .collect();
                    let body = stack_nav::render(&stack.name, &nav_entries, number_prefix);

                    let result = match existing {
                        Some(ref c) if c.body == body => Some("unchanged"),
                        Some(ref c) => {
                            segment.finish_on_error(
                                &repo,
                                &config,
                                authorize_review_mutation(
                                    &provider,
                                    snap.pr_number,
                                    &expected_head,
                                    &expected_base,
                                ),
                            )?;
                            let checkpoint = segment.mark_remote_before_attempt();
                            segment.finish_on_error(&repo, &config, checkpoint)?;
                            match provider.update_pr_comment(snap.pr_number, c.id, &body) {
                                Ok(()) => Some("updated"),
                                Err(e) => {
                                    if !json && !jsonl {
                                        println!(
                                            "{} Could not update nav comment on {} {}{}: {}",
                                            style("Warning:").yellow(),
                                            provider.pr_label(),
                                            number_prefix,
                                            snap.pr_number,
                                            e
                                        );
                                    }
                                    Some("error")
                                }
                            }
                        }
                        None => {
                            segment.finish_on_error(
                                &repo,
                                &config,
                                authorize_review_mutation(
                                    &provider,
                                    snap.pr_number,
                                    &expected_head,
                                    &expected_base,
                                ),
                            )?;
                            let checkpoint = segment.mark_remote_before_attempt();
                            segment.finish_on_error(&repo, &config, checkpoint)?;
                            match provider.create_pr_comment(snap.pr_number, &body) {
                                Ok(()) => Some("created"),
                                Err(e) => {
                                    if !json && !jsonl {
                                        println!(
                                            "{} Could not create nav comment on {} {}{}: {}",
                                            style("Warning:").yellow(),
                                            provider.pr_label(),
                                            number_prefix,
                                            snap.pr_number,
                                            e
                                        );
                                    }
                                    Some("error")
                                }
                            }
                        }
                    };
                    if let Some(s) = streamer.as_mut() {
                        let error = if result == Some("error") {
                            Some(format!(
                                "Could not {} nav comment on {} {}{}",
                                if existing.is_some() {
                                    "update"
                                } else {
                                    "create"
                                },
                                provider.pr_label(),
                                number_prefix,
                                snap.pr_number
                            ))
                        } else {
                            None
                        };
                        s.emit(&SyncStreamingResponse {
                            version: OUTPUT_VERSION,
                            command: "sync".to_string(),
                            event: SyncStreamingEvent::NavComment {
                                position: snap.json_index + 1,
                                pr_number: snap.pr_number,
                                action: result.unwrap_or("skip").to_string(),
                                error,
                            },
                        });
                    }
                    result
                }
                stack_nav::NavAction::Delete => {
                    let result = match existing {
                        Some(c) => {
                            segment.finish_on_error(
                                &repo,
                                &config,
                                authorize_review_mutation(
                                    &provider,
                                    snap.pr_number,
                                    &expected_head,
                                    &expected_base,
                                ),
                            )?;
                            let checkpoint = segment.mark_remote_before_attempt();
                            segment.finish_on_error(&repo, &config, checkpoint)?;
                            match provider.delete_pr_comment(snap.pr_number, c.id) {
                                Ok(()) => Some("deleted"),
                                Err(e) => {
                                    if !json && !jsonl {
                                        println!(
                                            "{} Could not delete nav comment on {} {}{}: {}",
                                            style("Warning:").yellow(),
                                            provider.pr_label(),
                                            number_prefix,
                                            snap.pr_number,
                                            e
                                        );
                                    }
                                    Some("error")
                                }
                            }
                        }
                        None => None,
                    };
                    if let Some(s) = streamer.as_mut() {
                        s.emit(&SyncStreamingResponse {
                            version: OUTPUT_VERSION,
                            command: "sync".to_string(),
                            event: SyncStreamingEvent::NavComment {
                                position: snap.json_index + 1,
                                pr_number: snap.pr_number,
                                action: result.unwrap_or("skip").to_string(),
                                error: if result == Some("error") {
                                    Some(format!(
                                        "Could not delete nav comment on {} {}{}",
                                        provider.pr_label(),
                                        number_prefix,
                                        snap.pr_number
                                    ))
                                } else {
                                    None
                                },
                            },
                        });
                    }
                    result
                }
            };

            if let Some(action) = action_result {
                if json || jsonl {
                    if let Some(entry_json) = json_entries.get_mut(snap.json_index) {
                        entry_json.nav_comment_action = Some(action.to_string());
                    }
                }
            }
        }
    } // end nav-comment reconcile

    // Save updated config
    if let Err(error) = config.save(git_dir) {
        return segment.finish_result(&repo, &config, Err(error));
    }
    segment.finish(&repo, &config)?;

    if json {
        print_json(&SyncResponse {
            version: OUTPUT_VERSION,
            sync: SyncResultJson {
                stack: stack.name.clone(),
                base: stack.base.clone(),
                rebased_before_sync,
                warnings: warnings.clone(),
                github_stack: github_stack_result.clone(),
                metadata: SyncMetadataJson {
                    gg_ids_added: metadata_counts.gg_ids_added,
                    gg_parents_updated: metadata_counts.gg_parents_updated,
                    gg_parents_removed: metadata_counts.gg_parents_removed,
                },
                entries: json_entries.clone(),
            },
        });
    } else if jsonl {
        if let Some(s) = streamer.as_mut() {
            s.emit(&SyncStreamingResponse {
                version: OUTPUT_VERSION,
                command: "sync".to_string(),
                event: SyncStreamingEvent::Summary {
                    stack: stack.name,
                    base: stack.base,
                    rebased_before_sync,
                    warnings: warnings.clone(),
                    github_stack: github_stack_result,
                    metadata: SyncMetadataJson {
                        gg_ids_added: metadata_counts.gg_ids_added,
                        gg_parents_updated: metadata_counts.gg_parents_updated,
                        gg_parents_removed: metadata_counts.gg_parents_removed,
                    },
                    entries: json_entries,
                },
            });
        }
    } else {
        println!();
        println!(
            "{} Synced {} commits",
            style("OK").green().bold(),
            entries_to_sync.len()
        );
    }

    Ok(())
}

fn sync_end_position(
    stack: &Stack,
    selected: Option<&crate::stack::StackEntry>,
    skipped: usize,
    initial_len: usize,
) -> Result<usize> {
    let Some(selected) = selected else {
        return Ok(stack.len());
    };
    if selected.position <= skipped || stack.is_empty() {
        return Ok(0);
    }
    let mut matches = stack.entries.iter().filter(|entry| {
        entry.oid == selected.oid || (selected.gg_id.is_some() && entry.gg_id == selected.gg_id)
    });
    match (matches.next(), matches.next()) {
        (Some(entry), None) => Ok(entry.position),
        // Lint and rebase preserve order, but can change every untagged SHA.
        // Only use the original position if no other entry was dropped.
        (None, None) if selected.gg_id.is_none() && stack.len() == initial_len - skipped => {
            Ok(selected.position - skipped)
        }
        _ => Err(GgError::Other(format!(
            "Cannot identify --until entry {} after updating the stack; inspect `gg ls` and retry.",
            selected.short_sha
        ))),
    }
}

fn restore_sync_start_position(
    repo: &Repository,
    start_branch: Option<&str>,
    start_head: git2::Oid,
    json: bool,
    jsonl: bool,
) -> Result<()> {
    if !json && !jsonl {
        println!(
            "{} Restoring repository to pre-sync state...",
            style("→").cyan()
        );
    }

    if let Some(branch) = start_branch {
        let branch_ref = format!("refs/heads/{}", branch);
        let mut reference = repo.find_reference(&branch_ref)?;
        reference.set_target(start_head, "gg sync: restore after lint failure")?;
        git::checkout_branch(repo, branch)?;
    } else {
        let commit = repo.find_commit(start_head)?;
        git::checkout_commit(repo, &commit)?;
    }

    if !json && !jsonl {
        println!("{} Repository restored", style("OK").green());
    }

    Ok(())
}

fn build_pr_payload(
    title: &str,
    description: Option<String>,
    stack_name: &str,
    short_sha: &str,
    template: Option<&str>,
) -> (String, String) {
    let body = match template {
        Some(tmpl) => {
            // Use template with placeholders
            let ctx = TemplateContext {
                description: description.as_deref(),
                stack_name,
                commit_sha: short_sha,
                title,
            };
            template::render_template(tmpl, &ctx)
        }
        None => {
            // Default behavior: use description or fallback
            let fallback = format!("Part of stack `{}`\n\nCommit: {}", stack_name, short_sha);
            description.unwrap_or(fallback)
        }
    };
    (title.to_string(), body)
}

fn is_wip_or_draft_prefix(title: &str) -> bool {
    let t = title.trim_start();
    let lower = t.to_ascii_lowercase();
    lower.starts_with("wip:") || lower.starts_with("draft:")
}

fn clean_title(title: &str) -> String {
    let trimmed = title.trim();
    trimmed.strip_suffix('.').unwrap_or(trimmed).to_string()
}

fn description_with_replacement_note(
    description: &str,
    provider: &Provider,
    old_pr_number: u64,
) -> String {
    format!(
        "{}\n\nReplaces {} {}{} because the source branch changed after `gg unstack`.",
        description,
        provider.pr_label(),
        provider.pr_number_prefix(),
        old_pr_number
    )
}

struct ReviewSyncTopology<'a> {
    repo: &'a Repository,
    config: &'a Config,
    expected_head: &'a str,
    expected_current_bases: &'a [String],
    desired_base: &'a str,
    active_previous_base: Option<&'a str>,
}

fn classify_review_for_sync(
    provider: &Provider,
    number: u64,
    topology: ReviewSyncTopology<'_>,
) -> Result<crate::provider::PrInfo> {
    let info = provider.get_pr_info(number)?;
    let expected_head_matches = info.head_branch.as_deref() == Some(topology.expected_head);
    let identity_matches = info.number == number && expected_head_matches;
    let current_topology_matches = info.base_branch.as_deref().is_some_and(|base| {
        topology
            .expected_current_bases
            .iter()
            .any(|expected| expected == base)
    });
    let reconciled_walk_back_matches =
        expected_head_matches && info.base_branch.as_deref() == Some(topology.desired_base);
    let rewritten_landing_transition_matches = expected_head_matches
        && topology
            .active_previous_base
            .is_some_and(|previous| info.base_branch.as_deref() == Some(previous));
    let migrated_topology_matches = migrated_review_topology_matches(
        topology.repo,
        topology.config,
        &info,
        topology.expected_head,
    );
    let mut target_matches = current_topology_matches
        || reconciled_walk_back_matches
        || rewritten_landing_transition_matches
        || migrated_topology_matches;
    if identity_matches && !target_matches {
        target_matches = mapped_merged_predecessor_review_matches(
            provider,
            topology.config,
            &info,
            topology.expected_head,
        )?;
    }
    if matches!(
        info.state,
        crate::provider::PrState::Merged | crate::provider::PrState::Closed
    ) {
        if identity_matches && target_matches {
            return Ok(info);
        }
        return Err(GgError::Other(format!(
            "Review {number} does not have the expected source and target; sync stopped."
        )));
    }
    if !matches!(
        info.state,
        crate::provider::PrState::Open | crate::provider::PrState::Draft
    ) || info.number != number
        || info.head_branch.is_none()
        || info.base_branch.is_none()
    {
        return Err(GgError::Other(format!(
            "Review {number} is not open at an identifiable source and target; sync stopped."
        )));
    }
    if !target_matches {
        return Err(GgError::Other(format!(
            "Review {number} does not have the expected source and target; sync stopped."
        )));
    }
    Ok(info)
}

/// Recognize the published topology left when `unstack` moves an entry to a
/// new stack while retaining its mapped review.
fn migrated_review_topology_matches(
    repo: &Repository,
    config: &Config,
    info: &crate::provider::PrInfo,
    expected_head: &str,
) -> bool {
    let Some((current_user, current_stack, current_entry_id)) =
        git::parse_entry_branch(expected_head)
    else {
        return false;
    };
    let Some(current_entry_id) = git::normalize_gg_id(&current_entry_id) else {
        return false;
    };
    let Some((source_user, source_stack, source_entry_id)) = info
        .head_branch
        .as_deref()
        .and_then(git::parse_entry_branch)
    else {
        return false;
    };
    let Some(source_entry_id) = git::normalize_gg_id(&source_entry_id) else {
        return false;
    };
    let Some((target_user, target_stack, target_entry_id)) = info
        .base_branch
        .as_deref()
        .and_then(git::parse_entry_branch)
    else {
        return false;
    };
    let Some(target_entry_id) = git::normalize_gg_id(&target_entry_id) else {
        return false;
    };

    if source_user != current_user
        || target_user != current_user
        || source_stack == current_stack
        || target_stack != source_stack
        || source_entry_id != current_entry_id
        || target_entry_id == source_entry_id
        || config.get_mr_for_entry(&current_stack, &current_entry_id) != Some(info.number)
    {
        return false;
    }

    let Some(old_stack) = config.get_stack(&source_stack) else {
        return false;
    };
    let target_is_accounted_for = old_stack.mrs.contains_key(&target_entry_id)
        || config
            .get_mr_for_entry(&current_stack, &target_entry_id)
            .is_some();
    if old_stack.mrs.contains_key(&source_entry_id) || !target_is_accounted_for {
        return false;
    }

    let Some(source_branch) = info.head_branch.as_deref() else {
        return false;
    };
    let Some(target_branch) = info.base_branch.as_deref() else {
        return false;
    };
    let source_ref = format!("refs/remotes/origin/{source_branch}");
    let target_ref = format!("refs/remotes/origin/{target_branch}");
    let Some(source_oid) = repo
        .find_reference(&source_ref)
        .ok()
        .and_then(|reference| reference.target())
    else {
        return false;
    };
    let Some(target_oid) = repo
        .find_reference(&target_ref)
        .ok()
        .and_then(|reference| reference.target())
    else {
        return false;
    };
    let Ok(source_commit) = repo.find_commit(source_oid) else {
        return false;
    };

    source_commit.parent_count() == 1
        && source_commit
            .parent_id(0)
            .is_ok_and(|parent_oid| parent_oid == target_oid)
}

/// Recognize a review that still targets a predecessor removed from the local
/// stack by `gg rebase` after that predecessor was merged externally.
fn mapped_merged_predecessor_review_matches(
    provider: &Provider,
    config: &Config,
    info: &crate::provider::PrInfo,
    expected_head: &str,
) -> Result<bool> {
    let Some((current_user, current_stack, current_entry_id)) =
        git::parse_entry_branch(expected_head)
    else {
        return Ok(false);
    };
    let Some(current_entry_id) = git::normalize_gg_id(&current_entry_id) else {
        return Ok(false);
    };
    let Some(target_branch) = info.base_branch.as_deref() else {
        return Ok(false);
    };
    let Some((target_user, target_stack, target_entry_id)) = git::parse_entry_branch(target_branch)
    else {
        return Ok(false);
    };
    let Some(target_entry_id) = git::normalize_gg_id(&target_entry_id) else {
        return Ok(false);
    };

    if target_user != current_user
        || target_stack != current_stack
        || target_entry_id == current_entry_id
        || config.get_mr_for_entry(&current_stack, &current_entry_id) != Some(info.number)
    {
        return Ok(false);
    }

    let Some(predecessor_number) = config.get_mr_for_entry(&current_stack, &target_entry_id) else {
        return Ok(false);
    };
    if predecessor_number == info.number {
        return Ok(false);
    }

    let predecessor = provider.get_pr_info(predecessor_number)?;
    Ok(predecessor.number == predecessor_number
        && predecessor.state == crate::provider::PrState::Merged
        && predecessor.head_branch.as_deref() == Some(target_branch))
}

fn authorize_review_mutation(
    provider: &Provider,
    number: u64,
    expected_head: &str,
    expected_base: &str,
) -> Result<crate::provider::PrInfo> {
    let info = provider.get_pr_info(number)?;
    if info.number != number
        || !matches!(
            info.state,
            crate::provider::PrState::Open | crate::provider::PrState::Draft
        )
        || info.head_branch.as_deref() != Some(expected_head)
        || info.base_branch.as_deref() != Some(expected_base)
    {
        return Err(GgError::Other(format!(
            "Review {number} is not open at the expected source and target; sync stopped."
        )));
    }
    Ok(info)
}

fn mismatched_pr_head_branch<'a>(
    pr_info: Option<&'a crate::provider::PrInfo>,
    entry_branch: &str,
) -> Option<&'a str> {
    pr_info
        .and_then(|info| info.head_branch.as_deref())
        .filter(|head_branch| *head_branch != entry_branch)
}

fn replacement_closing_comment(
    provider: &Provider,
    old_head_branch: &str,
    new_head_branch: &str,
    new_pr_number: u64,
    new_pr_url: &str,
) -> String {
    let replacement = if new_pr_url.is_empty() {
        format!("{}{}", provider.pr_number_prefix(), new_pr_number)
    } else {
        format!(
            "{}{} ({})",
            provider.pr_number_prefix(),
            new_pr_number,
            new_pr_url
        )
    };

    format!(
        "Closed by git-gud because this stack entry moved to a new source branch after `gg unstack`.\n\nOld source branch: `{}`\nNew source branch: `{}`\nReplacement: {}",
        old_head_branch, new_head_branch, replacement
    )
}

/// Ensure a title has the "Draft: " prefix for GitLab when draft is true.
/// GitLab controls draft state via the title prefix, so when syncing with --draft,
/// we need to ensure the title has the "Draft: " prefix.
/// This function only adds the prefix if:
/// - The provider is GitLab
/// - is_draft is true
/// - The title doesn't already have the prefix (case-insensitive check)
#[allow(dead_code)]
fn ensure_draft_prefix_for_gitlab(title: &str, provider: &Provider, is_draft: bool) -> String {
    // Only add prefix for GitLab when draft is true
    if !is_draft || !matches!(provider, Provider::GitLab) {
        return title.to_string();
    }

    let trimmed = title.trim_start();
    let lower = trimmed.to_ascii_lowercase();

    // Don't double-add if it already has the prefix
    if lower.starts_with("draft:") {
        title.to_string()
    } else {
        format!("Draft: {}", title)
    }
}

/// Create a branch pointing to a specific entry's commit
fn create_entry_branch(
    repo: &Repository,
    _stack: &Stack,
    entry: &crate::stack::StackEntry,
    branch_name: &str,
) -> Result<()> {
    let commit = repo.find_commit(entry.oid)?;

    // Delete existing branch if it exists
    if let Ok(mut branch) = repo.find_branch(branch_name, git2::BranchType::Local) {
        branch.delete()?;
    }

    // Create new branch at commit
    repo.branch(branch_name, &commit, true)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_pr_payload, clean_title, compute_target_branch, description_with_replacement_note,
        ensure_draft_prefix_for_gitlab, github_stack_preflight_skip, is_wip_or_draft_prefix,
        migrated_review_topology_matches, mismatched_pr_head_branch, replacement_closing_comment,
        suspend_sync_progress, sync_progress_bar, SyncSegment,
    };
    use crate::config::{Config, GithubStacksIntegration};
    use crate::git;
    use crate::github_stacks::{GithubStackReason, GithubStackSyncResult};
    use crate::operations::{OperationRecord, OperationStatus, RemoteEffect};
    use crate::output::{
        SyncEntryResultJson, SyncMetadataJson, SyncResponse, SyncResultJson, OUTPUT_VERSION,
    };
    use indicatif::{ProgressDrawTarget, TermLike};
    use std::fs;
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    #[derive(Debug)]
    struct FlushCountingTerm {
        flush_count: Arc<AtomicUsize>,
    }

    #[test]
    fn failed_prune_checkpoint_cannot_be_committed_from_segment_memory() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let config = Config::default();
        let mut segment = SyncSegment::new();
        segment.ensure_started(&repo, &config).unwrap();
        let operation_id = segment.guard_mut().id().to_string();
        let blocked_tmp = repo
            .path()
            .join("gg/operations")
            .join(format!("{operation_id}.json.tmp"));
        fs::create_dir(&blocked_tmp).unwrap();

        let result = segment.record_remote_effect_durable(RemoteEffect::BranchDeleted {
            remote: "origin".to_string(),
            branch: "testuser/stack--c-1111111".to_string(),
            prior_oid: Some("1111111111111111111111111111111111111111".to_string()),
        });
        assert!(result.is_err(), "blocked checkpoint unexpectedly succeeded");

        fs::remove_dir(blocked_tmp).unwrap();
        segment.finish(&repo, &config).unwrap();
        let record: OperationRecord = serde_json::from_slice(
            &fs::read(
                repo.path()
                    .join("gg/operations")
                    .join(format!("{operation_id}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(record.status, OperationStatus::Committed);
        assert!(
            record.remote_effects.is_empty(),
            "a failed durable checkpoint leaked into finalization"
        );
        assert!(!record.touched_remote);
    }

    impl TermLike for FlushCountingTerm {
        fn width(&self) -> u16 {
            80
        }

        fn move_cursor_up(&self, _n: usize) -> io::Result<()> {
            Ok(())
        }

        fn move_cursor_down(&self, _n: usize) -> io::Result<()> {
            Ok(())
        }

        fn move_cursor_right(&self, _n: usize) -> io::Result<()> {
            Ok(())
        }

        fn move_cursor_left(&self, _n: usize) -> io::Result<()> {
            Ok(())
        }

        fn write_line(&self, _s: &str) -> io::Result<()> {
            Ok(())
        }

        fn write_str(&self, _s: &str) -> io::Result<()> {
            Ok(())
        }

        fn clear_line(&self) -> io::Result<()> {
            Ok(())
        }

        fn flush(&self) -> io::Result<()> {
            self.flush_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[test]
    fn sync_progress_bar_keeps_rendering_without_progress_updates() {
        let flush_count = Arc::new(AtomicUsize::new(0));
        let draw_target = ProgressDrawTarget::term_like(Box::new(FlushCountingTerm {
            flush_count: flush_count.clone(),
        }));
        let progress_bar = sync_progress_bar(1, draw_target);
        progress_bar.set_message("Processing abc1234...");
        let initial_flush_count = flush_count.load(Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(1);

        while flush_count.load(Ordering::Relaxed) == initial_flush_count
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }

        let redrew_while_idle = flush_count.load(Ordering::Relaxed) > initial_flush_count;
        progress_bar.finish_and_clear();
        assert!(
            redrew_while_idle,
            "the spinner did not redraw while sync work was idle"
        );
    }

    #[test]
    fn sync_progress_bar_pauses_rendering_during_interactive_output() {
        let flush_count = Arc::new(AtomicUsize::new(0));
        let draw_target = ProgressDrawTarget::term_like(Box::new(FlushCountingTerm {
            flush_count: flush_count.clone(),
        }));
        let progress_bar = sync_progress_bar(1, draw_target);
        progress_bar.set_message("Processing abc1234...");

        let flushes_while_suspended = suspend_sync_progress(&progress_bar, || {
            let initial_flush_count = flush_count.load(Ordering::Relaxed);
            thread::sleep(Duration::from_millis(250));
            flush_count.load(Ordering::Relaxed) - initial_flush_count
        });

        progress_bar.finish_and_clear();
        assert_eq!(
            flushes_while_suspended, 0,
            "the spinner redrew while interactive output owned the terminal"
        );
    }

    #[test]
    fn test_get_remote_branch_oid() {
        // This is a simple unit test for the new function
        // Integration tests for the full sync flow exist in tests/integration_tests.rs
        use git2::Repository;

        // Create a temporary test repo
        let temp_dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(temp_dir.path()).unwrap();

        // Non-existent remote branch should return None
        let result = git::get_remote_branch_oid(&repo, "non-existent-branch");
        assert!(result.is_none());
    }

    #[test]
    fn test_build_pr_payload_prefers_description() {
        let (title, description) = build_pr_payload(
            "Add feature",
            Some("Details here".to_string()),
            "stack",
            "abc123",
            None,
        );
        assert_eq!(title, "Add feature");
        assert_eq!(description, "Details here");
    }

    #[test]
    fn test_build_pr_payload_falls_back_without_description() {
        let (title, description) = build_pr_payload("Add feature", None, "stack", "abc123", None);
        assert_eq!(title, "Add feature");
        assert_eq!(description, "Part of stack `stack`\n\nCommit: abc123");
    }

    #[test]
    fn test_clean_title_trims_trailing_period() {
        assert_eq!(clean_title("Add feature."), "Add feature");
        assert_eq!(clean_title("Add feature"), "Add feature");
        assert_eq!(clean_title(" Add feature. "), "Add feature");
    }

    #[test]
    fn test_is_wip_or_draft_prefix_case_insensitive() {
        assert!(is_wip_or_draft_prefix("WIP: something"));
        assert!(is_wip_or_draft_prefix("wip: something"));
        assert!(is_wip_or_draft_prefix("Draft: something"));
        assert!(is_wip_or_draft_prefix("draft: something"));
        assert!(is_wip_or_draft_prefix("   DRAFT: leading spaces"));
        assert!(!is_wip_or_draft_prefix("Not wip: prefix"));
        assert!(!is_wip_or_draft_prefix("WIP something"));
    }

    #[test]
    fn test_build_pr_payload_description_should_not_contain_gg_id() {
        // The description passed to build_pr_payload should already be filtered
        // by get_commit_description (which uses strip_gg_id_from_message internally).
        // This test documents that expectation - the caller is responsible for
        // passing a clean description without any GG-ID trailers.
        let clean_description = "This is the body.\n\nMore details about the change.";
        let (_, description) = build_pr_payload(
            "Add feature",
            Some(clean_description.to_string()),
            "stack",
            "abc123",
            None,
        );
        // Verify the description is passed through unchanged
        assert_eq!(description, clean_description);
        // And confirm no GG-ID trailer is present (which would indicate a bug in the caller)
        assert!(!description.contains("GG-ID:"));
    }

    #[test]
    fn test_build_pr_payload_with_template() {
        let template =
            "# {{title}}\n\n{{description}}\n\n---\nStack: {{stack_name}} | Commit: {{commit_sha}}";
        let (title, description) = build_pr_payload(
            "Add feature",
            Some("This is the description".to_string()),
            "my-stack",
            "abc1234",
            Some(template),
        );
        assert_eq!(title, "Add feature");
        assert_eq!(
            description,
            "# Add feature\n\nThis is the description\n\n---\nStack: my-stack | Commit: abc1234"
        );
    }

    #[test]
    fn test_build_pr_payload_with_template_no_description() {
        let template = "## {{title}}\n\n{{description}}\n\nPart of `{{stack_name}}`";
        let (title, description) =
            build_pr_payload("Fix bug", None, "bugfix", "def5678", Some(template));
        assert_eq!(title, "Fix bug");
        // {{description}} should be replaced with empty string when None
        assert_eq!(description, "## Fix bug\n\n\n\nPart of `bugfix`");
    }

    #[test]
    fn test_build_pr_payload_template_overrides_default_behavior() {
        // When template is provided, it should be used even if description is None
        // (instead of the default fallback)
        let template = "Custom: {{title}}";
        let (_, description) = build_pr_payload("Test", None, "stack", "abc", Some(template));
        assert_eq!(description, "Custom: Test");
        // Should NOT contain the default fallback
        assert!(!description.contains("Part of stack"));
    }

    #[test]
    fn test_ensure_draft_prefix_for_gitlab_adds_prefix() {
        use crate::provider::Provider;
        // GitLab + draft = should add prefix
        assert_eq!(
            ensure_draft_prefix_for_gitlab("Add feature", &Provider::GitLab, true),
            "Draft: Add feature"
        );
    }

    #[test]
    fn test_ensure_draft_prefix_for_gitlab_no_double_add() {
        use crate::provider::Provider;
        // Should not double-add if already present
        assert_eq!(
            ensure_draft_prefix_for_gitlab("Draft: Add feature", &Provider::GitLab, true),
            "Draft: Add feature"
        );
        assert_eq!(
            ensure_draft_prefix_for_gitlab("draft: Add feature", &Provider::GitLab, true),
            "draft: Add feature"
        );
        assert_eq!(
            ensure_draft_prefix_for_gitlab("DRAFT: Add feature", &Provider::GitLab, true),
            "DRAFT: Add feature"
        );
    }

    #[test]
    fn test_ensure_draft_prefix_for_gitlab_non_draft() {
        use crate::provider::Provider;
        // GitLab + not draft = no prefix
        assert_eq!(
            ensure_draft_prefix_for_gitlab("Add feature", &Provider::GitLab, false),
            "Add feature"
        );
    }

    #[test]
    fn test_ensure_draft_prefix_for_github_unchanged() {
        use crate::provider::Provider;
        // GitHub doesn't use title prefix for draft, so should be unchanged
        assert_eq!(
            ensure_draft_prefix_for_gitlab("Add feature", &Provider::GitHub, true),
            "Add feature"
        );
        assert_eq!(
            ensure_draft_prefix_for_gitlab("Add feature", &Provider::GitHub, false),
            "Add feature"
        );
    }

    #[test]
    fn test_ensure_draft_prefix_with_whitespace() {
        use crate::provider::Provider;
        // Should handle leading whitespace in draft prefix check
        assert_eq!(
            ensure_draft_prefix_for_gitlab("  Draft: Add feature", &Provider::GitLab, true),
            "  Draft: Add feature"
        );
    }

    #[test]
    fn test_build_pr_payload_wrapped_in_managed_markers() {
        use crate::managed_body;

        let (_, description) = build_pr_payload(
            "Add feature",
            Some("Details here".to_string()),
            "stack",
            "abc123",
            None,
        );
        let wrapped = managed_body::wrap(&description);
        assert!(wrapped.starts_with("<!-- gg:managed:start -->"));
        assert!(wrapped.ends_with("<!-- gg:managed:end -->"));
        assert!(wrapped.contains("Details here"));

        // Simulate user editing the PR body around the managed block
        let user_edited = format!(
            "## Checklist\n- [x] Tests pass\n- [ ] Docs updated\n\n{}\n\n## Notes\nReviewer comments here",
            wrapped
        );

        // Re-sync with updated description preserves user edits
        let (_, new_description) = build_pr_payload(
            "Add feature v2",
            Some("Updated details".to_string()),
            "stack",
            "abc123",
            None,
        );
        let result = managed_body::replace_managed(&user_edited, &new_description).unwrap();
        assert!(result.contains("- [x] Tests pass"));
        assert!(result.contains("- [ ] Docs updated"));
        assert!(result.contains("Reviewer comments here"));
        assert!(result.contains("Updated details"));
        assert!(!result.contains("Details here"));
    }

    #[test]
    fn test_description_with_replacement_note_mentions_old_pr() {
        use crate::provider::Provider;

        let description =
            description_with_replacement_note("Original body", &Provider::GitHub, 428);

        assert!(description.contains("Original body"));
        assert!(description.contains("Replaces PR #428"));
        assert!(description.contains("source branch changed after `gg unstack`"));
    }

    #[test]
    fn test_mismatched_pr_head_branch_detects_old_stack_head() {
        let info = crate::provider::PrInfo {
            number: 428,
            title: "Test PR".to_string(),
            state: crate::provider::PrState::Open,
            url: "https://example.com/pr/428".to_string(),
            head_branch: Some("testuser/old-stack--c-8b999da".to_string()),
            base_branch: None,
            merge_sha: None,
            draft: false,
            approved: false,
            mergeable: true,
            changes_requested: false,
            detailed_merge_status: None,
            auto_merge: false,
            head_sha: None,
            base_mergeability: crate::provider::BaseMergeability::Unknown,
        };

        assert_eq!(
            mismatched_pr_head_branch(Some(&info), "testuser/new-stack--c-8b999da"),
            Some("testuser/old-stack--c-8b999da")
        );
    }

    #[test]
    fn test_mismatched_pr_head_branch_skips_matching_or_unknown_head() {
        let matching = crate::provider::PrInfo {
            number: 428,
            title: "Test PR".to_string(),
            state: crate::provider::PrState::Open,
            url: "https://example.com/pr/428".to_string(),
            head_branch: Some("testuser/new-stack--c-8b999da".to_string()),
            base_branch: None,
            merge_sha: None,
            draft: false,
            approved: false,
            mergeable: true,
            changes_requested: false,
            detailed_merge_status: None,
            auto_merge: false,
            head_sha: None,
            base_mergeability: crate::provider::BaseMergeability::Unknown,
        };
        let unknown = crate::provider::PrInfo {
            head_branch: None,
            ..matching.clone()
        };

        assert_eq!(
            mismatched_pr_head_branch(Some(&matching), "testuser/new-stack--c-8b999da"),
            None
        );
        assert_eq!(
            mismatched_pr_head_branch(Some(&unknown), "testuser/new-stack--c-8b999da"),
            None
        );
    }

    struct MigratedTopologyFixture {
        repo: git2::Repository,
        _temp: tempfile::TempDir,
        config: Config,
        review: crate::provider::PrInfo,
        old_base_oid: git2::Oid,
    }

    fn commit_with_parent(
        repo: &git2::Repository,
        message: &str,
        parent_oid: Option<git2::Oid>,
    ) -> git2::Oid {
        let signature = git2::Signature::now("Test User", "test@example.com").unwrap();
        let tree_oid = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        match parent_oid {
            Some(parent_oid) => {
                let parent = repo.find_commit(parent_oid).unwrap();
                repo.commit(None, &signature, &signature, message, &tree, &[&parent])
                    .unwrap()
            }
            None => repo
                .commit(None, &signature, &signature, message, &tree, &[])
                .unwrap(),
        }
    }

    fn migrated_topology_fixture() -> MigratedTopologyFixture {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        let old_base_oid = commit_with_parent(&repo, "old base", None);
        let predecessor_oid = commit_with_parent(&repo, "predecessor", Some(old_base_oid));
        let source_oid = commit_with_parent(&repo, "migrated source", Some(predecessor_oid));
        repo.reference(
            "refs/remotes/origin/testuser/old-stack--c-1111111",
            predecessor_oid,
            true,
            "test fixture",
        )
        .unwrap();
        repo.reference(
            "refs/remotes/origin/testuser/old-stack--c-2222222",
            source_oid,
            true,
            "test fixture",
        )
        .unwrap();

        let mut config = Config::default();
        config.set_mr_for_entry("upper", "c-2222222", 428);
        config.set_mr_for_entry("old-stack", "c-1111111", 427);
        let review = crate::provider::PrInfo {
            number: 428,
            title: "Test PR".to_string(),
            state: crate::provider::PrState::Open,
            url: "https://example.com/pr/428".to_string(),
            head_branch: Some("testuser/old-stack--c-2222222".to_string()),
            base_branch: Some("testuser/old-stack--c-1111111".to_string()),
            merge_sha: None,
            draft: false,
            approved: false,
            mergeable: true,
            changes_requested: false,
            detailed_merge_status: None,
            auto_merge: false,
            head_sha: None,
            base_mergeability: crate::provider::BaseMergeability::Unknown,
        };
        MigratedTopologyFixture {
            repo,
            _temp: temp,
            config,
            review,
            old_base_oid,
        }
    }

    #[test]
    fn migrated_topology_accepts_retained_old_stack_predecessor() {
        let fixture = migrated_topology_fixture();
        assert!(migrated_review_topology_matches(
            &fixture.repo,
            &fixture.config,
            &fixture.review,
            "testuser/upper--c-2222222",
        ));
    }

    #[test]
    fn migrated_topology_accepts_predecessor_mapped_in_current_stack() {
        let fixture = migrated_topology_fixture();
        let mut config = fixture.config.clone();
        config.remove_mr_for_entry("old-stack", "c-1111111");
        config.set_mr_for_entry("upper", "c-1111111", 900);

        assert!(migrated_review_topology_matches(
            &fixture.repo,
            &config,
            &fixture.review,
            "testuser/upper--c-2222222",
        ));
    }

    #[test]
    fn migrated_topology_rejects_predecessor_absent_from_both_stacks() {
        let fixture = migrated_topology_fixture();
        let mut config = fixture.config.clone();
        config.remove_mr_for_entry("old-stack", "c-1111111");

        assert!(!migrated_review_topology_matches(
            &fixture.repo,
            &config,
            &fixture.review,
            "testuser/upper--c-2222222",
        ));
    }

    #[test]
    fn migrated_topology_requires_old_stack_and_moved_source() {
        let fixture = migrated_topology_fixture();

        let mut missing_old_stack = fixture.config.clone();
        missing_old_stack.remove_stack("old-stack");
        assert!(!migrated_review_topology_matches(
            &fixture.repo,
            &missing_old_stack,
            &fixture.review,
            "testuser/upper--c-2222222",
        ));

        let mut source_still_mapped = fixture.config.clone();
        source_still_mapped.set_mr_for_entry("old-stack", "c-2222222", 428);
        assert!(!migrated_review_topology_matches(
            &fixture.repo,
            &source_still_mapped,
            &fixture.review,
            "testuser/upper--c-2222222",
        ));
    }

    #[test]
    fn migrated_topology_requires_both_exact_remote_refs() {
        for missing_ref in [
            "refs/remotes/origin/testuser/old-stack--c-1111111",
            "refs/remotes/origin/testuser/old-stack--c-2222222",
        ] {
            let fixture = migrated_topology_fixture();
            fixture
                .repo
                .find_reference(missing_ref)
                .unwrap()
                .delete()
                .unwrap();
            assert!(!migrated_review_topology_matches(
                &fixture.repo,
                &fixture.config,
                &fixture.review,
                "testuser/upper--c-2222222",
            ));
        }
    }

    #[test]
    fn migrated_topology_requires_source_to_directly_parent_target() {
        let fixture = migrated_topology_fixture();
        fixture
            .repo
            .find_reference("refs/remotes/origin/testuser/old-stack--c-1111111")
            .unwrap()
            .set_target(fixture.old_base_oid, "break direct-parent evidence")
            .unwrap();

        assert!(!migrated_review_topology_matches(
            &fixture.repo,
            &fixture.config,
            &fixture.review,
            "testuser/upper--c-2222222",
        ));
    }

    #[test]
    fn test_replacement_closing_comment_mentions_branches_and_replacement() {
        use crate::provider::Provider;

        let comment = replacement_closing_comment(
            &Provider::GitHub,
            "testuser/old--c-1234567",
            "testuser/new--c-1234567",
            430,
            "https://github.com/org/repo/pull/430",
        );

        assert!(comment.contains("testuser/old--c-1234567"));
        assert!(comment.contains("testuser/new--c-1234567"));
        assert!(comment.contains("#430"));
        assert!(comment.contains("https://github.com/org/repo/pull/430"));
    }

    #[test]
    fn test_build_pr_payload_with_template_wrapped_survives_resync() {
        use crate::managed_body;

        let template = "## {{title}}\n\n{{description}}\n\n---\nStack: {{stack_name}}";
        let (_, description) = build_pr_payload(
            "Fix bug",
            Some("Bug fix description".to_string()),
            "my-stack",
            "def456",
            Some(template),
        );
        let wrapped = managed_body::wrap(&description);

        // User adds checklist before and after
        let body = format!("- [x] Review done\n\n{}\n\n- [ ] Deploy verified", wrapped);

        // Re-sync with new description
        let (_, new_desc) = build_pr_payload(
            "Fix bug v2",
            Some("Updated fix".to_string()),
            "my-stack",
            "def456",
            Some(template),
        );
        let result = managed_body::replace_managed(&body, &new_desc).unwrap();
        assert!(result.contains("- [x] Review done"));
        assert!(result.contains("- [ ] Deploy verified"));
        assert!(result.contains("Fix bug v2"));
        assert!(result.contains("Updated fix"));
    }

    #[test]
    fn test_legacy_body_without_markers_returns_none() {
        use crate::managed_body;

        let legacy_body =
            "This is a PR created before managed markers were introduced.\n\n- [x] Tests pass";
        assert!(managed_body::replace_managed(legacy_body, "New content").is_none());
    }

    #[test]
    fn test_sync_json_response_structure() {
        let github_stack = GithubStackSyncResult::skipped(
            GithubStacksIntegration::Auto,
            GithubStackReason::InsufficientPrs,
            vec![42],
        );
        let response = SyncResponse {
            version: OUTPUT_VERSION,
            sync: SyncResultJson {
                stack: "test-stack".to_string(),
                base: "main".to_string(),
                rebased_before_sync: false,
                warnings: vec![],
                github_stack: Some(github_stack),
                metadata: SyncMetadataJson::default(),
                entries: vec![SyncEntryResultJson {
                    position: 1,
                    sha: "abc1234".to_string(),
                    title: "Add feature".to_string(),
                    gg_id: "c-abc1234".to_string(),
                    branch: "user/test-stack/c-abc1234".to_string(),
                    action: "created".to_string(),
                    pr_number: Some(42),
                    pr_url: Some("https://github.com/org/repo/pull/42".to_string()),
                    draft: false,
                    pushed: true,
                    error: None,
                    nav_comment_action: None,
                }],
            },
        };

        let json_str = serde_json::to_string_pretty(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed["version"], OUTPUT_VERSION);
        assert_eq!(parsed["sync"]["stack"], "test-stack");
        assert_eq!(parsed["sync"]["base"], "main");
        assert_eq!(parsed["sync"]["rebased_before_sync"], false);
        assert!(parsed["sync"]["warnings"].is_array());
        assert_eq!(parsed["sync"]["github_stack"]["reason"], "insufficient_prs");
        assert!(parsed["sync"]["entries"].is_array());

        let entry = &parsed["sync"]["entries"][0];
        assert_eq!(entry["position"], 1);
        assert_eq!(entry["action"], "created");
        assert_eq!(entry["pr_number"], 42);
        assert_eq!(entry["pushed"], true);
        assert!(entry["error"].is_null());
    }

    #[test]
    fn github_stack_preflight_off_is_disabled() {
        let result =
            github_stack_preflight_skip(GithubStacksIntegration::Off, None, &[41, 42], false)
                .unwrap();
        assert_eq!(result.reason, Some(GithubStackReason::Disabled));
    }

    #[test]
    fn github_stack_preflight_partial_sync_skips_before_backend() {
        let result =
            github_stack_preflight_skip(GithubStacksIntegration::Auto, Some("2"), &[41, 42], false)
                .unwrap();
        assert_eq!(result.reason, Some(GithubStackReason::PartialSync));
    }

    #[test]
    fn github_stack_preflight_unresolved_prs_skip_before_insufficient_prs() {
        let result =
            github_stack_preflight_skip(GithubStacksIntegration::Auto, None, &[41], true).unwrap();
        assert_eq!(result.reason, Some(GithubStackReason::UnresolvedPrs));
    }

    #[test]
    fn github_stack_preflight_fewer_than_two_active_prs_is_insufficient() {
        let result =
            github_stack_preflight_skip(GithubStacksIntegration::Auto, None, &[41], false).unwrap();
        assert_eq!(result.reason, Some(GithubStackReason::InsufficientPrs));
    }

    // --- Tests for compute_target_branch (walk-back algorithm) ---

    fn make_test_stack(entries: Vec<crate::stack::StackEntry>) -> crate::stack::Stack {
        crate::stack::Stack {
            name: "test-stack".to_string(),
            username: "user".to_string(),
            base: "main".to_string(),
            entries,
            current_position: None,
        }
    }

    fn make_test_entry(gg_id: &str) -> crate::stack::StackEntry {
        crate::stack::StackEntry {
            oid: git2::Oid::ZERO_SHA1,
            short_sha: "abc1234".to_string(),
            title: "Test entry".to_string(),
            gg_id: Some(gg_id.to_string()),
            gg_parent: None,
            mr_number: None,
            mr_state: None,
            approved: false,
            ci_status: None,
            position: 1,
            in_merge_train: false,
            merge_train_position: None,
            auto_merge: false,
            changes_requested: false,
            mergeable: false,
        }
    }

    #[test]
    fn test_compute_target_branch_first_entry_targets_base() {
        let entries = vec![make_test_entry("c-aaa1111")];
        let stack = make_test_stack(entries.clone());
        let closed = vec![false];
        let result = compute_target_branch(0, "main", &entries, &closed, &stack);
        assert_eq!(result, "main");
    }

    #[test]
    fn test_compute_target_branch_targets_previous_when_open() {
        let entries = vec![make_test_entry("c-aaa1111"), make_test_entry("c-bbb2222")];
        let stack = make_test_stack(entries.clone());
        let closed = vec![false];
        let result = compute_target_branch(1, "main", &entries, &closed, &stack);
        // Should target the branch for entries[0]
        let expected = stack.entry_branch_name(&entries[0]).unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_compute_target_branch_skips_merged_predecessor() {
        let entries = vec![make_test_entry("c-aaa1111"), make_test_entry("c-bbb2222")];
        let stack = make_test_stack(entries.clone());
        // entries[0] is merged
        let closed = vec![true];
        let result = compute_target_branch(1, "main", &entries, &closed, &stack);
        // All predecessors merged → falls back to base
        assert_eq!(result, "main");
    }

    #[test]
    fn test_compute_target_branch_skips_multiple_merged_predecessors() {
        let entries = vec![
            make_test_entry("c-aaa1111"),
            make_test_entry("c-bbb2222"),
            make_test_entry("c-ccc3333"),
        ];
        let stack = make_test_stack(entries.clone());
        // entries[0] and entries[1] are both merged
        let closed = vec![true, true];
        let result = compute_target_branch(2, "main", &entries, &closed, &stack);
        assert_eq!(result, "main");
    }

    #[test]
    fn test_compute_target_branch_finds_nearest_open_ancestor() {
        let entries = vec![
            make_test_entry("c-aaa1111"),
            make_test_entry("c-bbb2222"),
            make_test_entry("c-ccc3333"),
        ];
        let stack = make_test_stack(entries.clone());
        // entries[0] is open, entries[1] is merged
        let closed = vec![false, true];
        let result = compute_target_branch(2, "main", &entries, &closed, &stack);
        // Should skip entries[1] (merged) and target entries[0]
        let expected = stack.entry_branch_name(&entries[0]).unwrap();
        assert_eq!(result, expected);
    }
}

/// Max bytes of failed-command output carried on `lint_command_done` events.
const LINT_EVENT_OUTPUT_MAX_BYTES: usize = 8 * 1024;

fn tail_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

fn lint_progress_event(event: crate::commands::run::RunProgress) -> SyncStreamingEvent {
    use crate::commands::run::RunProgress as P;
    match event {
        P::Started {
            total_entries,
            commands,
        } => SyncStreamingEvent::LintStarted {
            total_entries,
            commands,
        },
        P::EntryStarted {
            position,
            sha,
            title,
        } => SyncStreamingEvent::LintEntryStarted {
            position,
            sha,
            title,
        },
        P::CommandDone {
            position,
            command,
            passed,
            duration_ms,
            output,
        } => SyncStreamingEvent::LintCommandDone {
            position,
            command,
            passed,
            duration_ms,
            output: output.map(|o| tail_bytes(&o, LINT_EVENT_OUTPUT_MAX_BYTES)),
        },
        P::EntryDone {
            position,
            sha,
            passed,
            amended,
        } => SyncStreamingEvent::LintEntryDone {
            position,
            sha,
            passed,
            amended,
        },
        P::Done { all_passed } => SyncStreamingEvent::LintDone { all_passed },
    }
}

/// Build the sync-abort error, naming the first failing commit and command.
fn lint_failure_message(results: &[crate::output::RunCommitResult]) -> String {
    let detail = results
        .iter()
        .find(|r| !r.passed)
        .map(|r| {
            let cmd = r
                .commands
                .iter()
                .find(|c| !c.passed)
                .map(|c| format!(" (command: {})", c.command))
                .unwrap_or_default();
            format!(": first failure at [{}] {}{}", r.position, r.sha, cmd)
        })
        .unwrap_or_default();
    format!(
        "Lint failed for one or more commits{}. Sync aborted; repository restored to its original state.",
        detail
    )
}

#[cfg(test)]
mod lint_event_tests {
    use super::*;
    use crate::output::{RunCommandResult, RunCommitResult};

    #[test]
    fn tail_bytes_keeps_tail_on_char_boundary() {
        assert_eq!(tail_bytes("abc", 10), "abc");
        assert_eq!(tail_bytes("abcdef", 3), "…def");
        assert_eq!(tail_bytes("aéé", 3), "…é");
    }

    #[test]
    fn failure_message_names_commit_and_command() {
        let results = vec![
            RunCommitResult {
                position: 1,
                sha: "aaa".into(),
                title: "t".into(),
                passed: true,
                commands: vec![],
            },
            RunCommitResult {
                position: 2,
                sha: "bbb".into(),
                title: "t".into(),
                passed: false,
                commands: vec![RunCommandResult {
                    command: "./lint.sh".into(),
                    passed: false,
                    output: None,
                }],
            },
        ];
        let msg = lint_failure_message(&results);
        assert!(msg.starts_with("Lint failed for one or more commits"));
        assert!(msg.contains("[2] bbb"));
        assert!(msg.contains("./lint.sh"));
        assert!(msg.contains("restored"));
    }
}
