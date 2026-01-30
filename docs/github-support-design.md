# GitHub Support Design Document

## Overview

This document describes the architectural changes needed to add GitHub support to git-gud, which currently only supports GitLab.

## Goals

1. Support GitHub as a first-class provider alongside GitLab
2. Auto-detect provider from remote URL, with manual configuration override
3. Maintain feature parity between providers where possible
4. Use native CLI tools (`gh` for GitHub, `glab` for GitLab)
5. Keep platform-native notation (PR #123 for GitHub, MR !123 for GitLab)

## Current Architecture

### Provider Coupling

Currently, GitLab support is tightly coupled throughout the codebase:

```
src/
├── glab.rs          # All GitLab/glab CLI interactions
├── stack.rs         # Uses glab directly for MR refresh
├── commands/
│   ├── sync.rs      # Uses glab for MR creation/update
│   ├── land.rs      # Uses glab for MR merge
│   ├── setup.rs     # Uses glab for whoami
│   └── ls.rs        # Uses glab for MR status
└── error.rs         # GitLab-specific errors
```

### Key glab.rs Functions

| Function | Purpose |
|----------|---------|
| `check_glab_installed()` | Verify glab CLI available |
| `check_glab_auth()` | Verify authenticated |
| `whoami()` | Get current username |
| `create_mr()` | Create merge request |
| `view_mr()` | Get MR info |
| `update_mr_target()` | Change MR target branch |
| `merge_mr()` | Merge MR |
| `check_mr_approved()` | Check approval status |
| `get_mr_ci_status()` | Get CI/pipeline status |

## Proposed Architecture

### Provider Trait

Create a `Provider` trait that abstracts platform-specific operations:

```rust
// src/provider/mod.rs

pub mod github;
pub mod gitlab;

use crate::error::Result;

/// Pull/Merge Request state
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    Closed,
    Draft,
}

/// Pull/Merge Request information
#[derive(Debug, Clone)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub state: PrState,
    pub web_url: String,
    pub draft: bool,
    pub approved: bool,
    pub mergeable: bool,
}

/// CI/Check status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiStatus {
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
    Unknown,
}

/// Provider trait for Git hosting platforms
pub trait Provider: Send + Sync {
    /// Provider name for display
    fn name(&self) -> &'static str;
    
    /// PR/MR notation prefix (# for GitHub, ! for GitLab)
    fn pr_prefix(&self) -> &'static str;
    
    /// Check if CLI tool is installed
    fn check_installed(&self) -> Result<()>;
    
    /// Check if authenticated
    fn check_auth(&self) -> Result<()>;
    
    /// Get current username
    fn whoami(&self) -> Result<String>;
    
    /// Create a pull/merge request
    fn create_pr(
        &self,
        source_branch: &str,
        target_branch: &str,
        title: &str,
        description: &str,
        draft: bool,
    ) -> Result<u64>;
    
    /// View PR/MR information
    fn view_pr(&self, number: u64) -> Result<PrInfo>;
    
    /// Update PR/MR target branch
    fn update_pr_target(&self, number: u64, target_branch: &str) -> Result<()>;
    
    /// Merge PR/MR
    fn merge_pr(&self, number: u64, squash: bool, delete_branch: bool) -> Result<()>;
    
    /// Check if PR/MR is approved
    fn check_approved(&self, number: u64) -> Result<bool>;
    
    /// Get CI status for PR/MR
    fn get_ci_status(&self, number: u64) -> Result<CiStatus>;
}
```

### Provider Implementations

#### GitLab Provider

```rust
// src/provider/gitlab.rs

pub struct GitLabProvider;

impl Provider for GitLabProvider {
    fn name(&self) -> &'static str { "GitLab" }
    fn pr_prefix(&self) -> &'static str { "!" }
    
    // ... wrap existing glab.rs functions
}
```

#### GitHub Provider

```rust
// src/provider/github.rs

pub struct GitHubProvider;

impl Provider for GitHubProvider {
    fn name(&self) -> &'static str { "GitHub" }
    fn pr_prefix(&self) -> &'static str { "#" }
    
    fn check_installed(&self) -> Result<()> {
        // Check `gh --version`
    }
    
    fn check_auth(&self) -> Result<()> {
        // Check `gh auth status`
    }
    
    fn whoami(&self) -> Result<String> {
        // `gh api user --jq '.login'`
    }
    
    fn create_pr(&self, source: &str, target: &str, title: &str, desc: &str, draft: bool) -> Result<u64> {
        // `gh pr create --head <source> --base <target> --title <title> --body <desc> [--draft]`
    }
    
    fn view_pr(&self, number: u64) -> Result<PrInfo> {
        // `gh pr view <number> --json number,title,state,url,isDraft,reviewDecision,mergeable`
    }
    
    fn update_pr_target(&self, number: u64, target: &str) -> Result<()> {
        // `gh pr edit <number> --base <target>`
    }
    
    fn merge_pr(&self, number: u64, squash: bool, delete_branch: bool) -> Result<()> {
        // `gh pr merge <number> [--squash] [--delete-branch]`
    }
    
    fn check_approved(&self, number: u64) -> Result<bool> {
        // `gh pr view <number> --json reviewDecision`
        // reviewDecision: APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED, or null
    }
    
    fn get_ci_status(&self, number: u64) -> Result<CiStatus> {
        // `gh pr checks <number> --json bucket,state`
    }
}
```

### Configuration Changes

Update `config.rs` to include provider configuration:

```rust
// src/config.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    GitHub,
    GitLab,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Defaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_username: Option<String>,
    
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lint: Vec<String>,
    
    /// Git hosting provider (auto-detected if not set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderType>,
}
```

### Provider Detection

```rust
// src/provider/mod.rs

/// Detect provider from git remote URL
pub fn detect_provider(repo: &Repository) -> Option<ProviderType> {
    let remote = repo.find_remote("origin").ok()?;
    let url = remote.url()?;
    
    if url.contains("github.com") {
        Some(ProviderType::GitHub)
    } else if url.contains("gitlab.com") || url.contains("gitlab.") {
        Some(ProviderType::GitLab)
    } else {
        None
    }
}

/// Get provider instance based on config (with auto-detection fallback)
pub fn get_provider(config: &Config, repo: &Repository) -> Result<Box<dyn Provider>> {
    let provider_type = config.defaults.provider
        .clone()
        .or_else(|| detect_provider(repo))
        .ok_or(GgError::ProviderNotConfigured)?;
    
    match provider_type {
        ProviderType::GitHub => Ok(Box::new(GitHubProvider)),
        ProviderType::GitLab => Ok(Box::new(GitLabProvider)),
    }
}
```

### Error Types

Add new error variants:

```rust
// src/error.rs

#[derive(Error, Debug)]
pub enum GgError {
    // ... existing variants ...
    
    #[error("Could not detect git provider. Run `gg setup` to configure.")]
    ProviderNotConfigured,
    
    #[error("gh is not installed. Install from https://cli.github.com")]
    GhNotInstalled,
    
    #[error("Not authenticated with GitHub. Run `gh auth login` first.")]
    GhNotAuthenticated,
    
    #[error("gh command failed: {0}")]
    GhError(String),
}
```

## GitHub-Specific Considerations

### Approval Handling

GitHub's approval model differs from GitLab:

| GitLab | GitHub |
|--------|--------|
| Explicit approval required | Review decision (APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED, null) |
| Single approval state | Multiple reviews possible |
| Approval rules in project settings | Branch protection rules |

**Recommendation for `gg land`:**
- Check `reviewDecision == "APPROVED"` for branch-protected repos
- For repos without required reviews, allow landing if CI passes
- Add `--force` flag to bypass approval check

### CI Status Mapping

| GitHub Check State | CiStatus |
|-------------------|----------|
| pending | Pending |
| queued | Pending |
| in_progress | Running |
| completed + conclusion:success | Success |
| completed + conclusion:failure | Failed |
| completed + conclusion:cancelled | Canceled |
| completed + conclusion:skipped | Success |

### Draft PRs

Both platforms support draft PRs with similar semantics:
- GitHub: `--draft` flag, `isDraft` field
- GitLab: `--draft` flag, `draft`/`work_in_progress` fields

## File Structure After Refactor

```
src/
├── provider/
│   ├── mod.rs       # Provider trait + detection
│   ├── github.rs    # GitHub implementation
│   └── gitlab.rs    # GitLab implementation (refactored from glab.rs)
├── stack.rs         # Uses Provider trait
├── config.rs        # Updated with provider config
├── error.rs         # Updated with new error types
├── git.rs           # Unchanged
├── main.rs          # Unchanged
└── commands/
    ├── sync.rs      # Uses Provider trait
    ├── land.rs      # Uses Provider trait
    ├── setup.rs     # Updated to configure provider
    ├── ls.rs        # Uses Provider trait
    └── ...          # Others unchanged
```

## Migration Path

1. **Phase 1: Introduce Provider trait**
   - Create `src/provider/mod.rs` with trait definition
   - Create `src/provider/gitlab.rs` wrapping existing `glab.rs` functions
   - Update commands to use `GitLabProvider` directly (no behavioral change)

2. **Phase 2: Add GitHub support**
   - Create `src/provider/github.rs`
   - Add provider detection logic
   - Update `config.rs` with provider field
   - Update `setup` command to prompt for provider

3. **Phase 3: Refactor commands**
   - Update all commands to use `get_provider()` instead of direct GitLab calls
   - Remove old `glab.rs` (now in `provider/gitlab.rs`)
   - Update error messages to be provider-agnostic

4. **Phase 4: Testing & Polish**
   - Test with both GitHub and GitLab repos
   - Add provider indicator to `gg ls` output
   - Document dual-provider support

## gh CLI Commands Reference

| Operation | Command |
|-----------|---------|
| Check installed | `gh --version` |
| Check auth | `gh auth status` |
| Get username | `gh api user --jq '.login'` |
| Create PR | `gh pr create --head <src> --base <target> --title <t> --body <b> [--draft]` |
| View PR | `gh pr view <n> --json number,title,state,url,isDraft,reviewDecision,mergeable` |
| Edit PR base | `gh pr edit <n> --base <target>` |
| Merge PR | `gh pr merge <n> [--squash] [--delete-branch] [--admin]` |
| Check status | `gh pr checks <n> --json name,state,conclusion` |
| List PRs | `gh pr list --author @me --json number,title,state,headRefName` |

## Open Questions

1. **Self-hosted instances:** Should we support GitHub Enterprise and self-hosted GitLab? (Can be deferred)
2. **Multiple remotes:** What if a repo has both GitHub and GitLab remotes? (Use origin, add `--remote` flag later)
3. **Feature flags:** Should provider-specific features be gated? (Start with common denominator)

## Summary

The refactor introduces a `Provider` trait that abstracts Git hosting platform operations, enabling git-gud to work with both GitHub and GitLab. The implementation uses native CLI tools (`gh` and `glab`) for reliability and maintains platform-specific conventions (PR vs MR notation). Auto-detection from remote URLs provides sensible defaults while allowing manual configuration override.
