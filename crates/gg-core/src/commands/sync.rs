//! `gg sync` - Sync stack with remote provider (push branches and create/update PRs/MRs)

use console::style;
use dialoguer::Confirm;
use git2::Repository;
use indicatif::{ProgressBar, ProgressStyle};

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git::{self, get_commit_description, strip_gg_id_from_message};
use crate::managed_body;
use crate::operations::{OperationKind, RemoteEffect, SnapshotScope};
use crate::output::{
    print_json, SyncEntryResultJson, SyncMetadataJson, SyncResponse, SyncResultJson, OUTPUT_VERSION,
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

/// Format and display a push error with helpful context
fn maybe_rebase_if_base_is_behind(
    repo: &Repository,
    config: &Config,
    base_branch: &str,
    json: bool,
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
        if !json {
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
        crate::commands::rebase::run_with_repo(repo, None, json, false)?;
        return Ok(true);
    }

    if !json {
        println!(
            "{} Your stack is {} commits behind origin/{}. {} may show unrelated changes. Run 'gg rebase' first to update.",
            style("⚠").yellow().bold(),
            behind,
            base_branch,
            prs_label
        );
    }

    if json {
        return Ok(false);
    }

    let should_rebase = Confirm::new()
        .with_prompt("Rebase before syncing?")
        .default(true)
        .interact()
        .unwrap_or(true);

    if should_rebase {
        crate::commands::rebase::run_with_repo(repo, None, json, false)?;
        return Ok(true);
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

/// Run the sync command
#[allow(clippy::too_many_arguments)]
pub fn run(
    draft: bool,
    json: bool,
    no_rebase_check: bool,
    force: bool,
    update_descriptions: bool,
    update_title: bool,
    run_lint: bool,
    until: Option<String>,
    no_verify: bool,
) -> Result<()> {
    let repo = git::open_repo()?;

    let git_dir = repo.commondir();
    let mut config = Config::load_with_global(git_dir)?;

    // Acquire operation lock + record a Pending op for the undo log.
    let (_lock, mut guard) = git::acquire_operation_lock_and_record(
        &repo,
        &config,
        OperationKind::Sync,
        std::env::args().skip(1).collect(),
        None,
        SnapshotScope::AllUserBranches,
    )?;

    // Remote effects are persisted incrementally via `guard.record_remote_effect`
    // / `guard.mark_touched_remote` so a mid-loop failure leaves an accurate
    // trail (design §4.4). These locals mirror what has been recorded so the
    // success-path finalize can replay the same values without a disk read.
    let mut remote_effects: Vec<RemoteEffect> = Vec::new();
    let mut touched_remote = false;

    // Apply config defaults to CLI flags:
    // - draft: CLI flag OR config setting (either one enables drafts)
    // - update_descriptions: CLI flag OR config setting (either one enables updates;
    //   set sync_update_descriptions: false in config to opt out)
    let draft = draft || config.get_sync_draft();
    let update_descriptions = update_descriptions || config.get_sync_update_descriptions();
    let update_title = update_title || config.get_sync_update_title();

    // Load stack early to validate --until
    let initial_stack = Stack::load(&repo, &config)?;
    if initial_stack.is_empty() {
        if json {
            print_json(&SyncResponse {
                version: OUTPUT_VERSION,
                sync: SyncResultJson {
                    stack: initial_stack.name.clone(),
                    base: initial_stack.base.clone(),
                    rebased_before_sync: false,
                    warnings: vec![],
                    metadata: SyncMetadataJson::default(),
                    entries: vec![],
                },
            });
        } else {
            println!("{}", style("Stack is empty. Nothing to sync.").dim());
        }
        guard.finalize_with_scope(
            &repo,
            &config,
            SnapshotScope::AllUserBranches,
            vec![],
            false,
        )?;
        return Ok(());
    }

    // Validate --until parameter early (before provider checks and network calls)
    let lint_end_pos = if let Some(ref target) = until {
        Some(resolve_target(&initial_stack, target)?)
    } else {
        None
    };

    // Detect and check provider
    let provider = Provider::detect(&repo)?;
    provider.check_installed()?;
    provider.check_auth()?;

    // Fetch from remote to ensure we have up-to-date refs
    let _ = git::fetch_and_prune();

    let mut rebased_before_sync = false;
    if !no_rebase_check {
        rebased_before_sync =
            maybe_rebase_if_base_is_behind(&repo, &config, initial_stack.base.as_str(), json)?;
    }

    // Capture restore snapshot after optional pre-sync rebase. If lint fails,
    // we restore to this post-rebase state rather than silently undoing rebase.
    let sync_start_branch = git::current_branch_name(&repo);
    let sync_start_head = repo.head()?.peel_to_commit()?.id();

    let warnings: Vec<String> = Vec::new();

    // Run lint ONCE if requested (before GG-ID addition loop)
    if run_lint {
        // Reload stack to get post-rebase state. After rebase, landed commits
        // are dropped so initial_stack.len() may be stale.
        let current_stack = Stack::load(&repo, &config)?;
        // Clamp lint_end_pos to current stack size. If --until was specified but
        // rebase dropped commits (making the original position invalid), use the
        // new stack size instead of failing.
        let end_pos = lint_end_pos
            .map(|pos| pos.min(current_stack.len()))
            .unwrap_or(current_stack.len());
        if !json {
            println!("{}", console::style("Running lint before sync...").dim());
        }
        let lint_passed = crate::commands::lint::run(Some(end_pos), json, false)?;
        if !lint_passed {
            restore_sync_start_position(
                &repo,
                sync_start_branch.as_deref(),
                sync_start_head,
                json,
            )?;
            return Err(GgError::Other(
                "Lint failed for one or more commits. Sync aborted; repository restored to its original state."
                    .to_string(),
            ));
        }
        if !json {
            println!();
        }
    }

    let mut stack = Stack::load(&repo, &config)?;

    if stack.is_empty() {
        if json {
            print_json(&SyncResponse {
                version: OUTPUT_VERSION,
                sync: SyncResultJson {
                    stack: stack.name.clone(),
                    base: stack.base.clone(),
                    rebased_before_sync,
                    warnings: warnings.clone(),
                    metadata: SyncMetadataJson::default(),
                    entries: vec![],
                },
            });
        } else {
            println!("{}", style("Stack is empty. Nothing to sync.").dim());
        }
        guard.finalize_with_scope(
            &repo,
            &config,
            SnapshotScope::AllUserBranches,
            remote_effects,
            touched_remote,
        )?;
        return Ok(());
    }

    // Re-validate --until against potentially updated stack
    if let Some(ref target) = until {
        resolve_target(&stack, target)?;
    }

    if !json {
        println!("{}", style("Normalizing GG metadata...").dim());
    }
    // Intentional: sync always enforces GG-ID / GG-Parent invariants for the
    // stack so branch/PR mappings stay stable, even if auto_add_gg_ids is false.
    let metadata_counts = git::normalize_stack_metadata(&repo, &stack)?;
    stack = Stack::load(&repo, &config)?;

    // Determine sync range based on --until flag
    let sync_until = if let Some(ref target) = until {
        Some(resolve_target(&stack, target)?)
    } else {
        None
    };

    let entries_to_sync = if let Some(end_pos) = sync_until {
        &stack.entries[..end_pos]
    } else {
        &stack.entries[..]
    };

    // Load optional PR template
    let pr_template = template::load_template(git_dir);

    // Sync progress
    let pb = if json {
        ProgressBar::hidden()
    } else if atty::is(atty::Stream::Stderr) {
        let pb = ProgressBar::new(entries_to_sync.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("=>-"),
        );
        pb
    } else {
        ProgressBar::hidden()
    };

    // Process each entry
    // If a commit title starts with "WIP:" or "Draft:" (case-insensitive),
    // that PR and all subsequent PRs should be drafts.
    let mut force_draft = draft;
    let mut json_entries: Vec<SyncEntryResultJson> = Vec::new();
    let mut nav_snapshots: Vec<Option<NavEntrySnapshot>> = Vec::new();
    // Track which entries are closed/merged so downstream entries can skip them
    // when computing their target branch (walk-back algorithm for stacked MRs).
    let mut entry_is_closed: Vec<bool> = Vec::with_capacity(entries_to_sync.len());

    for (i, entry) in entries_to_sync.iter().enumerate() {
        let gg_id = entry.gg_id.as_ref().unwrap();
        let entry_branch = stack.entry_branch_name(entry).unwrap();
        let commit = repo.find_commit(entry.oid)?;
        let raw_title = strip_gg_id_from_message(&entry.title);

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
        let mut is_entry_closed = false;

        let (title, description) = build_pr_payload(
            &title,
            get_commit_description(&commit),
            &stack.name,
            &entry.short_sha,
            pr_template.as_deref(),
        );

        pb.set_message(format!("Processing {}...", entry.short_sha));

        // Create/update the remote branch for this commit
        create_entry_branch(&repo, &stack, entry, &entry_branch)?;

        // Check if remote branch exists and has the same OID as local
        let remote_oid = git::get_remote_branch_oid(&repo, &entry_branch);
        let needs_push = remote_oid != Some(entry.oid);

        // Only push if the remote is different or doesn't exist
        if needs_push {
            pushed = true;
            // Push the branch (always force-push with lease because rebases change commit SHAs)
            // This is safe because each entry branch is owned by this stack
            // If --force is passed, use hard force as an escape hatch
            let push_result = git::push_branch(&entry_branch, true, force, no_verify);
            if let Err(e) = push_result {
                pb.finish_and_clear();
                if json {
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
                    continue;
                }

                format_push_error(&e, &entry_branch);
                return Err(e);
            }

            // Record the push as a remote effect. `sync` always pushes with
            // force-with-lease because rebases rewrite entry-branch history;
            // the `force` field here reflects the hard --force escape hatch.
            let effect = RemoteEffect::Pushed {
                remote: "origin".to_string(),
                branch: entry_branch.clone(),
                force,
            };
            remote_effects.push(effect.clone());
            touched_remote = true;
            guard.record_remote_effect(effect);
        }

        // Determine target branch for MR — uses walk-back to skip merged predecessors.
        let target_branch =
            compute_target_branch(i, &stack.base, entries_to_sync, &entry_is_closed, &stack);

        // Create or update PR
        let existing_pr = config.get_mr_for_entry(&stack.name, gg_id);

        match existing_pr {
            Some(pr_num) => {
                pr_number = Some(pr_num);
                // Check if PR is still open before updating
                let pr_info = provider.get_pr_info(pr_num).ok();
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
                    // Skip updating closed/merged PRs
                    if !json {
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
                    let replacement_draft = pr_info
                        .as_ref()
                        .map(|info| info.draft)
                        .unwrap_or(entry_draft);
                    let replacement_description = managed_body::wrap(
                        &description_with_replacement_note(&description, &provider, pr_num),
                    );

                    match provider.create_pr(
                        &entry_branch,
                        &target_branch,
                        &title,
                        &replacement_description,
                        replacement_draft,
                    ) {
                        Ok(result) => {
                            config.set_mr_for_entry(&stack.name, gg_id, result.number);
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
                            remote_effects.push(created_effect.clone());
                            touched_remote = true;
                            guard.record_remote_effect(created_effect);

                            let close_comment = replacement_closing_comment(
                                &provider,
                                old_head_branch,
                                &entry_branch,
                                result.number,
                                &result.url,
                            );
                            if let Err(e) = provider.create_pr_comment(pr_num, &close_comment) {
                                if !json {
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
                                touched_remote = true;
                                guard.mark_touched_remote();
                            }

                            match provider.close_pr(pr_num) {
                                Ok(()) => {
                                    let closed_effect = RemoteEffect::PrClosed {
                                        number: pr_num,
                                        url: pr_info
                                            .as_ref()
                                            .map(|info| info.url.clone())
                                            .unwrap_or_default(),
                                    };
                                    remote_effects.push(closed_effect.clone());
                                    touched_remote = true;
                                    guard.record_remote_effect(closed_effect);
                                }
                                Err(e) => {
                                    if !json {
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

                            if !json {
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
                            action = "error".to_string();
                            entry_error = Some(e.to_string());
                            if !json {
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
                    if update_title {
                        if let Err(e) = provider.update_pr_title(pr_num, &title) {
                            if !json {
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
                                        if !json {
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
                                if !new_body.is_empty() {
                                    if let Err(e) =
                                        provider.update_pr_description(pr_num, &new_body)
                                    {
                                        if !json {
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
                                    }
                                }
                            }
                            Err(e) => {
                                // Could not read remote body — skip update to be safe
                                if !json {
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

                    // Update PR/MR base if needed. A successful `update_pr_base`
                    // call mutates remote state — even if the new base matches
                    // what the API already had, we've made a request that the
                    // provider treats as an authoritative update. Mark
                    // `touched_remote` so `gg undo` refuses to replay this sync
                    // locally (there's no safe local inverse for a remote base
                    // change).
                    match provider.update_pr_base(pr_num, &target_branch) {
                        Ok(()) => {
                            touched_remote = true;
                            guard.mark_touched_remote();
                        }
                        Err(e) => {
                            if !json {
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

                    // Show appropriate message based on whether we pushed
                    let status_msg = if needs_push {
                        "Force-pushed"
                    } else {
                        "Up to date"
                    };
                    if !json {
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
                    if needs_push || update_descriptions || update_title {
                        action = "updated".to_string();
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
                        remote_effects.push(effect.clone());
                        touched_remote = true;
                        guard.record_remote_effect(effect);

                        if !json {
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
                        if !json {
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

        if json {
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
            let json_index = if json {
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

        pb.inc(1);
    }

    if !json {
        pb.finish_with_message("Done!");
    }

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
                    if !json {
                        println!(
                            "{} Could not list comments on {} {}{}: {}",
                            style("Warning:").yellow(),
                            provider.pr_label(),
                            number_prefix,
                            snap.pr_number,
                            e
                        );
                    }
                    if json {
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

                    match existing {
                        Some(c) if c.body == body => Some("unchanged"),
                        Some(c) => match provider.update_pr_comment(snap.pr_number, c.id, &body) {
                            Ok(()) => Some("updated"),
                            Err(e) => {
                                if !json {
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
                        },
                        None => match provider.create_pr_comment(snap.pr_number, &body) {
                            Ok(()) => Some("created"),
                            Err(e) => {
                                if !json {
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
                        },
                    }
                }
                stack_nav::NavAction::Delete => match existing {
                    Some(c) => match provider.delete_pr_comment(snap.pr_number, c.id) {
                        Ok(()) => Some("deleted"),
                        Err(e) => {
                            if !json {
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
                    },
                    None => None,
                },
            };

            if let Some(action) = action_result {
                if json {
                    if let Some(entry_json) = json_entries.get_mut(snap.json_index) {
                        entry_json.nav_comment_action = Some(action.to_string());
                    }
                }
            }
        }
    } // end nav-comment reconcile

    // Save updated config
    config.save(git_dir)?;

    if json {
        print_json(&SyncResponse {
            version: OUTPUT_VERSION,
            sync: SyncResultJson {
                stack: stack.name,
                base: stack.base,
                rebased_before_sync,
                warnings,
                metadata: SyncMetadataJson {
                    gg_ids_added: metadata_counts.gg_ids_added,
                    gg_parents_updated: metadata_counts.gg_parents_updated,
                    gg_parents_removed: metadata_counts.gg_parents_removed,
                },
                entries: json_entries,
            },
        });
    } else {
        println!();
        println!(
            "{} Synced {} commits",
            style("OK").green().bold(),
            entries_to_sync.len()
        );
    }

    guard.finalize_with_scope(
        &repo,
        &config,
        SnapshotScope::AllUserBranches,
        remote_effects,
        touched_remote,
    )?;

    Ok(())
}

fn restore_sync_start_position(
    repo: &Repository,
    start_branch: Option<&str>,
    start_head: git2::Oid,
    json: bool,
) -> Result<()> {
    if !json {
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

    if !json {
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
        ensure_draft_prefix_for_gitlab, is_wip_or_draft_prefix, mismatched_pr_head_branch,
        replacement_closing_comment,
    };
    use crate::git;
    use crate::output::{
        SyncEntryResultJson, SyncMetadataJson, SyncResponse, SyncResultJson, OUTPUT_VERSION,
    };

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
            draft: false,
            approved: false,
            mergeable: true,
            changes_requested: false,
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
            draft: false,
            approved: false,
            mergeable: true,
            changes_requested: false,
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
        let response = SyncResponse {
            version: OUTPUT_VERSION,
            sync: SyncResultJson {
                stack: "test-stack".to_string(),
                base: "main".to_string(),
                rebased_before_sync: false,
                warnings: vec![],
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
        assert!(parsed["sync"]["entries"].is_array());

        let entry = &parsed["sync"]["entries"][0];
        assert_eq!(entry["position"], 1);
        assert_eq!(entry["action"], "created");
        assert_eq!(entry["pr_number"], 42);
        assert_eq!(entry["pushed"], true);
        assert!(entry["error"].is_null());
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
            oid: git2::Oid::zero(),
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
