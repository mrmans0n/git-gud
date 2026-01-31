//! GitLab provider implementation
//!
//! Wraps the existing glab module to implement the Provider trait.

use crate::error::Result;
use crate::glab;
use crate::provider::{CiStatus, PrInfo, PrState, Provider};

/// GitLab provider using the glab CLI
pub struct GitLabProvider;

impl Provider for GitLabProvider {
    fn name(&self) -> &'static str {
        "GitLab"
    }

    fn pr_prefix(&self) -> &'static str {
        "!"
    }

    fn check_installed(&self) -> Result<()> {
        glab::check_glab_installed()
    }

    fn check_auth(&self) -> Result<()> {
        glab::check_glab_auth()
    }

    fn whoami(&self) -> Result<String> {
        glab::whoami()
    }

    fn create_pr(
        &self,
        source_branch: &str,
        target_branch: &str,
        title: &str,
        description: &str,
        draft: bool,
    ) -> Result<u64> {
        glab::create_mr(source_branch, target_branch, title, description, draft)
    }

    fn view_pr(&self, number: u64) -> Result<PrInfo> {
        let mr_info = glab::view_mr(number)?;
        Ok(PrInfo {
            number: mr_info.iid,
            title: mr_info.title,
            state: convert_mr_state(mr_info.state),
            web_url: mr_info.web_url,
            draft: mr_info.draft,
            approved: mr_info.approved,
            mergeable: mr_info.mergeable,
        })
    }

    fn update_pr_target(&self, number: u64, target_branch: &str) -> Result<()> {
        glab::update_mr_target(number, target_branch)
    }

    fn merge_pr(&self, number: u64, squash: bool, delete_branch: bool) -> Result<()> {
        glab::merge_mr(number, squash, delete_branch)
    }

    fn check_approved(&self, number: u64) -> Result<bool> {
        glab::check_mr_approved(number)
    }

    fn get_ci_status(&self, number: u64) -> Result<CiStatus> {
        let status = glab::get_mr_ci_status(number)?;
        Ok(convert_ci_status(status))
    }
}

/// Convert glab::MrState to provider::PrState
fn convert_mr_state(state: glab::MrState) -> PrState {
    match state {
        glab::MrState::Open => PrState::Open,
        glab::MrState::Merged => PrState::Merged,
        glab::MrState::Closed => PrState::Closed,
        glab::MrState::Draft => PrState::Draft,
    }
}

/// Convert glab::CiStatus to provider::CiStatus
fn convert_ci_status(status: glab::CiStatus) -> CiStatus {
    match status {
        glab::CiStatus::Pending => CiStatus::Pending,
        glab::CiStatus::Running => CiStatus::Running,
        glab::CiStatus::Success => CiStatus::Success,
        glab::CiStatus::Failed => CiStatus::Failed,
        glab::CiStatus::Canceled => CiStatus::Canceled,
        glab::CiStatus::Unknown => CiStatus::Unknown,
    }
}
