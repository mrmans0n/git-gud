//! `gg sync` - Sync stack with remote provider (push branches and create/update PRs/MRs)

use console::style;
use dialoguer::Confirm;
use git2::Repository;
use indicatif::{ProgressBar, ProgressStyle};

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git::{
    self, generate_gg_id, get_commit_description, set_gg_id_in_message, strip_gg_id_from_message,
};
use crate::output::{
    print_json, SyncEntryResultJson, SyncResponse, SyncResultJson, OUTPUT_VERSION,
};
use crate::provider::Provider;
use crate::stack::{resolve_target, Stack};
use crate::template::{self, TemplateContext};

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

    let behind =
        match git::count_commits_behind(repo, base_branch, &format!("origin/{}", base_branch)) {
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

    if !json {
        println!(
            "{} Your stack is {} commits behind origin/{}. {} may show unrelated changes. Run 'gg rebase' first to update.",
            style("⚠").yellow().bold(),
            behind,
            base_branch,
            prs_label
        );
    }

    if config.get_sync_auto_rebase() {
        crate::commands::rebase::run_with_repo(repo, None, json)?;
        return Ok(true);
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
        crate::commands::rebase::run_with_repo(repo, None, json)?;
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

/// Run the sync command
pub fn run(
    draft: bool,
    json: bool,
    no_rebase_check: bool,
    force: bool,
    update_descriptions: bool,
    run_lint: bool,
    until: Option<String>,
) -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "sync")?;

    let git_dir = repo.commondir();
    let mut config = Config::load(git_dir)?;

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
                    entries: vec![],
                },
            });
        } else {
            println!("{}", style("Stack is empty. Nothing to sync.").dim());
        }
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

    // Run lint ONCE if requested (before GG-ID addition loop)
    if run_lint {
        let end_pos = lint_end_pos.unwrap_or(initial_stack.len());
        if !json {
            println!("{}", console::style("Running lint before sync...").dim());
        }
        crate::commands::lint::run(Some(end_pos), json, false)?;
        if !json {
            println!();
        }
    }

    let mut warnings: Vec<String> = Vec::new();

    // Now handle GG-ID addition in a loop (lint may have changed commits)
    // This loop ensures the operation lock is held for the entire operation
    let stack = loop {
        let stack = Stack::load(&repo, &config)?;

        if stack.is_empty() {
            if json {
                print_json(&SyncResponse {
                    version: OUTPUT_VERSION,
                    sync: SyncResultJson {
                        stack: stack.name.clone(),
                        base: stack.base.clone(),
                        rebased_before_sync,
                        warnings: warnings.clone(),
                        entries: vec![],
                    },
                });
            } else {
                println!("{}", style("Stack is empty. Nothing to sync.").dim());
            }
            return Ok(());
        }

        // Re-validate --until against potentially updated stack
        if let Some(ref target) = until {
            resolve_target(&stack, target)?;
        }

        // Check for missing GG-IDs
        let missing_ids = stack.entries_needing_gg_ids();
        if missing_ids.is_empty() {
            // All commits have GG-IDs, proceed with sync
            break stack;
        }

        if !json {
            println!(
                "{} {} commits are missing GG-IDs:",
                style("→").cyan(),
                missing_ids.len()
            );
            for entry in &missing_ids {
                println!("  [{}] {} {}", entry.position, entry.short_sha, entry.title);
            }
        }

        // Check config for auto_add_gg_ids (default: true)
        let should_add = if config.defaults.auto_add_gg_ids || json {
            true
        } else {
            Confirm::new()
                .with_prompt("Add GG-IDs to these commits? (requires rebase)")
                .default(true)
                .interact()
                .unwrap_or(true)
        };

        if !should_add {
            return Err(GgError::Other(
                "Cannot sync without GG-IDs. Aborting.".to_string(),
            ));
        }

        let needs_stash = !git::is_working_directory_clean(&repo)?;
        if needs_stash {
            if !json {
                println!("{}", style("Auto-stashing uncommitted changes...").dim());
            }
            git::run_git_command(&["stash", "push", "-m", "gg-sync-autostash"])?;
        }

        if let Err(err) = add_gg_ids_to_commits(&repo, &stack, json) {
            if needs_stash && !git::is_rebase_in_progress(&repo) {
                if !json {
                    println!(
                        "{}",
                        style("Attempting to restore stashed changes...").dim()
                    );
                }
                let _ = git::run_git_command(&["stash", "pop"]);
            }
            return Err(err);
        }

        // Check if rebase completed successfully
        if git::is_rebase_in_progress(&repo) {
            let note = if needs_stash {
                "\nNote: Your uncommitted changes are stashed and will be restored after the rebase completes."
            } else {
                ""
            };
            return Err(GgError::Other(format!(
                "Rebase in progress after adding GG-IDs.\n\
                 Please resolve any conflicts, then run:\n\
                 - 'git rebase --continue' (or 'gg continue') to finish the rebase\n\
                 - 'gg sync' again once the rebase is complete{}",
                note
            )));
        }

        if needs_stash {
            if !json {
                println!("{}", style("Restoring stashed changes...").dim());
            }
            match git::run_git_command(&["stash", "pop"]) {
                Ok(_) => {
                    if !json {
                        println!("{}", style("Changes restored").cyan());
                    }
                }
                Err(e) => {
                    let warning = format!(
                        "Could not restore stashed changes: {}. Your changes are in the stash. Run 'git stash pop' manually.",
                        e
                    );
                    if json {
                        warnings.push(warning);
                    } else {
                        println!("{} {}", style("Warning:").yellow(), warning);
                    }
                }
            }
        }

        if !json {
            println!(
                "{}",
                console::style("GG-IDs added successfully. Re-syncing...").dim()
            );
        }

        // Loop continues: reload stack and check for any remaining missing GG-IDs
        // (or proceed to sync if all commits now have GG-IDs)
    };

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
            let push_result = git::push_branch(&entry_branch, true, force);
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
                    });
                    continue;
                }

                format_push_error(&e, &entry_branch);
                return Err(e);
            }
        }

        // Determine target branch for MR
        let target_branch = if i == 0 {
            // First commit targets base branch
            stack.base.clone()
        } else {
            // Subsequent commits target previous entry's branch
            let prev_entry = &stack.entries[i - 1];
            stack.entry_branch_name(prev_entry).unwrap()
        };

        // Create or update PR
        let existing_pr = config.get_mr_for_entry(&stack.name, gg_id);

        match existing_pr {
            Some(pr_num) => {
                pr_number = Some(pr_num);
                // Check if PR is still open before updating
                let pr_info = provider.get_pr_info(pr_num).ok();
                pr_url = pr_info.as_ref().map(|info| info.url.clone());
                let is_closed = pr_info
                    .as_ref()
                    .map(|info| {
                        matches!(
                            info.state,
                            crate::provider::PrState::Merged | crate::provider::PrState::Closed
                        )
                    })
                    .unwrap_or(false);

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
                } else {
                    // If we're forcing draft on GitLab, we may need to update the title
                    // even if --update-descriptions wasn't provided.
                    if update_descriptions || (entry_draft && matches!(provider, Provider::GitLab))
                    {
                        // For GitLab with draft=true, ensure the title has "Draft: " prefix
                        let final_title =
                            ensure_draft_prefix_for_gitlab(&title, &provider, entry_draft);

                        if let Err(e) = provider.update_pr_title(pr_num, &final_title) {
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
                        if update_descriptions {
                            if let Err(e) = provider.update_pr_description(pr_num, &description) {
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

                    // Best-effort: if we want draft and the existing PR isn't a draft (GitHub only),
                    // convert it to draft.
                    if entry_draft && matches!(provider, Provider::GitHub) {
                        if let Some(info) = pr_info.as_ref() {
                            if !info.draft {
                                if let Err(e) = crate::gh::convert_pr_to_draft(pr_num) {
                                    if !json {
                                        pb.println(format!(
                                            "{} Could not convert {} {}{} to draft: {}",
                                            style("Warning:").yellow(),
                                            provider.pr_label(),
                                            provider.pr_number_prefix(),
                                            pr_num,
                                            e
                                        ));
                                    }
                                    if entry_error.is_none() {
                                        entry_error =
                                            Some(format!("Could not convert to draft: {e}"));
                                    }
                                }
                            }
                        }
                    }
                    // Update PR/MR base if needed
                    if let Err(e) = provider.update_pr_base(pr_num, &target_branch) {
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
                    if needs_push
                        || update_descriptions
                        || (entry_draft && matches!(provider, Provider::GitLab))
                    {
                        action = "updated".to_string();
                    }
                }
            }
            None => {
                // Create new PR/MR
                match provider.create_pr(
                    &entry_branch,
                    &target_branch,
                    &title,
                    &description,
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
            });
        }

        pb.inc(1);
    }

    if !json {
        pb.finish_with_message("Done!");
    }

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

/// Ensure a title has the "Draft: " prefix for GitLab when draft is true.
/// GitLab controls draft state via the title prefix, so when syncing with --draft,
/// we need to ensure the title has the "Draft: " prefix.
/// This function only adds the prefix if:
/// - The provider is GitLab
/// - is_draft is true
/// - The title doesn't already have the prefix (case-insensitive check)
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

/// Add GG-IDs to commits that are missing them by rewriting commit messages
/// This preserves the exact tree (including any lint changes) while only updating messages
fn add_gg_ids_to_commits(repo: &Repository, stack: &Stack, json: bool) -> Result<()> {
    if !json {
        println!("{}", style("Adding GG-IDs...").dim());
    }

    let base_ref = repo
        .revparse_single(&stack.base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", stack.base)))?;
    let base_commit = repo.find_commit(base_ref.id())?;

    let mut parent_oid = base_commit.id();

    // Walk through all entries in order, rewriting each commit
    for entry in &stack.entries {
        let original_commit = repo.find_commit(entry.oid)?;
        let original_message = original_commit.message().unwrap_or("");

        // Determine if we need a new GG-ID for this commit
        let new_message = if entry.gg_id.is_none() {
            let new_id = generate_gg_id();
            set_gg_id_in_message(original_message, &new_id)
        } else {
            // Even if this commit already has a GG-ID, we still need to rewrite it
            // because the parent has changed (due to previous rewrites in the stack)
            original_message.to_string()
        };

        // Create a new commit with the same tree but updated parent and message
        let new_oid = repo.commit(
            None, // Don't update any reference yet
            &original_commit.author(),
            &original_commit.committer(),
            &new_message,
            &original_commit.tree()?,
            &[&repo.find_commit(parent_oid)?],
        )?;

        // This new commit becomes the parent for the next one
        parent_oid = new_oid;
    }

    // Update the current branch to point to the last rewritten commit
    let head = repo.head()?;
    if let Some(branch_name) = head.shorthand() {
        // Update the branch reference
        repo.reference(
            &format!("refs/heads/{}", branch_name),
            parent_oid,
            true,
            "gg sync: added GG-IDs",
        )?;
    } else {
        return Err(GgError::Other(
            "Cannot add GG-IDs: HEAD is detached".to_string(),
        ));
    }

    if !json {
        println!("{} Added GG-IDs to commits", style("OK").green().bold());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_pr_payload, clean_title, ensure_draft_prefix_for_gitlab, is_wip_or_draft_prefix,
    };
    use crate::git;
    use crate::output::{SyncEntryResultJson, SyncResponse, SyncResultJson, OUTPUT_VERSION};

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
    fn test_sync_json_response_structure() {
        let response = SyncResponse {
            version: OUTPUT_VERSION,
            sync: SyncResultJson {
                stack: "test-stack".to_string(),
                base: "main".to_string(),
                rebased_before_sync: false,
                warnings: vec![],
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
}
