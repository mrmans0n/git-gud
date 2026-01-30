//! `gg sync` - Sync stack with GitLab (push branches and create/update MRs)

use console::style;
use dialoguer::Confirm;
use git2::Repository;
use indicatif::{ProgressBar, ProgressStyle};

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git::{self, generate_gg_id, get_gg_id, set_gg_id_in_message, strip_gg_id_from_message};
use crate::glab;
use crate::stack::Stack;

/// Run the sync command
pub fn run(draft: bool, force: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let mut config = Config::load(git_dir)?;

    // Check glab is available
    glab::check_glab_installed()?;
    glab::check_glab_auth()?;

    // Load current stack
    let stack = Stack::load(&repo, &config)?;

    if stack.is_empty() {
        println!("{}", style("Stack is empty. Nothing to sync.").dim());
        return Ok(());
    }

    // Check for missing GG-IDs
    let missing_ids = stack.entries_needing_gg_ids();
    if !missing_ids.is_empty() {
        println!(
            "{} {} commits are missing GG-IDs:",
            style("Warning:").yellow().bold(),
            missing_ids.len()
        );
        for entry in &missing_ids {
            println!("  [{}] {} {}", entry.position, entry.short_sha, entry.title);
        }

        let confirm = Confirm::new()
            .with_prompt("Add GG-IDs to these commits? (requires rebase)")
            .default(true)
            .interact()
            .unwrap_or(false);

        if confirm {
            add_gg_ids_to_commits(&repo, &stack)?;
            // Reload stack after rebase
            return run(draft, force);
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

        pb.set_message(format!("Processing {}...", entry.short_sha));

        // Create/update the remote branch for this commit
        create_entry_branch(&repo, &stack, entry, &entry_branch)?;

        // Push the branch
        let push_result = git::push_branch(&entry_branch, force);
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

        // Create or update MR
        let existing_mr = config.get_mr_for_entry(&stack.name, gg_id);

        match existing_mr {
            Some(mr_num) => {
                // Update MR target if needed
                if let Err(e) = glab::update_mr_target(mr_num, &target_branch) {
                    pb.println(format!(
                        "{} Could not update MR !{}: {}",
                        style("Warning:").yellow(),
                        mr_num,
                        e
                    ));
                }
                pb.println(format!(
                    "{} Force-pushed {} -> MR !{}",
                    style("OK").green().bold(),
                    style(&entry_branch).cyan(),
                    mr_num
                ));
            }
            None => {
                // Create new MR
                let title = strip_gg_id_from_message(&entry.title);
                let description = format!(
                    "Part of stack `{}`\n\nCommit: {}",
                    stack.name, entry.short_sha
                );

                match glab::create_mr(&entry_branch, &target_branch, &title, &description, draft) {
                    Ok(mr_num) => {
                        config.set_mr_for_entry(&stack.name, gg_id, mr_num);
                        let draft_label = if draft { " (draft)" } else { "" };
                        pb.println(format!(
                            "{} Pushed {} -> MR !{}{}",
                            style("OK").green().bold(),
                            style(&entry_branch).cyan(),
                            mr_num,
                            draft_label
                        ));
                    }
                    Err(e) => {
                        pb.println(format!(
                            "{} Failed to create MR for {}: {}",
                            style("Error:").red().bold(),
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
