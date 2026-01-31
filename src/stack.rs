//! Stack model for git-gud
//!
//! A stack is a linear sequence of commits on a branch, each identified
//! by a stable GG-ID trailer that persists across rebases.

use std::fs;
use std::path::Path;

use git2::{Commit, Repository};

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git::{self, get_gg_id, short_sha};
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
    /// PR/MR number if synced
    pub mr_number: Option<u64>,
    /// PR/MR state if synced
    pub mr_state: Option<PrState>,
    /// Whether the MR is approved
    pub approved: bool,
    /// CI status
    pub ci_status: Option<CiStatus>,
    /// Position in the stack (1-indexed)
    pub position: usize,
}

impl StackEntry {
    /// Create a new stack entry from a commit
    pub fn from_commit(commit: &Commit, position: usize) -> Self {
        StackEntry {
            oid: commit.id(),
            short_sha: short_sha(commit),
            title: git::get_commit_title(commit),
            gg_id: get_gg_id(commit),
            mr_number: None,
            mr_state: None,
            approved: false,
            ci_status: None,
            position,
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

        let (username, name) = git::parse_stack_branch(&branch_name).ok_or(GgError::NotOnStack)?;

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

    /// Check if any entries need GG-IDs
    #[allow(dead_code)] // Reserved for future use
    pub fn has_missing_gg_ids(&self) -> bool {
        self.entries.iter().any(|e| e.needs_gg_id())
    }

    /// Get entries that need GG-IDs
    pub fn entries_needing_gg_ids(&self) -> Vec<&StackEntry> {
        self.entries.iter().filter(|e| e.needs_gg_id()).collect()
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
            .map(|gg_id| git::format_entry_branch(&self.username, &self.name, gg_id))
    }

    /// Refresh PR/MR info for all entries from the provider
    pub fn refresh_mr_info(&mut self, provider: &dyn Provider) -> Result<()> {
        for entry in &mut self.entries {
            if let Some(mr_num) = entry.mr_number {
                match provider.view_pr(mr_num) {
                    Ok(info) => {
                        entry.mr_state = Some(info.state);
                        entry.approved = info.approved;
                    }
                    Err(_) => {
                        // PR/MR might have been deleted
                        entry.mr_state = None;
                    }
                }

                // Get CI status
                if let Ok(ci) = provider.get_ci_status(mr_num) {
                    entry.ci_status = Some(ci);
                }

                // Check approval status
                if let Ok(approved) = provider.check_approved(mr_num) {
                    entry.approved = approved;
                }
            }
        }
        Ok(())
    }
}

/// Store the current stack branch for use in detached HEAD mode
pub fn save_current_stack(git_dir: &Path, branch_name: &str) -> Result<()> {
    let stack_file = git_dir.join(CURRENT_STACK_FILE);
    if let Some(parent) = stack_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(stack_file, branch_name)?;
    Ok(())
}

/// Read the stored current stack
pub fn read_current_stack(git_dir: &Path) -> Option<String> {
    let stack_file = git_dir.join(CURRENT_STACK_FILE);
    fs::read_to_string(stack_file)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Clear the stored current stack (when returning to branch)
pub fn clear_current_stack(git_dir: &Path) -> Result<()> {
    let stack_file = git_dir.join(CURRENT_STACK_FILE);
    if stack_file.exists() {
        fs::remove_file(stack_file)?;
    }
    Ok(())
}

/// List all stacks in the repository
pub fn list_all_stacks(repo: &Repository, config: &Config, username: &str) -> Result<Vec<String>> {
    let mut stacks = Vec::new();

    // Get stacks from config
    for name in config.list_stacks() {
        stacks.push(name.to_string());
    }

    // Also scan local branches matching username/stack-name pattern
    let branches = repo.branches(Some(git2::BranchType::Local))?;
    for branch_result in branches {
        let (branch, _) = branch_result?;
        if let Some(name) = branch.name()? {
            if let Some((branch_user, stack_name)) = git::parse_stack_branch(name) {
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
