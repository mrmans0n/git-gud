//! Provider abstraction for GitHub and GitLab
//!
//! Provides a unified interface for working with different git hosting providers.

use git2::Repository;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::gh::{self, CiStatus as GhCiStatus, PrState as GhPrState};
use crate::git;
use crate::glab::{self, CiStatus as GlabCiStatus, MrState as GlabMrState};

/// Supported git hosting providers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    GitHub,
    GitLab,
}

/// Unified PR/MR state across providers
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    Closed,
    Draft,
}

/// Unified CI status across providers
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiStatus {
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
    Unknown,
}

/// Unified PR/MR information
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub state: PrState,
    pub url: String,
    pub draft: bool,
    pub approved: bool,
    pub mergeable: bool,
}

/// Result of creating a PR/MR
#[derive(Debug, Clone)]
pub struct PrCreationResult {
    pub number: u64,
    pub url: String,
}

impl Provider {
    /// Detect provider from config or repository URL
    ///
    /// Priority:
    /// 1. Config `defaults.provider` if set ("github" or "gitlab")
    /// 2. Auto-detect from remote URL (github.com, gitlab.com)
    pub fn detect(repo: &Repository) -> Result<Self> {
        // Try to load config and check for explicit provider setting
        let git_dir = repo.path();
        if let Ok(config) = Config::load(git_dir) {
            if let Some(provider) = config.defaults.provider.as_deref() {
                return Self::from_str(provider);
            }
        }

        // Fallback to URL-based detection
        match git::detect_remote_provider(repo) {
            Ok(git::RemoteProvider::GitHub) => Ok(Provider::GitHub),
            Ok(git::RemoteProvider::GitLab) => Ok(Provider::GitLab),
            Err(e) => Err(e),
        }
    }

