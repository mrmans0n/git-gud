//! Provider abstraction for GitHub and GitLab
//!
//! Provides a unified interface for working with different git hosting providers.

use git2::Repository;

use crate::error::Result;
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

impl Provider {
    /// Detect provider from repository
    pub fn detect(repo: &Repository) -> Result<Self> {
        match git::detect_remote_provider(repo) {
            Ok(git::RemoteProvider::GitHub) => Ok(Provider::GitHub),
            Ok(git::RemoteProvider::GitLab) => Ok(Provider::GitLab),
            Err(e) => Err(e),
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
    ) -> Result<u64> {
        match self {
            Provider::GitHub => {
                gh::create_pr(source_branch, target_branch, title, description, draft)
            }
            Provider::GitLab => {
                glab::create_mr(source_branch, target_branch, title, description, draft)
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

    /// Merge a PR/MR
    pub fn merge_pr(&self, number: u64, squash: bool, delete_branch: bool) -> Result<()> {
        match self {
            Provider::GitHub => gh::merge_pr(number, squash, delete_branch),
            Provider::GitLab => glab::merge_mr(number, squash, delete_branch),
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

    /// Get PR/MR label (PR or MR)
    pub fn pr_label(&self) -> &'static str {
        match self {
            Provider::GitHub => "PR",
            Provider::GitLab => "MR",
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
}
