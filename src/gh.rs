//! GitHub CLI (gh) integration
//!
//! Wraps gh subprocess calls for PR management.

use std::process::Command;

use serde::Deserialize;

use crate::error::{GgError, Result};

/// PR state from GitHub
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
    Closed,
    Draft,
}

/// PR information from gh
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

/// JSON response from `gh pr view --json`
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPrJson {
    number: u64,
    title: String,
    state: String,
    url: String,
    #[serde(default)]
    is_draft: bool,
    mergeable: Option<String>,
    #[serde(default)]
    reviews: Vec<GhReview>,
}

#[derive(Debug, Deserialize)]
struct GhReview {
    state: String,
}

/// Check if gh is installed
pub fn check_gh_installed() -> Result<()> {
    let output = Command::new("gh").arg("--version").output();

    match output {
        Ok(o) if o.status.success() => Ok(()),
        _ => Err(GgError::Other("gh CLI not installed".to_string())),
    }
}

/// Check if authenticated with GitHub
pub fn check_gh_auth() -> Result<()> {
    let output = Command::new("gh").args(["auth", "status"]).output()?;

    if output.status.success() {
        Ok(())
    } else {
        Err(GgError::Other(
            "Not authenticated with GitHub. Run `gh auth login` first.".to_string(),
        ))
    }
}

/// Get the current GitHub username
pub fn whoami() -> Result<String> {
    let output = Command::new("gh")
        .args(["api", "user", "--jq", ".login"])
        .output()?;

    if !output.status.success() {
        return Err(GgError::Other(
            "Could not determine GitHub username".to_string(),
        ));
    }

    let username = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if username.is_empty() {
        return Err(GgError::Other(
            "Could not determine GitHub username".to_string(),
        ));
    }

    Ok(username)
}

/// Result of creating a PR
#[derive(Debug, Clone)]
pub struct PrCreationResult {
    pub number: u64,
    pub url: String,
}

/// Create a new PR
pub fn create_pr(
    source_branch: &str,
    target_branch: &str,
    title: &str,
    description: &str,
    draft: bool,
) -> Result<PrCreationResult> {
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
        return Err(GgError::Other(format!("Failed to create PR: {}", stderr)));
    }

    // Parse the output to get the PR URL and extract number
    let stdout = String::from_utf8_lossy(&output.stdout);

    // gh outputs a URL like https://github.com/user/repo/pull/123
    for line in stdout.lines() {
        if line.contains("/pull/") {
            let url = line.trim().to_string();
            if let Some(num_str) = line.split("/pull/").nth(1) {
                let num_str = num_str.trim();
                if let Ok(num) = num_str.parse::<u64>() {
                    return Ok(PrCreationResult { number: num, url });
                }
            }
        }
    }

    Err(GgError::Other(
        "Could not parse PR number from gh output".to_string(),
    ))
}

/// View PR information
pub fn view_pr(pr_number: u64) -> Result<PrInfo> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "number,title,state,url,isDraft,mergeable,reviews",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to view PR #{}: {}",
            pr_number, stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pr_json: GhPrJson = serde_json::from_str(&stdout)
        .map_err(|e| GgError::Other(format!("Failed to parse PR JSON: {}", e)))?;

    let state = match pr_json.state.to_uppercase().as_str() {
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ if pr_json.is_draft => PrState::Draft,
        _ => PrState::Open,
    };

    let approved = pr_json.reviews.iter().any(|r| r.state == "APPROVED");

    let mergeable = pr_json.mergeable.as_deref() == Some("MERGEABLE");

    Ok(PrInfo {
        number: pr_json.number,
        title: pr_json.title,
        state,
        url: pr_json.url,
        draft: pr_json.is_draft,
        approved,
        mergeable,
    })
}

/// Alias for view_pr for compatibility
pub fn get_pr_info(pr_number: u64) -> Result<PrInfo> {
    view_pr(pr_number)
}

/// Update PR base branch
pub fn update_pr_base(pr_number: u64, base_branch: &str) -> Result<()> {
    let output = Command::new("gh")
        .args(["pr", "edit", &pr_number.to_string(), "--base", base_branch])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to update PR #{}: {}",
            pr_number, stderr
        )));
    }

    Ok(())
}

/// Merge a PR
pub fn merge_pr(pr_number: u64, squash: bool, delete_branch: bool) -> Result<()> {
    let pr_num_str = pr_number.to_string();
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
        return Err(GgError::Other(format!(
            "Failed to merge PR #{}: {}",
            pr_number, stderr
        )));
    }

    Ok(())
}

