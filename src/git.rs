//! Git operations for git-gud
//!
//! Provides utilities for repository discovery, branch management,
//! commit traversal, and rebase operations.

use std::process::Command;

#[allow(unused_imports)]
use git2::Branch;
use git2::{BranchType, Commit, Oid, Repository, Signature, Sort};
use regex::Regex;

use crate::error::{GgError, Result};

/// Prefix for GG-ID trailers in commit messages
pub const GG_ID_PREFIX: &str = "GG-ID:";

/// Open the repository at the current directory or its parents
pub fn open_repo() -> Result<Repository> {
    Repository::discover(".").map_err(|_| GgError::NotInRepo)
}

/// Find the base branch (main, master, or trunk)
pub fn find_base_branch(repo: &Repository) -> Result<String> {
    for branch_name in &["main", "master", "trunk"] {
        if repo.find_branch(branch_name, BranchType::Local).is_ok() {
            return Ok(branch_name.to_string());
        }
        // Also check remote branches
        let remote_name = format!("origin/{}", branch_name);
        if repo
            .find_reference(&format!("refs/remotes/{}", remote_name))
            .is_ok()
        {
            return Ok(branch_name.to_string());
        }
    }
    Err(GgError::NoBaseBranch)
}

/// Get the current branch name, if on a branch
pub fn current_branch_name(repo: &Repository) -> Option<String> {
    let head = repo.head().ok()?;
    if head.is_branch() {
        head.shorthand().map(|s| s.to_string())
    } else {
        None
    }
}