    /// Create provider from string ("github" or "gitlab")
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "github" => Ok(Provider::GitHub),
            "gitlab" => Ok(Provider::GitLab),
            _ => Err(GgError::Other(format!(
                "Unknown provider '{}'. Supported: github, gitlab",
                s
            ))),
        }
    }

    /// Convert provider to config string
    #[allow(dead_code)]
    pub fn as_config_str(self) -> &'static str {
        match self {
            Provider::GitHub => "github",
            Provider::GitLab => "gitlab",
        }
    }

    /// Check if CLI tool is installed
    pub fn check_installed(&self) -> Result<()> {
        match self {
            Provider::GitHub => gh::check_gh_installed(),
            Provider::GitLab => glab::check_glab_installed(),
        }
    }

    /// Check if authenticated with provider
    pub fn check_auth(&self) -> Result<()> {
        match self {
            Provider::GitHub => gh::check_gh_auth(),
            Provider::GitLab => glab::check_glab_auth(),
        }
    }

    /// Get current username
    pub fn whoami(&self) -> Result<String> {
        match self {
            Provider::GitHub => gh::whoami(),
            Provider::GitLab => glab::whoami(),
        }
    }

    /// Create a new PR/MR
    pub fn create_pr(
        &self,
        source_branch: &str,
        target_branch: &str,
        title: &str,
        description: &str,
        draft: bool,
    ) -> Result<PrCreationResult> {
        match self {
            Provider::GitHub => {
                let result =
                    gh::create_pr(source_branch, target_branch, title, description, draft)?;
                Ok(PrCreationResult {
                    number: result.number,
                    url: result.url,
                })
            }
            Provider::GitLab => {
                let result =
                    glab::create_mr(source_branch, target_branch, title, description, draft)?;
                Ok(PrCreationResult {
                    number: result.number,
                    url: result.url,
                })
            }
        }
    }

    /// Get PR/MR information
    pub fn get_pr_info(&self, number: u64) -> Result<PrInfo> {
        match self {
            Provider::GitHub => {
                let info = gh::get_pr_info(number)?;
                Ok(PrInfo {
                    number: info.number,
                    title: info.title,
                    state: convert_gh_state(info.state),
                    url: info.url,
                    draft: info.draft,
                    approved: info.approved,
                    mergeable: info.mergeable,
                })
            }
            Provider::GitLab => {
                let info = glab::get_mr_info(number)?;
                Ok(PrInfo {
                    number: info.iid,
                    title: info.title,
                    state: convert_glab_state(info.state),
                    url: info.web_url,
                    draft: info.draft,
                    approved: info.approved,
                    mergeable: info.mergeable,
                })
            }
        }
    }

    /// Update PR/MR base/target branch
    pub fn update_pr_base(&self, number: u64, base_branch: &str) -> Result<()> {
        match self {
            Provider::GitHub => gh::update_pr_base(number, base_branch),
            Provider::GitLab => glab::update_mr_target(number, base_branch),
        }
    }

    /// Update PR/MR description/body
    pub fn update_pr_description(&self, number: u64, description: &str) -> Result<()> {
        match self {
            Provider::GitHub => gh::update_pr_description(number, description),
            Provider::GitLab => glab::update_mr_description(number, description),
        }
    }

    /// Update PR/MR title
    pub fn update_pr_title(&self, number: u64, title: &str) -> Result<()> {
        match self {
            Provider::GitHub => gh::update_pr_title(number, title),
            Provider::GitLab => glab::update_mr_title(number, title),
        }
    }

    /// Merge a PR/MR immediately.
    pub fn merge_pr(&self, number: u64, squash: bool, delete_branch: bool) -> Result<()> {
        match self {
            Provider::GitHub => gh::merge_pr(number, squash, delete_branch),
            Provider::GitLab => glab::merge_mr(number, squash, delete_branch),
        }
    }

    /// Request auto-merge ("merge when pipeline succeeds").
    ///
    /// GitLab only.
    pub fn auto_merge_pr_when_pipeline_succeeds(
        &self,
        number: u64,
        squash: bool,
        delete_branch: bool,
    ) -> Result<()> {
        match self {
            Provider::GitHub => Err(GgError::Other(
                "Auto-merge-on-land is only supported for GitLab".to_string(),
            )),
            Provider::GitLab => {
                glab::auto_merge_mr_when_pipeline_succeeds(number, squash, delete_branch)
            }
        }
    }

    /// Check if PR/MR is approved
    pub fn check_pr_approved(&self, number: u64) -> Result<bool> {
        match self {
            Provider::GitHub => gh::check_pr_approved(number),
            Provider::GitLab => glab::check_mr_approved(number),
        }
    }

    /// Get CI status for PR/MR
    pub fn get_pr_ci_status(&self, number: u64) -> Result<CiStatus> {
        match self {
            Provider::GitHub => {
                let status = gh::get_pr_ci_status(number)?;
                Ok(convert_gh_ci_status(status))
            }
            Provider::GitLab => {
                let status = glab::get_mr_ci_status(number)?;
                Ok(convert_glab_ci_status(status))
            }
        }
    }

    /// Get provider name for display
    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        match self {
            Provider::GitHub => "GitHub",
            Provider::GitLab => "GitLab",
        }
    }

    /// List PRs/MRs for a specific branch
    /// Returns PR/MR numbers for open PRs/MRs with the given branch as source/head
    pub fn list_prs_for_branch(&self, branch: &str) -> Result<Vec<u64>> {
        match self {
            Provider::GitHub => gh::list_prs_for_branch(branch),
            Provider::GitLab => glab::list_mrs_for_branch(branch),
        }
    }

    /// Get PR/MR label (PR or MR)
    pub fn pr_label(&self) -> &'static str {
        match self {
            Provider::GitHub => "PR",
            Provider::GitLab => "MR",
        }
    }

    /// Get PR/MR number prefix (# for GitHub, ! for GitLab)
    pub fn pr_number_prefix(&self) -> &'static str {
        match self {
            Provider::GitHub => "#",
            Provider::GitLab => "!",
        }
    }

    /// Check if merge trains are enabled (GitLab only)
    /// Returns false for GitHub (not supported)
    pub fn check_merge_trains_enabled(&self) -> Result<bool> {
        match self {
            Provider::GitHub => Ok(false),
            Provider::GitLab => glab::check_merge_trains_enabled(),
        }
    }

    /// Add PR/MR to merge train (GitLab only)
    /// Falls back to regular merge for GitHub
    pub fn add_to_merge_train(&self, number: u64) -> Result<()> {
        match self {
            Provider::GitHub => {
                // GitHub doesn't support merge trains, fallback to regular merge
                Err(GgError::Other(
                    "Merge trains are not supported on GitHub".to_string(),
                ))
            }
            Provider::GitLab => glab::add_to_merge_train(number),
        }
    }

    /// Get merge train status (GitLab only)
    /// Returns None for GitHub (not supported)
    pub fn get_merge_train_status(
        &self,
        number: u64,
        target_branch: &str,
    ) -> Result<Option<glab::MergeTrainInfo>> {
        match self {
            Provider::GitHub => Ok(None),
            Provider::GitLab => Ok(Some(glab::get_merge_train_status(number, target_branch)?)),
        }
    }
}