/// Mark PR as ready for review (convert from draft)
#[allow(dead_code)]
pub fn mark_ready_for_review(pr_number: u64) -> Result<()> {
    let output = Command::new("gh")
        .args(["pr", "ready", &pr_number.to_string()])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to mark PR #{} as ready: {}",
            pr_number, stderr
        )));
    }

    Ok(())
}

/// Approve a PR
#[allow(dead_code)]
pub fn approve_pr(pr_number: u64) -> Result<()> {
    let output = Command::new("gh")
        .args(["pr", "review", &pr_number.to_string(), "--approve"])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to approve PR #{}: {}",
            pr_number, stderr
        )));
    }

    Ok(())
}

/// Check if PR has required approvals
pub fn check_pr_approved(pr_number: u64) -> Result<bool> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "reviewDecision",
            "--jq",
            ".reviewDecision",
        ])
        .output()?;

    if !output.status.success() {
        return Ok(false);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // APPROVED = explicitly approved
    // Empty = no review required (e.g., no branch protection rules requiring review)
    // "" = same as empty
    Ok(stdout == "APPROVED" || stdout.is_empty() || stdout == "null")
}

/// Get CI status for a PR
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum CiStatus {
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
    Unknown,
}

pub fn get_pr_ci_status(pr_number: u64) -> Result<CiStatus> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "statusCheckRollup",
            "--jq",
            ".statusCheckRollup[].conclusion",
        ])
        .output()?;

    if !output.status.success() {
        return Ok(CiStatus::Unknown);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Check all conclusions - if any failed, overall is failed
    let mut has_success = false;
    let mut has_pending = false;

    for line in stdout.lines() {
        match line.trim().to_uppercase().as_str() {
            "FAILURE" | "FAILED" => return Ok(CiStatus::Failed),
            "SUCCESS" => has_success = true,
            "PENDING" | "QUEUED" => has_pending = true,
            "CANCELLED" | "CANCELED" => return Ok(CiStatus::Canceled),
            _ => {}
        }
    }

    if has_pending {
        Ok(CiStatus::Pending)
    } else if has_success {
        Ok(CiStatus::Success)
    } else {
        Ok(CiStatus::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_ne!(CiStatus::Success, CiStatus::Failed);
    }

    #[test]
    fn test_pr_info_construction() {
        let info = PrInfo {
            number: 42,
            title: "Test PR".to_string(),
            state: PrState::Open,
            url: "https://github.com/test/repo/pull/42".to_string(),
            draft: false,
            approved: true,
            mergeable: true,
        };
        assert_eq!(info.number, 42);
        assert_eq!(info.title, "Test PR");
        assert_eq!(info.state, PrState::Open);
        assert!(info.approved);
        assert!(info.mergeable);
    }

    #[test]
    fn test_gh_pr_json_deserialization() {
        let json = r#"{
            "number": 123,
            "title": "My PR",
            "state": "OPEN",
            "url": "https://github.com/owner/repo/pull/123",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "reviews": [{"state": "APPROVED"}]
        }"#;

        let parsed: GhPrJson = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.number, 123);
        assert_eq!(parsed.title, "My PR");
        assert_eq!(parsed.state, "OPEN");
        assert!(!parsed.is_draft);
        assert_eq!(parsed.mergeable, Some("MERGEABLE".to_string()));
        assert_eq!(parsed.reviews.len(), 1);
        assert_eq!(parsed.reviews[0].state, "APPROVED");
    }

    #[test]
    fn test_gh_pr_json_with_missing_optional_fields() {
        let json = r#"{
            "number": 456,
            "title": "Draft PR",
            "state": "OPEN",
            "url": "https://github.com/owner/repo/pull/456"
        }"#;

        let parsed: GhPrJson = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.number, 456);
        assert!(!parsed.is_draft); // defaults to false
        assert!(parsed.mergeable.is_none());
        assert!(parsed.reviews.is_empty()); // defaults to empty
    }

    #[test]
    fn test_pr_creation_result_construction() {
        let result = PrCreationResult {
            number: 42,
            url: "https://github.com/user/repo/pull/42".to_string(),
        };
        assert_eq!(result.number, 42);
        assert_eq!(result.url, "https://github.com/user/repo/pull/42");
    }

    #[test]
    fn test_pr_creation_result_clone() {
        let result = PrCreationResult {
            number: 123,
            url: "https://github.com/test/repo/pull/123".to_string(),
        };
        let cloned = result.clone();
        assert_eq!(cloned.number, 123);
        assert_eq!(cloned.url, "https://github.com/test/repo/pull/123");
    }
}
