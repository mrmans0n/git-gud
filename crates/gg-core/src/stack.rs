//! Stack model for git-gud
//!
//! A stack is a linear sequence of commits on a branch, each identified
//! by a stable GG-ID trailer that persists across rebases.

use std::fs;
use std::path::Path;

use git2::{Commit, Repository};

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git::{self, get_gg_id, get_gg_parent, short_sha};
use crate::provider::{CiStatus, PrState, Provider};

/// File to store the current stack when in detached HEAD mode
const CURRENT_STACK_FILE: &str = "gg/current_stack";

/// A single entry in a stack (one commit)
#[derive(Debug, Clone)]
pub struct StackEntry {
    /// The commit OID
    pub oid: git2::Oid,
    /// Short SHA for display
    pub short_sha: String,
    /// Commit title (first line)
    pub title: String,
    /// GG-ID (stable identifier)
    pub gg_id: Option<String>,
    /// GG-ID of the previous stack entry
    pub gg_parent: Option<String>,
    /// PR number if synced
    pub mr_number: Option<u64>,
    /// PR state if synced
    pub mr_state: Option<PrState>,
    /// Whether the PR is approved
    pub approved: bool,
    /// Whether changes have been requested on the PR
    pub changes_requested: bool,
    /// Whether the PR is mergeable
    pub mergeable: bool,
    /// CI status
    pub ci_status: Option<CiStatus>,
    /// Position in the stack (1-indexed)
    pub position: usize,
    /// Whether this MR is in a merge train (GitLab only)
    pub in_merge_train: bool,
    /// Position in merge train if applicable
    pub merge_train_position: Option<usize>,
}

impl StackEntry {
    /// Create a new stack entry from a commit
    pub fn from_commit(commit: &Commit, position: usize) -> Self {
        StackEntry {
            oid: commit.id(),
            short_sha: short_sha(commit),
            title: git::get_commit_title(commit),
            gg_id: get_gg_id(commit),
            gg_parent: get_gg_parent(commit),
            mr_number: None,
            mr_state: None,
            approved: false,
            changes_requested: false,
            mergeable: false,
            ci_status: None,
            position,
            in_merge_train: false,
            merge_train_position: None,
        }
    }

    /// Check if this entry has been synced (has MR)
    pub fn is_synced(&self) -> bool {
        self.mr_number.is_some()
    }

    /// Check if this entry needs a GG-ID
    pub fn needs_gg_id(&self) -> bool {
        self.gg_id.is_none()
    }

    /// Get status display string
    pub fn status_display(&self) -> String {
        match (&self.mr_state, self.approved) {
            (Some(PrState::Merged), _) => "merged".to_string(),
            (Some(PrState::Closed), _) => "closed".to_string(),
            (Some(PrState::Draft), _) => "draft".to_string(),
            (Some(PrState::Open), true) => "approved".to_string(),
            (Some(PrState::Open), false) => "open".to_string(),
            // If we have an MR number but no state, it's pushed but status not fetched
            // Use --refresh to fetch current state
            (None, _) if self.mr_number.is_some() => String::new(),
            (None, _) => "not pushed".to_string(),
        }
    }
}

