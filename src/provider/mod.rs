//! Provider abstraction for Git hosting platforms (GitHub, GitLab)
//!
//! This module provides a unified interface for interacting with different
//! Git hosting platforms, enabling git-gud to work with both GitHub and GitLab.

pub mod github;
pub mod gitlab;

use git2::Repository;

use crate::config::Config;
use crate::error::{GgError, Result};

/// Pull/Merge Request state
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    Closed,
    Draft,
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

/// Pull/Merge Request information
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used for display/future features
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub state: PrState,
    pub web_url: String,
    pub draft: bool,
    pub approved: bool,
    pub mergeable: bool,
}

/// Provider type enum for configuration
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    GitHub,
    GitLab,
}

/// Provider trait for Git hosting platforms
#[allow(dead_code)] // name() reserved for future use in UI
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
    let provider_type = config
        .defaults
        .provider
        .clone()
        .or_else(|| detect_provider(repo))
        .ok_or(GgError::ProviderNotConfigured)?;

    match provider_type {
        ProviderType::GitHub => Ok(Box::new(github::GitHubProvider)),
        ProviderType::GitLab => Ok(Box::new(gitlab::GitLabProvider)),
    }
}
