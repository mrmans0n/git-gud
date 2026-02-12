//! Git operations for git-gud
//!
//! Provides utilities for repository discovery, branch management,
//! commit traversal, and rebase operations.

use std::fs::{self, File};
use std::process::Command;
use std::time::Duration;

use fs2::FileExt;
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

/// Operation lock handle that automatically releases on drop
pub struct OperationLock {
    _lock_file: File,
}

impl Drop for OperationLock {
    fn drop(&mut self) {
        // Lock is released when the file handle is dropped.
        // Do not delete the lock file to avoid races with new lock acquisitions.
    }
}

/// Acquire an exclusive operation lock to prevent concurrent gg operations
/// Returns a lock handle that will automatically release when dropped
pub fn acquire_operation_lock(repo: &Repository, operation: &str) -> Result<OperationLock> {
    let git_dir = repo.path();
    let gg_dir = git_dir.join("gg");

    // Ensure gg directory exists
    fs::create_dir_all(&gg_dir)?;

    let lock_path = gg_dir.join("operation.lock");

    // Try to open or create lock file
    let lock_file = File::create(&lock_path)?;

    // Try to acquire exclusive lock with timeout
    let timeout = Duration::from_secs(10);
    let start = std::time::Instant::now();

    loop {
        match lock_file.try_lock_exclusive() {
            Ok(()) => {
                // Successfully acquired lock
                // Write operation info for debugging
                let info = format!("operation: {}\npid: {}\n", operation, std::process::id());
                let _ = fs::write(&lock_path, info);

                return Ok(OperationLock {
                    _lock_file: lock_file,
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Lock is held by another process
                if start.elapsed() >= timeout {
                    // Try to read lock info for better error message
                    let lock_info = fs::read_to_string(&lock_path)
                        .unwrap_or_else(|_| "unknown operation".to_string());

                    return Err(GgError::Other(format!(
                        "Another gg operation is currently running.\n\
                         Lock info:\n{}\n\
                         If no other gg process is running, the lock may be stale.\n\
                         You can manually remove: {}",
                        lock_info.trim(),
                        lock_path.display()
                    )));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e.into()),
        }
    }
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
/// Note: Entry branches (username/stack--entry_id) should NOT be parsed as stack branches
pub fn parse_stack_branch(branch_name: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = branch_name.split('/').collect();
    if parts.len() == 2 {
        // Exclude entry branches which have "--" in the stack name portion
        if parts[1].contains("--") {
            return None;
        }
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

/// Check if a branch is the HEAD of any worktree
/// Returns the worktree name if the branch is checked out, None otherwise
pub fn is_branch_checked_out_in_worktree(repo: &Repository, branch_name: &str) -> Option<String> {
    if let Ok(worktrees) = repo.worktrees() {
        for name in worktrees.iter().flatten() {
            if let Ok(wt) = repo.find_worktree(name) {
                // Open repo from worktree to check its HEAD
                if let Ok(wt_repo) = Repository::open_from_worktree(&wt) {
                    if let Some(head_name) = current_branch_name(&wt_repo) {
                        if head_name == branch_name {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Try to prune a worktree if it's stale (directory no longer exists)
/// Returns true if the worktree was pruned successfully
pub fn try_prune_worktree(repo: &Repository, wt_name: &str) -> bool {
    if let Ok(wt) = repo.find_worktree(wt_name) {
        if wt.validate().is_err() {
            // Worktree is stale, try to prune it
            if wt
                .prune(Some(
                    git2::WorktreePruneOptions::new()
                        .working_tree(true)
                        .valid(false)
                        .locked(false),
                ))
                .is_ok()
            {
                return true;
            }
        }
    }
    false
}

/// Remove a worktree using git CLI
/// This handles both stale and valid worktrees
pub fn remove_worktree(wt_name: &str) -> Result<()> {
    run_git_command(&["worktree", "remove", wt_name, "--force"])?;
    Ok(())
}

/// Check if the working directory is clean
/// Only checks for actual changes (modified, staged, deleted, etc.)
/// Ignores untracked files and submodules to match `git status` behavior
pub fn is_working_directory_clean(repo: &Repository) -> Result<bool> {
    use git2::StatusOptions;

    let mut opts = StatusOptions::new();
    opts.include_untracked(false)
        .include_ignored(false)
        .include_unmodified(false)
        .exclude_submodules(true);

    let statuses = repo.statuses(Some(&mut opts))?;
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
            let raw = captures.get(1).map(|m| m.as_str().trim())?;
            return normalize_gg_id(raw);
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

fn extract_description_from_message(message: &str) -> Option<String> {
    let stripped = strip_gg_id_from_message(message);
    let newline_idx = stripped.find('\n')?;
    let description = stripped[newline_idx + 1..].trim();
    if description.is_empty() {
        None
    } else {
        Some(description.to_string())
    }
}

/// Get the commit message title (first line)
pub fn get_commit_title(commit: &Commit) -> String {
    commit.summary().unwrap_or("<no summary>").to_string()
}

/// Get the commit message description (body), if present
pub fn get_commit_description(commit: &Commit) -> Option<String> {
    let message = commit.message()?;
    extract_description_from_message(message)
}

/// Checkout a branch by name
pub fn checkout_branch(repo: &Repository, branch_name: &str) -> Result<()> {
    let refname = format!("refs/heads/{}", branch_name);
    let obj = repo.revparse_single(&refname)?;

    repo.checkout_tree(&obj, None)?;
    repo.set_head(&refname)?;
    Ok(())
}

/// Move a branch to point at the current HEAD commit
pub fn move_branch_to_head(repo: &Repository, branch_name: &str) -> Result<()> {
    let head_oid = repo.head()?.peel_to_commit()?.id();
    let mut reference = repo.find_reference(&format!("refs/heads/{}", branch_name))?;
    reference.set_target(head_oid, "gg lint: update branch")?;
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

/// Fetch from origin and prune stale remote-tracking refs
/// This ensures we have up-to-date remote state before operations like sync
pub fn fetch_and_prune() -> Result<()> {
    // Using subprocess because git2's fetch requires complex auth callback setup
    let _ = std::process::Command::new("git")
        .args(["fetch", "origin", "--prune"])
        .output();
    Ok(())
}

/// Get the OID of a remote branch, if it exists
/// Returns None if the remote branch doesn't exist
pub fn get_remote_branch_oid(repo: &Repository, branch_name: &str) -> Option<Oid> {
    let remote_ref = format!("refs/remotes/origin/{}", branch_name);
    repo.revparse_single(&remote_ref).ok().map(|obj| obj.id())
}

/// Push a branch to origin
///
/// - `force_with_lease`: Use --force-with-lease (safe force, recommended for stacked diffs)
/// - `hard_force`: Use --force (overrides force_with_lease, use only as escape hatch)
///
/// If force_with_lease fails with "stale info", retries without lease since
/// the remote branch may have been deleted (e.g., after a PR was merged)
pub fn push_branch(branch_name: &str, force_with_lease: bool, hard_force: bool) -> Result<()> {
    let mut args = vec!["push", "origin", branch_name];
    if hard_force {
        args.insert(1, "--force");
    } else if force_with_lease {
        args.insert(1, "--force-with-lease");
    }

    let output = Command::new("git").args(&args).output()?;

    if output.status.success() {
        return Ok(());
    }

    // Push failed - parse stderr for better error messages
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr_str = stderr.trim();

    // Check for "stale info" error (force-with-lease conflict)
    if force_with_lease && !hard_force && stderr_str.contains("stale info") {
        // Check if we're in a non-interactive environment
        if !atty::is(atty::Stream::Stdin) {
            return Err(GgError::Other(format!(
                "Remote branch '{}' has been updated since your last fetch.\n\
                 This could mean someone else has pushed changes.\n\
                 \n\
                 To proceed safely:\n\
                 1. Run 'git fetch origin'\n\
                 2. Review the changes\n\
                 3. Run 'gg sync' again\n\
                 \n\
                 If you're certain you want to overwrite remote changes, run with --force flag.",
                branch_name
            )));
        }

        // In interactive mode, prompt user for confirmation
        use dialoguer::Confirm;
        eprintln!(
            "\n{} Remote branch '{}' has been updated since your last fetch.",
            console::style("Warning:").yellow().bold(),
            branch_name
        );
        eprintln!("This could mean someone else has pushed changes to this branch.");
        eprintln!();

        let should_force = Confirm::new()
            .with_prompt(format!(
                "Do you want to force-push and overwrite remote changes on '{}'?",
                branch_name
            ))
            .default(false)
            .interact()
            .unwrap_or(false);

        if !should_force {
            return Err(GgError::Other(
                "Push cancelled. Run 'git fetch origin' to update your local state.".to_string(),
            ));
        }

        // User confirmed, proceed with force push
        eprintln!("{}", console::style("Force-pushing...").dim());
        let retry_args = vec!["push", "--force", "origin", branch_name];
        return run_git_command(&retry_args).map(|_| ());
    }

    // Parse stderr to extract hook errors vs git errors
    let (hook_error, git_error) = parse_push_error(stderr_str);

    Err(GgError::PushFailed {
        branch: branch_name.to_string(),
        hook_error,
        git_error,
    })
}

/// Parse git push stderr to separate hook errors from git errors
fn parse_push_error(stderr: &str) -> (Option<String>, Option<String>) {
    let lines: Vec<&str> = stderr.lines().collect();

    // Look for pre-push hook output (usually appears before "error: failed to push")
    // Hook output typically doesn't start with "error:" or "remote:"
    let mut hook_lines = Vec::new();
    let mut git_lines = Vec::new();

    for line in lines {
        let trimmed = line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Git error messages typically start with "error:" or "remote:"
        if trimmed.starts_with("error:") || trimmed.starts_with("remote:") {
            // Skip the generic "failed to push some refs" message (it's redundant)
            if !trimmed.contains("failed to push some refs") {
                git_lines.push(trimmed);
            }
        } else {
            // Assume this is hook output
            hook_lines.push(trimmed);
        }
    }

    let hook_error = if hook_lines.is_empty() {
        None
    } else {
        Some(hook_lines.join("\n"))
    };

    let git_error = if git_lines.is_empty() {
        None
    } else {
        Some(git_lines.join("\n"))
    };

    (hook_error, git_error)
}

/// Delete a remote branch
pub fn delete_remote_branch(branch_name: &str) -> Result<()> {
    run_git_command(&["push", "origin", "--delete", branch_name])?;
    Ok(())
}

/// Continue a rebase
pub fn rebase_continue() -> Result<()> {
    // Set GIT_EDITOR=true to avoid "Terminal is dumb, but EDITOR unset" errors
    // This allows rebase to continue without requiring an interactive editor
    let output = Command::new("git")
        .args(["rebase", "--continue"])
        .env("GIT_EDITOR", "true")
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(GgError::Other(format!(
            "git rebase --continue failed: {}",
            stderr
        )))
    }
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

/// Sanitize and validate a stack name
///
/// - Converts spaces to hyphens (kebab-case)
/// - Rejects names containing `/` (conflicts with branch format)
/// - Rejects names containing `--` (conflicts with entry branch format)
/// - Rejects names with other invalid git ref characters
///
/// Returns the sanitized name or an error
pub fn sanitize_stack_name(name: &str) -> Result<String> {
    // First, convert spaces to hyphens
    let sanitized = name.replace(' ', "-");

    // Check for reserved sequences that conflict with gg branch format
    if sanitized.contains('/') {
        return Err(GgError::InvalidStackName(
            "Stack name cannot contain '/' (conflicts with branch format)".to_string(),
        ));
    }

    if sanitized.contains("--") {
        return Err(GgError::InvalidStackName(
            "Stack name cannot contain '--' (conflicts with entry branch format)".to_string(),
        ));
    }

    // Check for other invalid git ref characters
    // Git ref names cannot contain: space, ~, ^, :, ?, *, [, \, control chars
    // Also cannot start or end with dot, contain "..", or end with ".lock"
    let invalid_chars = ['~', '^', ':', '?', '*', '[', '\\', '@'];
    for c in invalid_chars {
        if sanitized.contains(c) {
            return Err(GgError::InvalidStackName(format!(
                "Stack name cannot contain '{}'",
                c
            )));
        }
    }

    // Check for control characters
    if sanitized.chars().any(|c| c.is_control()) {
        return Err(GgError::InvalidStackName(
            "Stack name cannot contain control characters".to_string(),
        ));
    }

    // Check for ".." sequence
    if sanitized.contains("..") {
        return Err(GgError::InvalidStackName(
            "Stack name cannot contain '..'".to_string(),
        ));
    }

    // Check for starting/ending with dot
    if sanitized.starts_with('.') || sanitized.ends_with('.') {
        return Err(GgError::InvalidStackName(
            "Stack name cannot start or end with '.'".to_string(),
        ));
    }

    // Check for ending with .lock
    if sanitized.ends_with(".lock") {
        return Err(GgError::InvalidStackName(
            "Stack name cannot end with '.lock'".to_string(),
        ));
    }

    // Check for empty name after sanitization
    if sanitized.is_empty() || sanitized.chars().all(|c| c == '-') {
        return Err(GgError::InvalidStackName(
            "Stack name cannot be empty".to_string(),
        ));
    }

    Ok(sanitized)
}

/// Validate branch username used in branch names.
///
/// The username must not contain `/` or other invalid git ref characters.
pub fn validate_branch_username(username: &str) -> Result<()> {
    if username.is_empty() {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot be empty".to_string(),
        ));
    }

    if username.contains('/') {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot contain '/'".to_string(),
        ));
    }

    if username.chars().any(|c| c.is_whitespace()) {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot contain whitespace".to_string(),
        ));
    }

    let invalid_chars = ['~', '^', ':', '?', '*', '[', '\\', '@'];
    for c in invalid_chars {
        if username.contains(c) {
            return Err(GgError::InvalidBranchUsername(format!(
                "Branch username cannot contain '{}'",
                c
            )));
        }
    }

    if username.chars().any(|c| c.is_control()) {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot contain control characters".to_string(),
        ));
    }

    if username.contains("..") {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot contain '..'".to_string(),
        ));
    }

    if username.starts_with('.') || username.ends_with('.') {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot start or end with '.'".to_string(),
        ));
    }

    if username.ends_with(".lock") {
        return Err(GgError::InvalidBranchUsername(
            "Branch username cannot end with '.lock'".to_string(),
        ));
    }

    Ok(())
}

/// Validate GG-ID format and normalize to lowercase.
pub fn normalize_gg_id(gg_id: &str) -> Option<String> {
    let re = Regex::new(r"(?i)^c-[0-9a-f]{7}$").ok()?;
    if re.is_match(gg_id) {
        Some(gg_id.to_lowercase())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_stack_name() {
        // Spaces converted to hyphens
        assert_eq!(
            sanitize_stack_name("my new feature").unwrap(),
            "my-new-feature"
        );

        // Already valid
        assert_eq!(sanitize_stack_name("my-feature").unwrap(), "my-feature");

        // Invalid: contains /
        assert!(sanitize_stack_name("my/feature").is_err());

        // Invalid: contains --
        assert!(sanitize_stack_name("my--feature").is_err());

        // Invalid: contains special chars
        assert!(sanitize_stack_name("my~feature").is_err());
        assert!(sanitize_stack_name("my:feature").is_err());
        assert!(sanitize_stack_name("my*feature").is_err());

        // Invalid: starts/ends with dot
        assert!(sanitize_stack_name(".myfeature").is_err());
        assert!(sanitize_stack_name("myfeature.").is_err());

        // Invalid: contains ..
        assert!(sanitize_stack_name("my..feature").is_err());

        // Invalid: ends with .lock
        assert!(sanitize_stack_name("feature.lock").is_err());

        // Invalid: empty
        assert!(sanitize_stack_name("").is_err());
        assert!(sanitize_stack_name("   ").is_err());
    }

    #[test]
    fn test_validate_branch_username() {
        assert!(validate_branch_username("nacho").is_ok());
        assert!(validate_branch_username("nacho-lopez").is_ok());
        assert!(validate_branch_username("nacho.lopez").is_ok());

        assert!(validate_branch_username("").is_err());
        assert!(validate_branch_username("na/cho").is_err());
        assert!(validate_branch_username("na cho").is_err());
        assert!(validate_branch_username("na..cho").is_err());
        assert!(validate_branch_username(".nacho").is_err());
        assert!(validate_branch_username("nacho.").is_err());
        assert!(validate_branch_username("nacho.lock").is_err());
        assert!(validate_branch_username("nacho@home").is_err());
    }

    #[test]
    fn test_normalize_gg_id() {
        assert_eq!(normalize_gg_id("c-abc1234"), Some("c-abc1234".to_string()));
        assert_eq!(normalize_gg_id("C-ABC1234"), Some("c-abc1234".to_string()));
        assert_eq!(normalize_gg_id("c-abcdefg"), None);
        assert_eq!(normalize_gg_id("c-123456"), None);
        assert_eq!(normalize_gg_id("invalid"), None);
    }

    #[test]
    fn test_parse_stack_branch() {
        assert_eq!(
            parse_stack_branch("nacho/my-feature"),
            Some(("nacho".to_string(), "my-feature".to_string()))
        );
        assert_eq!(parse_stack_branch("main"), None);
        assert_eq!(parse_stack_branch("nacho/my-feature/c-abc123"), None);
        // Entry branches should NOT be parsed as stack branches
        assert_eq!(parse_stack_branch("nacho/my-feature--c-abc123"), None);
        assert_eq!(parse_stack_branch("nacho/claude.md--c-7d3f2a6"), None);
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
    fn test_strip_gg_id_edge_cases() {
        // Case insensitive
        let msg = "Title\n\nBody\n\ngg-id: c-abc123";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.to_lowercase().contains("gg-id"));
        assert_eq!(result, "Title\n\nBody");

        // Mixed case
        let msg = "Title\n\nBody\n\nGg-Id: c-abc123";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.to_lowercase().contains("gg-id"));

        // Multiple GG-IDs (shouldn't happen but should handle)
        let msg = "Title\n\nBody\n\nGG-ID: c-abc123\nGG-ID: c-def456";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.contains("GG-ID"));
        assert_eq!(result, "Title\n\nBody");

        // GG-ID with extra spaces after colon
        let msg = "Title\n\nBody\n\nGG-ID:   c-abc123";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.contains("GG-ID"));

        // GG-ID only (no body)
        let msg = "Title\n\nGG-ID: c-abc123";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.contains("GG-ID"));
        assert_eq!(result, "Title");

        // GG-ID in middle of body (rare but possible)
        let msg = "Title\n\nFirst paragraph\n\nGG-ID: c-abc123\n\nSecond paragraph";
        let result = strip_gg_id_from_message(msg);
        assert!(!result.contains("GG-ID"));
        assert!(result.contains("First paragraph"));
        assert!(result.contains("Second paragraph"));
    }

    #[test]
    fn test_extract_description_from_message() {
        let msg = "Add feature\n\nSome description\n\nGG-ID: c-abc123";
        let result = extract_description_from_message(msg);
        assert_eq!(result.as_deref(), Some("Some description"));

        let msg_no_body = "Add feature\n\nGG-ID: c-abc123";
        let result = extract_description_from_message(msg_no_body);
        assert!(result.is_none());

        let msg_multi = "Add feature\n\nLine one\n\nLine two";
        let result = extract_description_from_message(msg_multi);
        assert_eq!(result.as_deref(), Some("Line one\n\nLine two"));
    }

    #[test]
    fn test_extract_description_filters_gg_id() {
        // Ensure GG-ID is never present in extracted description
        // This is critical for PR/MR descriptions

        // GG-ID at end of body
        let msg = "Title\n\nThis is the body.\n\nMore details here.\n\nGG-ID: c-abc123";
        let result = extract_description_from_message(msg);
        assert!(result.is_some());
        let desc = result.unwrap();
        assert!(!desc.contains("GG-ID"));
        assert!(desc.contains("This is the body."));
        assert!(desc.contains("More details here."));

        // GG-ID with lowercase
        let msg = "Title\n\nBody text\n\ngg-id: c-abc123";
        let result = extract_description_from_message(msg);
        assert!(result.is_some());
        assert!(!result.unwrap().to_lowercase().contains("gg-id"));

        // Multiple paragraphs with GG-ID at end
        let msg = "feat: add new feature\n\nFirst paragraph explaining the change.\n\nSecond paragraph with more details.\n\n- Bullet point 1\n- Bullet point 2\n\nGG-ID: c-1234567";
        let result = extract_description_from_message(msg);
        assert!(result.is_some());
        let desc = result.unwrap();
        assert!(!desc.contains("GG-ID"));
        assert!(desc.contains("First paragraph"));
        assert!(desc.contains("Bullet point"));
    }

    #[test]
    fn test_generate_gg_id() {
        let id = generate_gg_id();
        assert!(id.starts_with("c-"));
        assert_eq!(id.len(), 9); // "c-" + 7 chars
    }

    #[test]
    fn test_parse_push_error_with_hook() {
        let stderr = "😢 ktlint found style violations in Kotlin code.\nerror: failed to push some refs to 'gitlab.example.com:user/repo.git'";
        let (hook_error, git_error) = parse_push_error(stderr);

        assert!(hook_error.is_some());
        assert_eq!(
            hook_error.unwrap(),
            "😢 ktlint found style violations in Kotlin code."
        );
        assert!(git_error.is_none()); // Generic "failed to push" is filtered out
    }

    #[test]
    fn test_parse_push_error_hook_only() {
        let stderr = "Pre-push hook failed\nSome detailed error message";
        let (hook_error, git_error) = parse_push_error(stderr);

        assert!(hook_error.is_some());
        assert!(hook_error.unwrap().contains("Pre-push hook failed"));
        assert!(git_error.is_none());
    }

    #[test]
    fn test_parse_push_error_git_only() {
        let stderr = "error: failed to push some refs to 'origin'\nremote: Permission denied";
        let (hook_error, git_error) = parse_push_error(stderr);

        assert!(hook_error.is_none());
        assert!(git_error.is_some());
        assert_eq!(git_error.unwrap(), "remote: Permission denied");
    }

    #[test]
    fn test_parse_push_error_both() {
        let stderr =
            "Hook output line 1\nHook output line 2\nerror: some git error\nremote: remote error";
        let (hook_error, git_error) = parse_push_error(stderr);

        assert!(hook_error.is_some());
        assert!(hook_error.unwrap().contains("Hook output line 1"));
        assert!(git_error.is_some());
        assert!(git_error.unwrap().contains("some git error"));
    }

    #[test]
    fn test_parse_push_error_empty() {
        let stderr = "";
        let (hook_error, git_error) = parse_push_error(stderr);

        assert!(hook_error.is_none());
        assert!(git_error.is_none());
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