/// A complete stack
#[derive(Debug)]
pub struct Stack {
    /// Stack name (from branch)
    pub name: String,
    /// Username (from branch)
    pub username: String,
    /// Base branch
    pub base: String,
    /// Stack entries (commits), ordered from base to HEAD
    pub entries: Vec<StackEntry>,
    /// Current HEAD position in stack (0-indexed into entries, or None if at stack head)
    pub current_position: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackPrefixMismatch {
    pub current_branch: String,
    pub actual_prefix: String,
    pub expected_prefix: String,
    pub stack_name: String,
    pub suggested_branch: String,
}

impl StackPrefixMismatch {
    pub fn warning_message(&self) -> String {
        format!(
            "current branch '{}' does not use the configured prefix '{}/'. Some stack discovery, listing, and saved PR/MR mappings may be inaccurate. Consider renaming it: git branch -m {}",
            self.current_branch, self.expected_prefix, self.suggested_branch
        )
    }
}

impl Stack {
    /// Load stack from the current branch (or stored stack if in detached HEAD)
    pub fn load(repo: &Repository, config: &Config) -> Result<Self> {
        let git_dir = repo.path();

        // First, try to get branch name from HEAD
        let on_branch = git::current_branch_name(repo).is_some();
        let branch_name = git::current_branch_name(repo)
            .or_else(|| {
                // If in detached HEAD, try to read stored stack
                read_current_stack(git_dir)
            })
            .ok_or(GgError::NotOnStack)?;

        let (username, name) = git::parse_stack_branch(&branch_name).ok_or_else(|| {
            GgError::NotOnStackBranch(format_not_stack_branch_error(&branch_name, config))
        })?;

        // Determine base branch
        let base = config
            .get_base_for_stack(&name)
            .map(|s| s.to_string())
            .or_else(|| git::find_base_branch(repo).ok())
            .ok_or(GgError::NoBaseBranch)?;

        // Get commit OIDs - use branch ref if in detached HEAD mode
        let stack_ref = if on_branch {
            None
        } else {
            Some(branch_name.clone())
        };
        let oids = git::get_stack_commit_oids(repo, &base, stack_ref.as_deref())?;

        // Build entries
        let mut entries: Vec<StackEntry> = Vec::with_capacity(oids.len());
        for (i, oid) in oids.iter().enumerate() {
            let commit = repo.find_commit(*oid)?;
            entries.push(StackEntry::from_commit(&commit, i + 1));
        }

        // Enrich with MR info from config
        if let Some(stack_config) = config.get_stack(&name) {
            for entry in &mut entries {
                if let Some(gg_id) = &entry.gg_id {
                    if let Some(mr_num) = stack_config.mrs.get(gg_id) {
                        entry.mr_number = Some(*mr_num);
                    }
                }
            }
        }

        // Determine current position (if in detached HEAD at a stack commit)
        let head = repo.head()?.peel_to_commit()?;
        let current_position = entries.iter().position(|e| e.oid == head.id());

        Ok(Stack {
            name,
            username,
            base,
            entries,
            current_position,
        })
    }

    pub fn prefix_mismatch(&self, config: &Config) -> Option<StackPrefixMismatch> {
        let expected_prefix = config.defaults.branch_username.as_deref()?;
        if expected_prefix.is_empty() || expected_prefix == self.username {
            return None;
        }

        Some(StackPrefixMismatch {
            current_branch: self.branch_name(),
            actual_prefix: self.username.clone(),
            expected_prefix: expected_prefix.to_string(),
            stack_name: self.name.clone(),
            suggested_branch: git::format_stack_branch(expected_prefix, &self.name),
        })
    }

    /// Check if any entries need GG-IDs
    #[allow(dead_code)] // Reserved for future use
    pub fn has_missing_gg_ids(&self) -> bool {
        self.entries.iter().any(|e| e.needs_gg_id())
    }

    /// Get entries that need GG-IDs
    pub fn entries_needing_gg_ids(&self) -> Vec<&StackEntry> {
        self.entries.iter().filter(|e| e.needs_gg_id()).collect()
    }

    /// Get the expected GG-Parent for an entry position (1-indexed)
    pub fn expected_parent_gg_id(&self, position: usize) -> Option<&str> {
        if position <= 1 || position > self.entries.len() {
            None
        } else {
            self.entries[position - 2].gg_id.as_deref()
        }
    }

    /// Get entry by position (1-indexed)
    pub fn get_entry_by_position(&self, pos: usize) -> Option<&StackEntry> {
        if pos == 0 || pos > self.entries.len() {
            None
        } else {
            self.entries.get(pos - 1)
        }
    }

    /// Get entry by GG-ID
    pub fn get_entry_by_gg_id(&self, gg_id: &str) -> Option<&StackEntry> {
        self.entries
            .iter()
            .find(|e| e.gg_id.as_deref() == Some(gg_id))
    }

    /// Get the first entry
    pub fn first(&self) -> Option<&StackEntry> {
        self.entries.first()
    }

    /// Get the last entry (stack head)
    pub fn last(&self) -> Option<&StackEntry> {
        self.entries.last()
    }

    /// Get the current entry (based on HEAD)
    #[allow(dead_code)] // Reserved for future use
    pub fn current(&self) -> Option<&StackEntry> {
        self.current_position.and_then(|p| self.entries.get(p))
    }

    /// Get the previous entry relative to current position
    pub fn prev(&self) -> Option<&StackEntry> {
        let current = self
            .current_position
            .unwrap_or(self.entries.len().saturating_sub(1));
        if current > 0 {
            self.entries.get(current - 1)
        } else {
            None
        }
    }

    /// Get the next entry relative to current position
    pub fn next(&self) -> Option<&StackEntry> {
        let current = self.current_position.unwrap_or(self.entries.len());
        if current < self.entries.len() - 1 {
            self.entries.get(current + 1)
        } else {
            None
        }
    }

    /// Total number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if stack is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Count synced entries
    pub fn synced_count(&self) -> usize {
        self.entries.iter().filter(|e| e.is_synced()).count()
    }