/// Parse a stack branch name into (username, stack_name)
pub fn parse_stack_branch(branch_name: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = branch_name.split('/').collect();
    if parts.len() == 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

/// Parse an entry branch name into (username, stack_name, entry_id)
/// Format: username/stack_name--entry_id
pub fn parse_entry_branch(branch_name: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = branch_name.split('/').collect();
    if parts.len() == 2 && parts[1].contains("--") {
        let stack_parts: Vec<&str> = parts[1].split("--").collect();
        if stack_parts.len() == 2 {
            Some((
                parts[0].to_string(),
                stack_parts[0].to_string(),
                stack_parts[1].to_string(),
            ))
        } else {
            None
        }
    } else {
        None
    }
}

/// Format a stack branch name
pub fn format_stack_branch(username: &str, stack_name: &str) -> String {
    format!("{}/{}", username, stack_name)
}

/// Format a remote branch name for a specific entry
pub fn format_entry_branch(username: &str, stack_name: &str, entry_id: &str) -> String {
    format!("{}/{}--{}", username, stack_name, entry_id)
}

/// Find the first entry branch for a stack (username/stack_name--*)
pub fn find_entry_branch_for_stack(
    repo: &Repository,
    username: &str,
    stack_name: &str,
) -> Option<String> {
    let branches = repo.branches(Some(BranchType::Local)).ok()?;
    for (branch, _) in branches.flatten() {
        if let Ok(Some(name)) = branch.name() {
            if let Some((branch_user, branch_stack, _entry_id)) = parse_entry_branch(name) {
                if branch_user == username && branch_stack == stack_name {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Check if the working directory is clean
pub fn is_working_directory_clean(repo: &Repository) -> Result<bool> {
    let statuses = repo.statuses(None)?;
    Ok(statuses.is_empty())
}

/// Require a clean working directory, returning error if dirty
pub fn require_clean_working_directory(repo: &Repository) -> Result<()> {
    if !is_working_directory_clean(repo)? {
        return Err(GgError::DirtyWorkingDirectory);
    }
    Ok(())
}

/// Get all commits between base and stack tip (in order from base to tip)
/// Returns commit OIDs rather than Commit objects to avoid lifetime issues
/// If `stack_branch` is provided, use that branch instead of HEAD (for detached HEAD mode)
pub fn get_stack_commit_oids(
    repo: &Repository,
    base_branch: &str,
    stack_branch: Option<&str>,
) -> Result<Vec<Oid>> {
    // Get the tip of the stack - either from a branch or from HEAD
    let tip_oid = if let Some(branch) = stack_branch {
        let branch_ref = format!("refs/heads/{}", branch);
        repo.revparse_single(&branch_ref)?.id()
    } else {
        repo.head()?.peel_to_commit()?.id()
    };

    let base_ref = repo
        .revparse_single(base_branch)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", base_branch)))
        .map_err(|_| GgError::NoBaseBranch)?;

    let base_oid = base_ref.id();

    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
    revwalk.push(tip_oid)?;
    revwalk.hide(base_oid)?;

    let mut oids = Vec::new();
    for oid_result in revwalk {
        let oid = oid_result?;
        let commit = repo.find_commit(oid)?;

        // Check for merge commits
        if commit.parent_count() > 1 {
            return Err(GgError::MergeCommitInStack);
        }

        oids.push(oid);
    }

    Ok(oids)
}

/// Extract the GG-ID from a commit message (case-insensitive)
pub fn get_gg_id(commit: &Commit) -> Option<String> {
    let message = commit.message()?;
    let re = Regex::new(r"(?i)^GG-ID:\s*(.+)$").ok()?;

    for line in message.lines() {
        if let Some(captures) = re.captures(line.trim()) {
            return captures.get(1).map(|m| m.as_str().trim().to_string());
        }
    }
    None
}

/// Generate a new GG-ID
pub fn generate_gg_id() -> String {
    format!("c-{}", &uuid::Uuid::new_v4().to_string()[..7])
}

/// Add or update GG-ID trailer in a message
pub fn set_gg_id_in_message(message: &str, gg_id: &str) -> String {
    let re = Regex::new(r"(?im)^GG-ID:\s*.+$").unwrap();

    if re.is_match(message) {
        // Replace existing GG-ID
        re.replace(message, format!("{} {}", GG_ID_PREFIX, gg_id))
            .to_string()
    } else {
        // Append GG-ID trailer
        let trimmed = message.trim_end();
        format!("{}\n\n{} {}", trimmed, GG_ID_PREFIX, gg_id)
    }
}

/// Strip GG-ID trailer from a message (for MR titles/descriptions)
pub fn strip_gg_id_from_message(message: &str) -> String {
    let re = Regex::new(r"(?im)^GG-ID:\s*.+\n?").unwrap();
    let result = re.replace_all(message, "");
    result.trim_end().to_string()
}

/// Get the commit message title (first line)
pub fn get_commit_title(commit: &Commit) -> String {
    commit.summary().unwrap_or("<no summary>").to_string()
}

/// Checkout a branch by name
pub fn checkout_branch(repo: &Repository, branch_name: &str) -> Result<()> {
    let refname = format!("refs/heads/{}", branch_name);
    let obj = repo.revparse_single(&refname)?;

    repo.checkout_tree(&obj, None)?;
    repo.set_head(&refname)?;
    Ok(())
}

/// Checkout a specific commit (detached HEAD)
pub fn checkout_commit(repo: &Repository, commit: &Commit) -> Result<()> {
    let obj = commit.as_object();
    repo.checkout_tree(obj, None)?;
    repo.set_head_detached(commit.id())?;
    Ok(())
}

/// Get the repository signature
pub fn get_signature(repo: &Repository) -> Result<Signature<'static>> {
    repo.signature().map_err(GgError::Git)
}

/// Run git command as subprocess (for operations git2 doesn't support well)
pub fn run_git_command(args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(GgError::Other(format!(
            "git {} failed: {}",
            args.join(" "),
            stderr
        )))
    }
}

/// Push a branch to origin
pub fn push_branch(branch_name: &str, force: bool) -> Result<()> {
    let mut args = vec!["push", "origin", branch_name];
    if force {
        args.insert(1, "--force-with-lease");
    }
    run_git_command(&args)?;
    Ok(())
}

/// Delete a remote branch
pub fn delete_remote_branch(branch_name: &str) -> Result<()> {
    run_git_command(&["push", "origin", "--delete", branch_name])?;
    Ok(())
}

/// Continue a rebase
pub fn rebase_continue() -> Result<()> {
    run_git_command(&["rebase", "--continue"])?;
    Ok(())
}

/// Abort a rebase
pub fn rebase_abort() -> Result<()> {
    run_git_command(&["rebase", "--abort"])?;
    Ok(())
}

/// Check if a rebase is in progress
pub fn is_rebase_in_progress(repo: &Repository) -> bool {
    let git_dir = repo.path();
    git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists()
}

/// Get short SHA from a commit
pub fn short_sha(commit: &Commit) -> String {
    commit.id().to_string()[..7].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_stack_branch() {
        assert_eq!(
            parse_stack_branch("nacho/my-feature"),
            Some(("nacho".to_string(), "my-feature".to_string()))
        );
        assert_eq!(parse_stack_branch("main"), None);
        assert_eq!(parse_stack_branch("nacho/my-feature/c-abc123"), None);
    }

    #[test]
    fn test_format_entry_branch() {
        assert_eq!(
            format_entry_branch("nacho", "my-feature", "c-abc123"),
            "nacho/my-feature--c-abc123"
        );
    }

    #[test]
    fn test_parse_entry_branch() {
        // Format: username/stack_name--entry_id
        assert_eq!(
            parse_entry_branch("nacho/my-feature--c-abc123"),
            Some((
                "nacho".to_string(),
                "my-feature".to_string(),
                "c-abc123".to_string()
            ))
        );
        assert_eq!(parse_entry_branch("main"), None);
        assert_eq!(parse_entry_branch("nacho/my-feature"), None);
        // Old format with slash should not match
        assert_eq!(parse_entry_branch("nacho/my-feature/c-abc123"), None);
    }

    #[test]
    fn test_set_gg_id_in_message() {
        let msg = "Add feature\n\nSome description";
        let result = set_gg_id_in_message(msg, "c-abc123");
        assert!(result.contains("GG-ID: c-abc123"));

        // Test replacement
        let msg_with_id = "Add feature\n\nGG-ID: c-old123";
        let result = set_gg_id_in_message(msg_with_id, "c-new456");
        assert!(result.contains("GG-ID: c-new456"));
        assert!(!result.contains("c-old123"));
    }

    #[test]
    fn test_strip_gg_id_from_message() {
        let msg = "Add feature\n\nSome description\n\nGG-ID: c-abc123";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.contains("GG-ID"));
        assert!(result.contains("Add feature"));
        assert!(result.contains("Some description"));
    }

    #[test]
    fn test_generate_gg_id() {
        let id = generate_gg_id();
        assert!(id.starts_with("c-"));
        assert_eq!(id.len(), 9); // "c-" + 7 chars
    }
}

/// Remote provider type
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum RemoteProvider {
    GitHub,
    GitLab,
}

/// Detect the remote provider based on the remote URL
#[allow(dead_code)]
pub fn detect_remote_provider(repo: &Repository) -> Result<RemoteProvider> {
    let remote = repo
        .find_remote("origin")
        .map_err(|_| GgError::Other("No origin remote found".to_string()))?;

    let url = remote
        .url()
        .ok_or_else(|| GgError::Other("Origin remote has no URL".to_string()))?;

    if url.contains("github.com") {
        Ok(RemoteProvider::GitHub)
    } else if url.contains("gitlab.com") {
        Ok(RemoteProvider::GitLab)
    } else {
        // Default to GitLab for self-hosted instances
        Err(GgError::Other(format!(
            "Could not detect remote provider from URL: {}. Supported: github.com, gitlab.com",
            url
        )))
    }
}