// Conversion helpers

fn convert_gh_state(state: GhPrState) -> PrState {
    match state {
        GhPrState::Open => PrState::Open,
        GhPrState::Merged => PrState::Merged,
        GhPrState::Closed => PrState::Closed,
        GhPrState::Draft => PrState::Draft,
    }
}

fn convert_glab_state(state: GlabMrState) -> PrState {
    match state {
        GlabMrState::Open => PrState::Open,
        GlabMrState::Merged => PrState::Merged,
        GlabMrState::Closed => PrState::Closed,
        GlabMrState::Draft => PrState::Draft,
    }
}

fn convert_gh_ci_status(status: GhCiStatus) -> CiStatus {
    match status {
        GhCiStatus::Pending => CiStatus::Pending,
        GhCiStatus::Running => CiStatus::Running,
        GhCiStatus::Success => CiStatus::Success,
        GhCiStatus::Failed => CiStatus::Failed,
        GhCiStatus::Canceled => CiStatus::Canceled,
        GhCiStatus::Unknown => CiStatus::Unknown,
    }
}

fn convert_glab_ci_status(status: GlabCiStatus) -> CiStatus {
    match status {
        GlabCiStatus::Pending => CiStatus::Pending,
        GlabCiStatus::Running => CiStatus::Running,
        GlabCiStatus::Success => CiStatus::Success,
        GlabCiStatus::Failed => CiStatus::Failed,
        GlabCiStatus::Canceled => CiStatus::Canceled,
        GlabCiStatus::Unknown => CiStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_equality() {
        assert_eq!(Provider::GitHub, Provider::GitHub);
        assert_eq!(Provider::GitLab, Provider::GitLab);
        assert_ne!(Provider::GitHub, Provider::GitLab);
    }

    #[test]
    fn test_pr_state_equality() {
        assert_eq!(PrState::Open, PrState::Open);
        assert_eq!(PrState::Merged, PrState::Merged);
        assert_eq!(PrState::Closed, PrState::Closed);
        assert_eq!(PrState::Draft, PrState::Draft);
        assert_ne!(PrState::Open, PrState::Merged);
    }

    #[test]
    fn test_ci_status_equality() {
        assert_eq!(CiStatus::Success, CiStatus::Success);
        assert_eq!(CiStatus::Failed, CiStatus::Failed);
        assert_eq!(CiStatus::Pending, CiStatus::Pending);
        assert_eq!(CiStatus::Running, CiStatus::Running);
        assert_eq!(CiStatus::Canceled, CiStatus::Canceled);
        assert_eq!(CiStatus::Unknown, CiStatus::Unknown);
        assert_ne!(CiStatus::Success, CiStatus::Failed);
    }

    #[test]
    fn test_provider_name() {
        assert_eq!(Provider::GitHub.name(), "GitHub");
        assert_eq!(Provider::GitLab.name(), "GitLab");
    }

    #[test]
    fn test_provider_pr_label() {
        assert_eq!(Provider::GitHub.pr_label(), "PR");
        assert_eq!(Provider::GitLab.pr_label(), "MR");
    }

    #[test]
    fn test_provider_pr_number_prefix() {
        assert_eq!(Provider::GitHub.pr_number_prefix(), "#");
        assert_eq!(Provider::GitLab.pr_number_prefix(), "!");
    }

    #[test]
    fn test_pr_info_construction() {
        let info = PrInfo {
            number: 42,
            title: "Test PR".to_string(),
            state: PrState::Open,
            url: "https://example.com/pr/42".to_string(),
            draft: false,
            approved: true,
            mergeable: true,
        };
        assert_eq!(info.number, 42);
        assert_eq!(info.title, "Test PR");
        assert_eq!(info.state, PrState::Open);
        assert!(info.approved);
        assert!(info.mergeable);
        assert!(!info.draft);
    }

    #[test]
    fn test_pr_creation_result_construction() {
        let result = PrCreationResult {
            number: 123,
            url: "https://github.com/user/repo/pull/123".to_string(),
        };
        assert_eq!(result.number, 123);
        assert_eq!(result.url, "https://github.com/user/repo/pull/123");
    }

    #[test]
    fn test_pr_creation_result_empty_url() {
        let result = PrCreationResult {
            number: 456,
            url: String::new(),
        };
        assert_eq!(result.number, 456);
        assert!(result.url.is_empty());
    }

    #[test]
    fn test_convert_gh_state() {
        assert_eq!(convert_gh_state(GhPrState::Open), PrState::Open);
        assert_eq!(convert_gh_state(GhPrState::Merged), PrState::Merged);
        assert_eq!(convert_gh_state(GhPrState::Closed), PrState::Closed);
        assert_eq!(convert_gh_state(GhPrState::Draft), PrState::Draft);
    }

    #[test]
    fn test_convert_glab_state() {
        assert_eq!(convert_glab_state(GlabMrState::Open), PrState::Open);
        assert_eq!(convert_glab_state(GlabMrState::Merged), PrState::Merged);
        assert_eq!(convert_glab_state(GlabMrState::Closed), PrState::Closed);
        assert_eq!(convert_glab_state(GlabMrState::Draft), PrState::Draft);
    }

    #[test]
    fn test_convert_gh_ci_status() {
        assert_eq!(convert_gh_ci_status(GhCiStatus::Pending), CiStatus::Pending);
        assert_eq!(convert_gh_ci_status(GhCiStatus::Running), CiStatus::Running);
        assert_eq!(convert_gh_ci_status(GhCiStatus::Success), CiStatus::Success);
        assert_eq!(convert_gh_ci_status(GhCiStatus::Failed), CiStatus::Failed);
        assert_eq!(
            convert_gh_ci_status(GhCiStatus::Canceled),
            CiStatus::Canceled
        );
        assert_eq!(convert_gh_ci_status(GhCiStatus::Unknown), CiStatus::Unknown);
    }

    #[test]
    fn test_convert_glab_ci_status() {
        assert_eq!(
            convert_glab_ci_status(GlabCiStatus::Pending),
            CiStatus::Pending
        );
        assert_eq!(
            convert_glab_ci_status(GlabCiStatus::Running),
            CiStatus::Running
        );
        assert_eq!(
            convert_glab_ci_status(GlabCiStatus::Success),
            CiStatus::Success
        );
        assert_eq!(
            convert_glab_ci_status(GlabCiStatus::Failed),
            CiStatus::Failed
        );
        assert_eq!(
            convert_glab_ci_status(GlabCiStatus::Canceled),
            CiStatus::Canceled
        );
        assert_eq!(
            convert_glab_ci_status(GlabCiStatus::Unknown),
            CiStatus::Unknown
        );
    }

    #[test]
    fn test_provider_from_str() {
        // Valid providers (case-insensitive)
        assert_eq!(Provider::from_str("github").unwrap(), Provider::GitHub);
        assert_eq!(Provider::from_str("GitHub").unwrap(), Provider::GitHub);
        assert_eq!(Provider::from_str("GITHUB").unwrap(), Provider::GitHub);
        assert_eq!(Provider::from_str("gitlab").unwrap(), Provider::GitLab);
        assert_eq!(Provider::from_str("GitLab").unwrap(), Provider::GitLab);
        assert_eq!(Provider::from_str("GITLAB").unwrap(), Provider::GitLab);

        // Invalid providers
        assert!(Provider::from_str("bitbucket").is_err());
        assert!(Provider::from_str("").is_err());
        assert!(Provider::from_str("git").is_err());
    }

    #[test]
    fn test_provider_as_config_str() {
        assert_eq!(Provider::GitHub.as_config_str(), "github");
        assert_eq!(Provider::GitLab.as_config_str(), "gitlab");
    }
}