    /// Format stack branch name
    pub fn branch_name(&self) -> String {
        git::format_stack_branch(&self.username, &self.name)
    }

    /// Format entry branch name for a given entry
    pub fn entry_branch_name(&self, entry: &StackEntry) -> Option<String> {
        entry
            .gg_id
            .as_ref()
            .and_then(|gg_id| git::normalize_gg_id(gg_id))
            .map(|gg_id| git::format_entry_branch(&self.username, &self.name, &gg_id))
    }

    /// Refresh PR/MR info for all entries from provider
    pub fn refresh_mr_info(&mut self, provider: &Provider) -> Result<()> {
        for entry in &mut self.entries {
            if let Some(pr_num) = entry.mr_number {
                match provider.get_pr_info(pr_num) {
                    Ok(info) => {
                        entry.mr_state = Some(info.state);
                        entry.approved = info.approved;
                        entry.changes_requested = info.changes_requested;
                        entry.mergeable = info.mergeable;
                    }
                    Err(_) => {
                        // PR/MR might have been deleted
                        entry.mr_state = None;
                    }
                }

                // Get CI status
                if let Ok(ci) = provider.get_pr_ci_status(pr_num) {
                    entry.ci_status = Some(ci);
                }

                // Check approval status
                if let Ok(approved) = provider.check_pr_approved(pr_num) {
                    entry.approved = approved;
                }

                // Check merge train status (GitLab only)
                if let Ok(Some(train_info)) = provider.get_merge_train_status(pr_num, &self.base) {
                    use crate::glab::MergeTrainStatus;
                    entry.in_merge_train = !matches!(train_info.status, MergeTrainStatus::Idle);
                    entry.merge_train_position = train_info.position;
                }
            }
        }
        Ok(())
    }
}

fn format_not_stack_branch_error(branch_name: &str, config: &Config) -> String {
    if let Some(expected_prefix) = config
        .defaults
        .branch_username
        .as_deref()
        .filter(|prefix| !prefix.is_empty())
        .filter(|_| !branch_name.contains('/'))
    {
        let suggested_branch = git::format_stack_branch(expected_prefix, branch_name);
        format!(
            "Current branch '{}' is not a stack branch. Expected format: '<prefix>/<stack-name>', for example '{}'. Rename it with: git branch -m {}",
            branch_name, suggested_branch, suggested_branch
        )
    } else {
        format!(
            "Current branch '{}' is not a stack branch. Expected format: '<prefix>/<stack-name>'. Use `gg co <stack-name>` to create or switch to a stack.",
            branch_name
        )
    }
}

/// Resolve a target string (position, GG-ID, or SHA) to a position in the stack
pub fn resolve_target(stack: &Stack, target: &str) -> Result<usize> {
    // Try to parse target as position (1-indexed number)
    if let Ok(pos) = target.parse::<usize>() {
        if pos == 0 || pos > stack.len() {
            return Err(GgError::Other(format!(
                "Position {} is out of range (1-{})",
                pos,
                stack.len()
            )));
        }
        return Ok(pos);
    }

    // Try to find by GG-ID
    if let Some(entry) = stack.get_entry_by_gg_id(target) {
        return Ok(entry.position);
    }

    // Try to find by SHA prefix
    for entry in &stack.entries {
        if entry.short_sha.starts_with(target) || entry.oid.to_string().starts_with(target) {
            return Ok(entry.position);
        }
    }

    Err(GgError::Other(format!(
        "Could not find commit matching '{}' in stack",
        target
    )))
}

/// Store the current stack branch for use in detached HEAD mode
#[allow(dead_code)]
pub fn save_current_stack(git_dir: &Path, branch_name: &str) -> Result<()> {
    let stack_file = git_dir.join(CURRENT_STACK_FILE);
    if let Some(parent) = stack_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(stack_file, branch_name)?;
    Ok(())
}

/// Read the stored current stack (returns branch name only)
pub fn read_current_stack(git_dir: &Path) -> Option<String> {
    let stack_file = git_dir.join(CURRENT_STACK_FILE);
    let content = fs::read_to_string(stack_file).ok()?;
    let trimmed = content.trim();
    // Handle new format (with |) and old format (just branch name)
    Some(trimmed.split('|').next()?.to_string())
}

/// Clear the stored current stack (when returning to branch)
pub fn clear_current_stack(git_dir: &Path) -> Result<()> {
    let stack_file = git_dir.join(CURRENT_STACK_FILE);
    if stack_file.exists() {
        fs::remove_file(stack_file)?;
    }
    Ok(())
}

/// Save navigation context (branch, position, and original OID) for detached HEAD tracking
pub fn save_nav_context(
    git_dir: &Path,
    branch_name: &str,
    position: usize,
    oid: git2::Oid,
) -> Result<()> {
    let stack_file = git_dir.join(CURRENT_STACK_FILE);
    if let Some(parent) = stack_file.parent() {
        fs::create_dir_all(parent)?;
    }
    // Format: branch_name|position|oid
    let content = format!("{}|{}|{}", branch_name, position, oid);
    fs::write(stack_file, content)?;
    Ok(())
}

/// Read navigation context (branch, position, oid) if available
pub fn read_nav_context(git_dir: &Path) -> Option<(String, usize, git2::Oid)> {
    let stack_file = git_dir.join(CURRENT_STACK_FILE);
    let content = fs::read_to_string(stack_file).ok()?;
    let parts: Vec<&str> = content.trim().split('|').collect();
    if parts.len() == 3 {
        let branch = parts[0].to_string();
        let position = parts[1].parse().ok()?;
        let oid = git2::Oid::from_str(parts[2]).ok()?;
        Some((branch, position, oid))
    } else {
        // Old format (just branch name) - return None
        None
    }
}

/// Whether the detached-HEAD commit was added on top of the navigated commit
/// (`Inserted`) or rewrites it in place (`Amended`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnintegratedKind {
    Inserted,
    Amended,
}

