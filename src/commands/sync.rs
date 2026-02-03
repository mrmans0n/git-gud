//! `gg sync` - Sync stack with remote provider (push branches and create/update PRs/MRs)

use console::style;
use dialoguer::Confirm;
use git2::Repository;
use indicatif::{ProgressBar, ProgressStyle};

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git::{
    self, generate_gg_id, get_commit_description, get_gg_id, set_gg_id_in_message,
    strip_gg_id_from_message,
};
use crate::provider::Provider;
use crate::stack::Stack;
use crate::template::{self, TemplateContext};

/// Run the sync command
pub fn run(draft: bool, force: bool, update_descriptions: bool) -> Result<()> {
    let repo = git::open_repo()?;

    // Acquire operation lock to prevent concurrent operations
    let _lock = git::acquire_operation_lock(&repo, "sync")?;

    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Detect and check provider
    let provider = Provider::detect(&repo)?;
    provider.check_installed()?;
    provider.check_auth()?;

    // Fetch from remote to ensure we have up-to-date refs
    // This prevents "stale info" errors when remote branches were deleted (e.g., after merge)
    let _ = git::fetch_and_prune();

    // Load current stack
    let stack = Stack::load(&repo, &config)?;

    // Load optional PR template
    let pr_template = template::load_template(git_dir);

    if stack.is_empty() {
        println!("{}", style("Stack is empty. Nothing to sync.").dim());
        return Ok(());
    }

    // Check for missing GG-IDs
    let missing_ids = stack.entries_needing_gg_ids();
    if !missing_ids.is_empty() {
        println!(
            "{} {} commits are missing GG-IDs:",
            style("→").cyan(),
            missing_ids.len()
        );
        for entry in &missing_ids {
            println!("  [{}] {} {}", entry.position, entry.short_sha, entry.title);
        }

        // Check config for auto_add_gg_ids (default: true)
        let should_add = if config.defaults.auto_add_gg_ids {
            true
        } else {
            Confirm::new()
                .with_prompt("Add GG-IDs to these commits? (requires rebase)")
                .default(true)
                .interact()
                .unwrap_or(true)
        };

        if should_add {
            add_gg_ids_to_commits(&repo, &stack)?;

            // Check if rebase completed successfully
            // SAFETY: Prevent recursive call if rebase is still in progress
            if git::is_rebase_in_progress(&repo) {
                return Err(GgError::Other(
                    "Rebase in progress after adding GG-IDs.\n\
                     Please resolve any conflicts, then run:\n\
                     - 'git rebase --continue' (or 'gg continue') to finish the rebase\n\
                     - 'gg sync' again once the rebase is complete"
                        .to_string(),
                ));
            }

            println!(
                "{}",
                console::style("GG-IDs added successfully. Re-syncing...").dim()
            );

            // Reload stack after rebase and continue
            return run(draft, force, update_descriptions);
        } else {
            return Err(GgError::Other(
                "Cannot sync without GG-IDs. Aborting.".to_string(),
            ));
        }
    }

    // Sync progress
    let pb = ProgressBar::new(stack.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("=>-"),
    );

    // Process each entry
    for (i, entry) in stack.entries.iter().enumerate() {
        let gg_id = entry.gg_id.as_ref().unwrap();
        let entry_branch = stack.entry_branch_name(entry).unwrap();
        let commit = repo.find_commit(entry.oid)?;
        let raw_title = strip_gg_id_from_message(&entry.title);
        let title = clean_title(&raw_title);
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

        // Push the branch (always force-push with lease because rebases change commit SHAs)
        // This is safe because each entry branch is owned by this stack
        // If --force is passed, use hard force as an escape hatch
        let push_result = git::push_branch(&entry_branch, true, force);
        if let Err(e) = push_result {
            pb.abandon_with_message(format!("Failed to push {}: {}", entry_branch, e));
            return Err(e);
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
                // Check if PR is still open before updating
                let pr_info = provider.get_pr_info(pr_num).ok();
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
                    // Skip updating closed/merged PRs
                    pb.println(format!(
                        "{} {} {}{} already closed/merged, skipping",
                        style("○").dim(),
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num
                    ));
                } else {
                    if update_descriptions {
                        if let Err(e) = provider.update_pr_title(pr_num, &title) {
                            pb.println(format!(
                                "{} Could not update {} {}{} title: {}",
                                style("Warning:").yellow(),
                                provider.pr_label(),
                                provider.pr_number_prefix(),
                                pr_num,
                                e
                            ));
                        }
                        if let Err(e) = provider.update_pr_description(pr_num, &description) {
                            pb.println(format!(
                                "{} Could not update {} {}{} description: {}",
                                style("Warning:").yellow(),
                                provider.pr_label(),
                                provider.pr_number_prefix(),
                                pr_num,
                                e
                            ));
                        }
                    }
                    // Update PR/MR base if needed
                    if let Err(e) = provider.update_pr_base(pr_num, &target_branch) {
                        pb.println(format!(
                            "{} Could not update {} {}{}: {}",
                            style("Warning:").yellow(),
                            provider.pr_label(),
                            provider.pr_number_prefix(),
                            pr_num,
                            e
                        ));
                    }
                    pb.println(format!(
                        "{} Force-pushed {} -> {} {}{}",
                        style("OK").green().bold(),
                        style(&entry_branch).cyan(),
                        provider.pr_label(),
                        provider.pr_number_prefix(),
                        pr_num
                    ));
                }
            }
            None => {
                // Create new PR/MR
                match provider.create_pr(&entry_branch, &target_branch, &title, &description, draft)
                {
                    Ok(result) => {
                        config.set_mr_for_entry(&stack.name, gg_id, result.number);
                        let draft_label = if draft { " (draft)" } else { "" };
                        pb.println(format!(
                            "{} Pushed {} -> {} {}{}{}",
                            style("OK").green().bold(),
                            style(&entry_branch).cyan(),
                            provider.pr_label(),
                            provider.pr_number_prefix(),
                            result.number,
                            draft_label
                        ));
                        // Show clickable URL for new PRs/MRs
                        if !result.url.is_empty() {
                            pb.println(format!("   {}", style(&result.url).underlined().blue()));
                        }
                    }
                    Err(e) => {
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

        pb.inc(1);
    }

    pb.finish_with_message("Done!");

    // Save updated config
    config.save(git_dir)?;

    println!();
    println!(
        "{} Synced {} commits",
        style("OK").green().bold(),
        stack.len()
    );

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

fn clean_title(title: &str) -> String {
    let trimmed = title.trim();
    trimmed.strip_suffix('.').unwrap_or(trimmed).to_string()
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

/// Add GG-IDs to commits that are missing them via interactive rebase
fn add_gg_ids_to_commits(repo: &Repository, stack: &Stack) -> Result<()> {
    println!("{}", style("Adding GG-IDs via rebase...").dim());

    // We need to do a rebase to add GG-IDs to commits
    // For simplicity, we'll use git command

    let base_ref = repo
        .revparse_single(&stack.base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", stack.base)))?;

    // Create a temporary script to add GG-IDs
    // We'll use git filter-branch or git rebase with exec

    // First, let's use git rebase --exec approach
    let mut script_commands = Vec::new();

    for entry in &stack.entries {
        if entry.gg_id.is_none() {
            let new_id = generate_gg_id();
            // We'll need to amend the commit message
            script_commands.push(format!(
                "if [ \"$(git rev-parse HEAD)\" != \"{}\" ]; then true; else git commit --amend -m \"$(git log -1 --format='%B')\\n\\nGG-ID: {}\"; fi",
                entry.oid, new_id
            ));
        }
    }

    // Simple approach: use git rebase with environment variable for editor
    // Actually, let's do this programmatically with git2

    use git2::RebaseOptions;

    let _head = repo.head()?.peel_to_commit()?;
    let base_commit = repo.find_annotated_commit(base_ref.id())?;

    let mut rebase_opts = RebaseOptions::new();
    let mut rebase = repo.rebase(None, Some(&base_commit), None, Some(&mut rebase_opts))?;

    let sig = git::get_signature(repo)?;

    while let Some(op) = rebase.next() {
        let op = op?;
        let commit = repo.find_commit(op.id())?;

        // Check if this commit needs a GG-ID
        let needs_id = get_gg_id(&commit).is_none();

        let message = commit.message().unwrap_or("");
        let new_message = if needs_id {
            let new_id = generate_gg_id();
            set_gg_id_in_message(message, &new_id)
        } else {
            message.to_string()
        };

        rebase.commit(None, &sig, Some(&new_message))?;
    }

    rebase.finish(None)?;

    println!("{} Added GG-IDs to commits", style("OK").green().bold());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build_pr_payload, clean_title};

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
}
