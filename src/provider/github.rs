//! GitHub provider implementation
//!
//! Uses the gh CLI for GitHub operations.

use std::process::Command;

use serde::Deserialize;

use crate::error::{GgError, Result};
use crate::provider::{CiStatus, PrInfo, PrState, Provider};

/// GitHub provider using the gh CLI
pub struct GitHubProvider;

/// JSON response from `gh pr view --json`
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPrJson {
    number: u64,
    title: String,
    state: String,
    url: String,
    is_draft: Option<bool>,
    review_decision: Option<String>,
    mergeable: Option<String>,
}

/// JSON response from `gh pr checks --json`
#[derive(Debug, Deserialize)]
struct GhCheckJson {
    state: Option<String>,
    conclusion: Option<String>,
}

impl Provider for GitHubProvider {
    fn name(&self) -> &'static str {
        "GitHub"
    }

    fn pr_prefix(&self) -> &'static str {
        "#"
    }

    fn check_installed(&self) -> Result<()> {
        let output = Command::new("gh").arg("--version").output();

        match output {
            Ok(o) if o.status.success() => Ok(()),
            _ => Err(GgError::GhNotInstalled),
        }
    }

    fn check_auth(&self) -> Result<()> {
        let output = Command::new("gh").args(["auth", "status"]).output()?;

        if output.status.success() {
            Ok(())
        } else {
            Err(GgError::GhNotAuthenticated)
        }
    }

    fn whoami(&self) -> Result<String> {
        let output = Command::new("gh")
            .args(["api", "user", "--jq", ".login"])
            .output()?;

        if !output.status.success() {
            return Err(GgError::GhNotAuthenticated);
        }

        let username = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if username.is_empty() {
            return Err(GgError::GhError(
                "Could not determine GitHub username".to_string(),
            ));
        }

        Ok(username)
    }

    fn create_pr(
        &self,
        source_branch: &str,
        target_branch: &str,
        title: &str,
        description: &str,
        draft: bool,
    ) -> Result<u64> {
        let mut args = vec![
            "pr",
            "create",
            "--head",
            source_branch,
            "--base",
            target_branch,
            "--title",
            title,
            "--body",
            description,
        ];

        if draft {
            args.push("--draft");
        }

        let output = Command::new("gh").args(&args).output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GgError::GhError(format!("Failed to create PR: {}", stderr)));
        }

        // Parse the output to get the PR number
        // gh outputs a URL like https://github.com/owner/repo/pull/123
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Try to extract PR number from URL
        for word in stdout.split_whitespace() {
            if word.contains("/pull/") {
                if let Some(num_str) = word.split("/pull/").nth(1) {
                    let num_str = num_str.trim_end_matches(|c: char| !c.is_ascii_digit());
                    if let Ok(num) = num_str.parse::<u64>() {
                        return Ok(num);
                    }
                }
            }
        }

        Err(GgError::GhError(
            "Could not parse PR number from gh output".to_string(),
        ))
    }

    fn view_pr(&self, number: u64) -> Result<PrInfo> {
        let output = Command::new("gh")
            .args([
                "pr",
                "view",
                &number.to_string(),
                "--json",
                "number,title,state,url,isDraft,reviewDecision,mergeable",
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GgError::GhError(format!(
                "Failed to view PR #{}: {}",
                number, stderr
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let pr_json: GhPrJson = serde_json::from_str(&stdout)
            .map_err(|e| GgError::GhError(format!("Failed to parse PR JSON: {}", e)))?;

        let draft = pr_json.is_draft.unwrap_or(false);

        let state = match pr_json.state.as_str() {
            "MERGED" => PrState::Merged,
            "CLOSED" => PrState::Closed,
            _ if draft => PrState::Draft,
            _ => PrState::Open,
        };

        // reviewDecision: APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED, or null
        let approved = pr_json.review_decision.as_deref() == Some("APPROVED");

        // mergeable: MERGEABLE, CONFLICTING, UNKNOWN
        let mergeable =
            pr_json.mergeable.as_deref() == Some("MERGEABLE") && state == PrState::Open && !draft;

        Ok(PrInfo {
            number: pr_json.number,
            title: pr_json.title,
            state,
            web_url: pr_json.url,
            draft,
            approved,
            mergeable,
        })
    }

    fn update_pr_target(&self, number: u64, target_branch: &str) -> Result<()> {
        let output = Command::new("gh")
            .args(["pr", "edit", &number.to_string(), "--base", target_branch])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GgError::GhError(format!(
                "Failed to update PR #{}: {}",
                number, stderr
            )));
        }

        Ok(())
    }

    fn merge_pr(&self, number: u64, squash: bool, delete_branch: bool) -> Result<()> {
        let pr_num_str = number.to_string();
        let mut args = vec!["pr", "merge", &pr_num_str];

        if squash {
            args.push("--squash");
        } else {
            args.push("--merge");
        }

        if delete_branch {
            args.push("--delete-branch");
        }

        let output = Command::new("gh").args(&args).output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GgError::GhError(format!(
                "Failed to merge PR #{}: {}",
                number, stderr
            )));
        }

        Ok(())
    }

    fn check_approved(&self, number: u64) -> Result<bool> {
        let output = Command::new("gh")
            .args([
                "pr",
                "view",
                &number.to_string(),
                "--json",
                "reviewDecision",
            ])
            .output()?;

        if !output.status.success() {
            // If the call fails, assume not approved
            return Ok(false);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse JSON to check reviewDecision
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ReviewResponse {
            review_decision: Option<String>,
        }

        if let Ok(response) = serde_json::from_str::<ReviewResponse>(&stdout) {
            Ok(response.review_decision.as_deref() == Some("APPROVED"))
        } else {
            Ok(false)
        }
    }

    fn get_ci_status(&self, number: u64) -> Result<CiStatus> {
        let output = Command::new("gh")
            .args([
                "pr",
                "checks",
                &number.to_string(),
                "--json",
                "state,conclusion",
            ])
            .output()?;

        if !output.status.success() {
            return Ok(CiStatus::Unknown);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse JSON array of checks
        let checks: Vec<GhCheckJson> = match serde_json::from_str(&stdout) {
            Ok(c) => c,
            Err(_) => return Ok(CiStatus::Unknown),
        };

        if checks.is_empty() {
            return Ok(CiStatus::Unknown);
        }

        // Aggregate check states
        let mut has_failure = false;
        let mut has_pending = false;
        let mut has_running = false;
        let mut has_canceled = false;

        for check in &checks {
            match check.state.as_deref() {
                Some("PENDING") | Some("QUEUED") => has_pending = true,
                Some("IN_PROGRESS") => has_running = true,
                Some("COMPLETED") => match check.conclusion.as_deref() {
                    Some("FAILURE") | Some("TIMED_OUT") | Some("STARTUP_FAILURE") => {
                        has_failure = true
                    }
                    Some("CANCELLED") => has_canceled = true,
                    Some("SUCCESS") | Some("NEUTRAL") | Some("SKIPPED") => {}
                    _ => {}
                },
                _ => {}
            }
        }

        // Determine overall status (failure > running > pending > canceled > success)
        if has_failure {
            Ok(CiStatus::Failed)
        } else if has_running {
            Ok(CiStatus::Running)
        } else if has_pending {
            Ok(CiStatus::Pending)
        } else if has_canceled {
            Ok(CiStatus::Canceled)
        } else {
            Ok(CiStatus::Success)
        }
    }
}