/// A commit (or chain of commits) made at a detached mid-stack HEAD that has not
/// yet been folded into the stack branch.
#[derive(Debug, Clone)]
pub struct UnintegratedCommit {
    /// Current detached HEAD oid (the tip of the un-integrated work).
    pub head_oid: git2::Oid,
    /// The commit we navigated to (`gg mv`), recorded in the nav context.
    pub original_oid: git2::Oid,
    /// Stack branch name from the nav context.
    pub branch_name: String,
    /// 0-indexed position of the navigated commit within the stack.
    pub saved_position: usize,
    /// 1-indexed display position the un-integrated work sits on.
    pub sits_on_position: usize,
    /// Short sha of `head_oid`.
    pub short_sha: String,
    /// First line of the head commit message.
    pub subject: String,
    /// Number of un-integrated commits (1 for an amend).
    pub count: usize,
    pub kind: UnintegratedKind,
}

/// Detect a commit made at a detached mid-stack HEAD that has not been folded
/// into the stack yet. Returns `None` when there is nothing to integrate.
///
/// Pure detection — never mutates the repository.
pub fn detect_unintegrated(repo: &Repository, stack: &Stack) -> Result<Option<UnintegratedCommit>> {
    let (branch_name, saved_position, original_oid) = match read_nav_context(repo.path()) {
        Some(ctx) => ctx,
        None => return Ok(None),
    };

    if !repo.head_detached()? {
        return Ok(None);
    }
    let head = repo.head()?.peel_to_commit()?;
    let head_oid = head.id();
    if head_oid == original_oid {
        return Ok(None);
    }

    // Only mid-stack positions can have un-integrated work: there must be at
    // least one commit above the navigated position to rebase onto the new
    // HEAD. At the tip there is nothing to move (`rebase --onto` would span an
    // empty range), and `gg` checks out the branch rather than detaching there
    // anyway. Mirrors the guard in `check_and_rebase_if_modified`.
    if saved_position + 1 >= stack.len() {
        return Ok(None);
    }

    let kind = if repo.graph_descendant_of(head_oid, original_oid)? {
        UnintegratedKind::Inserted
    } else {
        let original = repo.find_commit(original_oid)?;
        let original_parent = original.parent_id(0).ok();
        let head_parent = head.parent_id(0).ok();
        if original_parent.is_some() && original_parent == head_parent {
            UnintegratedKind::Amended
        } else {
            return Ok(None);
        }
    };

    let branch_tip = match stack.entries.last() {
        Some(entry) => entry.oid,
        None => return Ok(None),
    };
    if branch_tip == head_oid || repo.graph_descendant_of(branch_tip, head_oid)? {
        return Ok(None);
    }

    let (count, _) = repo.graph_ahead_behind(head_oid, original_oid)?;
    let count = count.max(1);

    let subject = head.summary()?.unwrap_or("(no message)").to_string();
    let short_sha = repo
        .find_object(head_oid, None)?
        .short_id()?
        .as_str()
        .unwrap_or("")
        .to_string();

    Ok(Some(UnintegratedCommit {
        head_oid,
        original_oid,
        branch_name,
        saved_position,
        sits_on_position: saved_position + 1,
        short_sha,
        subject,
        count,
        kind,
    }))
}

