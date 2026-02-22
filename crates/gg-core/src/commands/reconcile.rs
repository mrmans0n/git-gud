//! `gg reconcile` - Reconcile stacks that were pushed without using `gg sync`
//!
//! This command helps recover stacks that got out of sync, such as when
//! someone does `gg co mystack` → commit → `git push` (skipping `gg sync`).
//!
//! It will:
//! 1. Add GG-IDs to commits that don't have them (via rebase)
//! 2. Search for existing PRs/MRs for the stack's entry branches and map them

use console::style;
use dialoguer::Confirm;
use git2::Repository;

use crate::config::Config;
use crate::error::Result;
use crate::git::{self, generate_gg_id, get_gg_id, set_gg_id_in_message};
use crate::provider::Provider;
use crate::stack::Stack;

/// Actions that reconcile would perform
#[derive(Debug)]
struct ReconcileActions {
    /// Commits that need GG-IDs added
    commits_needing_ids: Vec<CommitInfo>,
    /// Entry branches that have PRs/MRs to map
    prs_to_map: Vec<PrMapping>,
}

#[derive(Debug)]
struct CommitInfo {
    short_sha: String,
    title: String,
}

#[derive(Debug)]
struct PrMapping {
    gg_id: String,
    branch: String,
    pr_number: u64,
}

impl ReconcileActions {
    fn is_empty(&self) -> bool {
        self.commits_needing_ids.is_empty() && self.prs_to_map.is_empty()
    }
}

/// Run the reconcile command
pub fn run(dry_run: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.commondir();
    let mut config = Config::load(git_dir)?;

    // Detect provider
    let provider = Provider::detect(&repo)?;

    // Only check installed/auth if not doing a dry run
    // (dry run can show what would be done without actual provider access)
    if !dry_run {
        provider.check_installed()?;
        provider.check_auth()?;
    }

    // Load current stack
    let stack = Stack::load(&repo, &config)?;

    if stack.is_empty() {
        println!("{}", style("Stack is empty. Nothing to reconcile.").dim());
        return Ok(());
    }

    println!(
        "{} Analyzing stack {} ({} commits)...",
        style("→").cyan(),
        style(&stack.name).bold(),
        stack.len()
    );

    // Phase 1: Find commits needing GG-IDs
    let commits_needing_ids: Vec<CommitInfo> = stack
        .entries
        .iter()
        .filter(|e| e.gg_id.is_none())
        .map(|e| CommitInfo {
            short_sha: e.short_sha.clone(),
            title: e.title.clone(),
        })
        .collect();

    // Phase 2: Find PRs/MRs for entry branches that aren't mapped
    // In dry-run mode, try to find PRs but don't fail if provider isn't available
    let prs_to_map = if dry_run {
        // Check if provider is available before trying to list PRs
        if provider.check_installed().is_ok() && provider.check_auth().is_ok() {
            find_unmapped_prs(&repo, &stack, &config, &provider)?
        } else {
            println!(
                "{}",
                style("  (Skipping PR/MR discovery - provider not authenticated)").dim()
            );
            Vec::new()
        }
    } else {
        find_unmapped_prs(&repo, &stack, &config, &provider)?
    };

    let actions = ReconcileActions {
        commits_needing_ids,
        prs_to_map,
    };

    // Display what would be done
    display_actions(&actions, &provider);

    if actions.is_empty() {
        println!(
            "\n{} Stack is already reconciled. Nothing to do.",
            style("✓").green().bold()
        );
        return Ok(());
    }

    if dry_run {
        println!("\n{} Dry run complete. No changes made.", style("→").cyan());
        return Ok(());
    }

    // Confirm before proceeding
    if !actions.commits_needing_ids.is_empty() {
        let should_add_ids = Confirm::new()
            .with_prompt("Add GG-IDs to commits? (requires rebase)")
            .default(true)
            .interact()
            .unwrap_or(false);

        if should_add_ids {
            add_gg_ids_to_commits(&repo, &stack)?;
            // Reload stack after rebase to get updated GG-IDs
            let stack = Stack::load(&repo, &config)?;
            // Re-search for PRs with the new stack
            let prs_to_map = find_unmapped_prs(&repo, &stack, &config, &provider)?;
            map_prs(&mut config, &stack.name, &prs_to_map, &provider)?;
        } else {
            println!("{}", style("Skipping GG-ID addition.").dim());
        }
    } else {
        // Just map the PRs
        map_prs(&mut config, &stack.name, &actions.prs_to_map, &provider)?;
    }

    // Save updated config
    config.save(git_dir)?;

    println!("\n{} Reconciliation complete!", style("OK").green().bold());

    Ok(())
}