/// List all stacks in the repository
pub fn list_all_stacks(repo: &Repository, config: &Config, username: &str) -> Result<Vec<String>> {
    let mut stacks = Vec::new();

    // Get stacks from config
    for name in config.list_stacks() {
        stacks.push(name.to_string());
    }

    // Also scan local branches matching username/stack-name or username/stack-name/entry-id pattern
    let branches = repo.branches(Some(git2::BranchType::Local))?;
    for branch_result in branches {
        let (branch, _) = branch_result?;
        if let Some(name) = branch.name()? {
            // Check for 2-part stack branch (username/stack-name)
            if let Some((branch_user, stack_name)) = git::parse_stack_branch(name) {
                if branch_user == username && !stacks.contains(&stack_name) {
                    stacks.push(stack_name);
                }
            }
            // Also check for 3-part entry branch (username/stack-name/entry-id)
            else if let Some((branch_user, stack_name, _entry_id)) = git::parse_entry_branch(name)
            {
                if branch_user == username && !stacks.contains(&stack_name) {
                    stacks.push(stack_name);
                }
            }
        }
    }

    stacks.sort();
    stacks.dedup();
    Ok(stacks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_entry(pos: usize, gg_id: Option<&str>) -> StackEntry {
        StackEntry {
            oid: git2::Oid::ZERO_SHA1,
            short_sha: format!("sha{}", pos),
            title: format!("commit {}", pos),
            gg_id: gg_id.map(ToString::to_string),
            gg_parent: None,
            mr_number: None,
            mr_state: None,
            approved: false,
            changes_requested: false,
            mergeable: false,
            ci_status: None,
            position: pos,
            in_merge_train: false,
            merge_train_position: None,
        }
    }

    #[test]
    fn expected_parent_first_entry_is_none() {
        let stack = Stack {
            name: "s".to_string(),
            username: "u".to_string(),
            base: "main".to_string(),
            entries: vec![mk_entry(1, Some("c-a"))],
            current_position: Some(0),
        };

        assert_eq!(stack.expected_parent_gg_id(1), None);
    }

    #[test]
    fn expected_parent_uses_previous_entry_gg_id() {
        let stack = Stack {
            name: "s".to_string(),
            username: "u".to_string(),
            base: "main".to_string(),
            entries: vec![mk_entry(1, Some("c-a")), mk_entry(2, Some("c-b"))],
            current_position: Some(1),
        };

        assert_eq!(stack.expected_parent_gg_id(2), Some("c-a"));
    }

    #[test]
    fn prefix_mismatch_returns_warning_data_for_wrong_configured_prefix() {
        let stack = Stack {
            name: "feature".to_string(),
            username: "other".to_string(),
            base: "main".to_string(),
            entries: vec![],
            current_position: None,
        };
        let mut config = Config::default();
        config.defaults.branch_username = Some("testuser".to_string());

        let mismatch = stack.prefix_mismatch(&config).expect("should mismatch");

        assert_eq!(mismatch.current_branch, "other/feature");
        assert_eq!(mismatch.actual_prefix, "other");
        assert_eq!(mismatch.expected_prefix, "testuser");
        assert_eq!(mismatch.stack_name, "feature");
        assert_eq!(mismatch.suggested_branch, "testuser/feature");
        assert!(mismatch
            .warning_message()
            .contains("git branch -m testuser/feature"));
    }

    #[test]
    fn prefix_mismatch_is_none_without_configured_prefix() {
        let stack = Stack {
            name: "feature".to_string(),
            username: "other".to_string(),
            base: "main".to_string(),
            entries: vec![],
            current_position: None,
        };

        assert!(stack.prefix_mismatch(&Config::default()).is_none());
    }

    #[test]
    fn prefix_mismatch_is_none_when_prefix_matches() {
        let stack = Stack {
            name: "feature".to_string(),
            username: "testuser".to_string(),
            base: "main".to_string(),
            entries: vec![],
            current_position: None,
        };
        let mut config = Config::default();
        config.defaults.branch_username = Some("testuser".to_string());

        assert!(stack.prefix_mismatch(&config).is_none());
    }

    #[test]
    fn non_stack_branch_error_includes_configured_prefix_hint() {
        let mut config = Config::default();
        config.defaults.branch_username = Some("testuser".to_string());

        let message = format_not_stack_branch_error("feature", &config);

        assert!(message.contains("Current branch 'feature' is not a stack branch"));
        assert!(message.contains("testuser/feature"));
        assert!(message.contains("git branch -m testuser/feature"));
    }
}