/// Find PRs/MRs for entry branches that aren't mapped in config
fn find_unmapped_prs(
    _repo: &Repository,
    stack: &Stack,
    config: &Config,
    provider: &Provider,
) -> Result<Vec<PrMapping>> {
    let mut mappings = Vec::new();

    for entry in &stack.entries {
        // Skip entries without GG-ID (they need IDs first)
        let gg_id = match &entry.gg_id {
            Some(id) => id,
            None => continue,
        };

        // Skip entries that already have a mapped PR/MR
        if config.get_mr_for_entry(&stack.name, gg_id).is_some() {
            continue;
        }

        // Get the entry branch name
        let entry_branch = match stack.entry_branch_name(entry) {
            Some(branch) => branch,
            None => continue,
        };

        // Search for PRs/MRs with this branch
        match provider.list_prs_for_branch(&entry_branch) {
            Ok(prs) if !prs.is_empty() => {
                // Use the first (most recent) PR
                // Could be enhanced to prompt user if multiple
                let pr_number = prs[0];
                mappings.push(PrMapping {
                    gg_id: gg_id.clone(),
                    branch: entry_branch,
                    pr_number,
                });
            }
            Ok(_) => {
                // No PRs found for this branch - that's fine
            }
            Err(e) => {
                // Log warning but continue
                eprintln!(
                    "{} Could not search {}s for {}: {}",
                    style("Warning:").yellow(),
                    provider.pr_label(),
                    entry_branch,
                    e
                );
            }
        }
    }

    Ok(mappings)
}

/// Display what actions would be performed
fn display_actions(actions: &ReconcileActions, provider: &Provider) {
    if !actions.commits_needing_ids.is_empty() {
        println!(
            "\n{} {} commits need GG-IDs:",
            style("→").cyan(),
            actions.commits_needing_ids.len()
        );
        for commit in &actions.commits_needing_ids {
            println!(
                "  {} {} {}",
                style("•").dim(),
                style(&commit.short_sha).yellow(),
                commit.title
            );
        }
    }

    if !actions.prs_to_map.is_empty() {
        println!(
            "\n{} {} existing {}s found to map:",
            style("→").cyan(),
            actions.prs_to_map.len(),
            provider.pr_label()
        );
        for mapping in &actions.prs_to_map {
            println!(
                "  {} {} → {} {}{}",
                style("•").dim(),
                style(&mapping.branch).cyan(),
                provider.pr_label(),
                provider.pr_number_prefix(),
                mapping.pr_number
            );
        }
    }
}

/// Map PRs/MRs to entries in config
fn map_prs(
    config: &mut Config,
    stack_name: &str,
    mappings: &[PrMapping],
    provider: &Provider,
) -> Result<()> {
    for mapping in mappings {
        config.set_mr_for_entry(stack_name, &mapping.gg_id, mapping.pr_number);
        println!(
            "{} Mapped {} → {} {}{}",
            style("OK").green().bold(),
            style(&mapping.gg_id).cyan(),
            provider.pr_label(),
            provider.pr_number_prefix(),
            mapping.pr_number
        );
    }
    Ok(())
}

/// Add GG-IDs to commits that are missing them via interactive rebase
fn add_gg_ids_to_commits(repo: &Repository, stack: &Stack) -> Result<()> {
    println!("{}", style("Adding GG-IDs via rebase...").dim());

    let base_ref = repo
        .revparse_single(&stack.base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", stack.base)))?;

    use git2::RebaseOptions;

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
    use super::*;

    #[test]
    fn test_reconcile_actions_is_empty() {
        let actions = ReconcileActions {
            commits_needing_ids: vec![],
            prs_to_map: vec![],
        };
        assert!(actions.is_empty());
    }

    #[test]
    fn test_reconcile_actions_not_empty_with_commits() {
        let actions = ReconcileActions {
            commits_needing_ids: vec![CommitInfo {
                short_sha: "abc1234".to_string(),
                title: "Test commit".to_string(),
            }],
            prs_to_map: vec![],
        };
        assert!(!actions.is_empty());
    }

    #[test]
    fn test_reconcile_actions_not_empty_with_prs() {
        let actions = ReconcileActions {
            commits_needing_ids: vec![],
            prs_to_map: vec![PrMapping {
                gg_id: "c-abc1234".to_string(),
                branch: "user/stack--c-abc1234".to_string(),
                pr_number: 42,
            }],
        };
        assert!(!actions.is_empty());
    }

    #[test]
    fn test_commit_info_debug() {
        let info = CommitInfo {
            short_sha: "abc1234".to_string(),
            title: "Test commit".to_string(),
        };
        let debug_str = format!("{:?}", info);
        assert!(debug_str.contains("abc1234"));
        assert!(debug_str.contains("Test commit"));
    }

    #[test]
    fn test_pr_mapping_debug() {
        let mapping = PrMapping {
            gg_id: "c-abc1234".to_string(),
            branch: "user/stack--c-abc1234".to_string(),
            pr_number: 42,
        };
        let debug_str = format!("{:?}", mapping);
        assert!(debug_str.contains("c-abc1234"));
        assert!(debug_str.contains("42"));
    }
}
