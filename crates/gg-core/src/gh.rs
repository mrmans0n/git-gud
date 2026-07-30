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
    pub head_branch: Option<String>,
    pub draft: bool,
    pub approved: bool,
    pub mergeable: bool,
    pub changes_requested: bool,
}

/// The fields needed to display a PR in the review inbox.
#[derive(Debug, Clone)]
pub struct InboxPrSnapshot {
    pub state: PrState,
    pub url: String,
    pub approved: bool,
    pub changes_requested: bool,
    pub mergeable: bool,
    pub ci_status: Option<CiStatus>,
}

/// JSON response from `gh pr view --json`
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GhPrJson {
    number: u64,
    title: String,
    state: String,
    url: String,
    head_ref_name: Option<String>,
    #[serde(default)]
    is_draft: bool,
    mergeable: Option<String>,
    #[serde(default)]
    reviews: Vec<GhReview>,
    review_decision: Option<String>,
    #[serde(default)]
    status_check_rollup: Vec<GhStatusCheck>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GhReview {
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhStatusCheck {
    conclusion: Option<String>,
    status: Option<String>,
    state: Option<String>,
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
///
/// Distinguishes between actual auth failures and network errors:
/// - Returns `Ok(())` if authenticated
/// - Returns `Err(GgError::NetworkError(...))` if a network error is detected
/// - Returns `Err(GgError::Other(...))` for actual auth failures
pub fn check_gh_auth() -> Result<()> {
    let output = Command::new("gh").args(["auth", "status"]).output()?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{} {}", stderr, stdout);

    if crate::error::is_network_error(&combined) {
        return Err(GgError::NetworkError(
            "Could not verify GitHub authentication (network error). Check your connection."
                .to_string(),
        ));
    }

    Err(GgError::Other(
        "Not authenticated with GitHub. Run `gh auth login` first.".to_string(),
    ))
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
            "number,title,state,url,headRefName,isDraft,mergeable,reviews,reviewDecision",
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

    let approved = pr_json.review_decision.as_deref() == Some("APPROVED");
    let changes_requested = pr_json.review_decision.as_deref() == Some("CHANGES_REQUESTED");

    let mergeable = pr_json.mergeable.as_deref() == Some("MERGEABLE");

    Ok(PrInfo {
        number: pr_json.number,
        title: pr_json.title,
        state,
        url: pr_json.url,
        head_branch: pr_json.head_ref_name,
        draft: pr_json.is_draft,
        approved,
        mergeable,
        changes_requested,
    })
}

/// Close a PR without merging.
pub fn close_pr(pr_number: u64) -> Result<()> {
    let output = Command::new("gh")
        .args(["pr", "close", &pr_number.to_string()])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to close PR #{}: {}",
            pr_number, stderr
        )));
    }

    Ok(())
}

/// Alias for view_pr for compatibility
pub fn get_pr_info(pr_number: u64) -> Result<PrInfo> {
    view_pr(pr_number)
}

/// Get all inbox fields for a PR with one `gh pr view` request.
pub fn get_inbox_snapshot(pr_number: u64) -> Result<InboxPrSnapshot> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "number,title,state,url,headRefName,isDraft,mergeable,reviewDecision,statusCheckRollup",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to view PR #{}: {}",
            pr_number, stderr
        )));
    }

    parse_inbox_snapshot(&output.stdout)
}

fn parse_inbox_snapshot(bytes: &[u8]) -> Result<InboxPrSnapshot> {
    let pr_json: GhPrJson = serde_json::from_slice(bytes)
        .map_err(|e| GgError::Other(format!("Failed to parse PR JSON: {}", e)))?;

    let state = match pr_json.state.to_uppercase().as_str() {
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ if pr_json.is_draft => PrState::Draft,
        _ => PrState::Open,
    };

    Ok(InboxPrSnapshot {
        state,
        url: pr_json.url,
        approved: matches!(pr_json.review_decision.as_deref(), Some("APPROVED") | None),
        changes_requested: pr_json.review_decision.as_deref() == Some("CHANGES_REQUESTED"),
        mergeable: pr_json.mergeable.as_deref() == Some("MERGEABLE"),
        ci_status: aggregate_status_checks(&pr_json.status_check_rollup),
    })
}

fn aggregate_status_checks(checks: &[GhStatusCheck]) -> Option<CiStatus> {
    if checks.is_empty() {
        return None;
    }

    let mut has_canceled = false;
    let mut has_pending = false;
    let mut has_success = false;

    for check in checks {
        let conclusion = check.conclusion.as_deref().map(str::to_uppercase);
        let status = check.status.as_deref().map(str::to_uppercase);
        let state = check.state.as_deref().map(str::to_uppercase);

        for value in [conclusion.as_deref(), state.as_deref()]
            .into_iter()
            .flatten()
        {
            match value {
                "FAILURE" | "FAILED" | "TIMED_OUT" | "ACTION_REQUIRED" | "ERROR" | "STALE"
                | "STARTUP_FAILURE" => return Some(CiStatus::Failed),
                "CANCELLED" | "CANCELED" => has_canceled = true,
                "EXPECTED" | "PENDING" | "QUEUED" | "IN_PROGRESS" => has_pending = true,
                "SUCCESS" | "NEUTRAL" | "SKIPPED" => has_success = true,
                _ => {}
            }
        }

        if status.as_deref().is_some_and(|value| value != "COMPLETED")
            || (check.state.is_none()
                && check
                    .conclusion
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty()))
        {
            has_pending = true;
        }
    }

    if has_canceled {
        Some(CiStatus::Canceled)
    } else if has_pending {
        Some(CiStatus::Pending)
    } else if has_success {
        Some(CiStatus::Success)
    } else {
        Some(CiStatus::Unknown)
    }
}

/// Convert an existing PR to draft (GitHub only)
pub fn convert_pr_to_draft(pr_number: u64) -> Result<()> {
    let output = Command::new("gh")
        .args(["pr", "ready", "--undo", &pr_number.to_string()])
        .output()?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Best-effort: if it's already a draft, gh may still return a non-zero status in some cases.
    if stderr.to_lowercase().contains("already") && stderr.to_lowercase().contains("draft") {
        return Ok(());
    }

    Err(GgError::Other(format!(
        "Failed to convert PR #{} to draft: {}",
        pr_number, stderr
    )))
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

/// Get PR body text
pub fn get_pr_body(pr_number: u64) -> Result<String> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "body",
            "--jq",
            ".body",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to get PR #{} body: {}",
            pr_number, stderr
        )));
    }

    let body = String::from_utf8_lossy(&output.stdout);
    // Strip only the single trailing newline that `gh --jq` appends,
    // preserving any user-authored trailing whitespace.
    let body = body.strip_suffix('\n').unwrap_or(&body).to_string();
    Ok(body)
}

/// Update PR description/body
pub fn update_pr_description(pr_number: u64, description: &str) -> Result<()> {
    let output = Command::new("gh")
        .args(["pr", "edit", &pr_number.to_string(), "--body", description])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to update PR #{} description: {}",
            pr_number, stderr
        )));
    }

    Ok(())
}

/// Update PR title
pub fn update_pr_title(pr_number: u64, title: &str) -> Result<()> {
    let output = Command::new("gh")
        .args(["pr", "edit", &pr_number.to_string(), "--title", title])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to update PR #{} title: {}",
            pr_number, stderr
        )));
    }

    Ok(())
}

/// Merge a PR
pub fn merge_pr(pr_number: u64, squash: bool, delete_branch: bool, admin: bool) -> Result<()> {
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

    if admin {
        args.push("--admin");
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

/// List PRs for a specific head branch
/// Returns a list of PR numbers for open PRs with the given head branch
pub fn list_prs_for_branch(branch: &str) -> Result<Vec<u64>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--head",
            branch,
            "--json",
            "number",
            "--jq",
            ".[].number",
        ])
        .output()?;

    if !output.status.success() {
        // If no PRs found, gh returns success with empty output
        // If there's an actual error, return it
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            return Err(GgError::Other(format!(
                "Failed to list PRs for branch {}: {}",
                branch, stderr
            )));
        }
        return Ok(vec![]);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut prs = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if !line.is_empty() {
            if let Ok(num) = line.parse::<u64>() {
                prs.push(num);
            }
        }
    }

    Ok(prs)
}

/// A GitHub issue comment (which includes PR comments on the Conversation tab).
#[derive(Debug, Clone, Deserialize)]
pub struct IssueComment {
    pub id: u64,
    pub body: String,
}

/// List all comments on a PR (issue comments, i.e. Conversation-tab comments).
///
/// Paginates manually (100 per page) because `gh api --paginate` without
/// `--slurp` concatenates raw JSON arrays, which is not valid JSON — parsing
/// would fail as soon as a PR has more than one page of comments. We iterate
/// pages until an empty array comes back.
pub fn list_issue_comments(pr_number: u64) -> Result<Vec<IssueComment>> {
    let mut all = Vec::new();
    let mut page = 1u32;

    loop {
        let endpoint = format!(
            "repos/{{owner}}/{{repo}}/issues/{}/comments?per_page=100&page={}",
            pr_number, page
        );
        let output = Command::new("gh").args(["api", &endpoint]).output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GgError::Other(format!(
                "Failed to list comments for PR #{} (page {}): {}",
                pr_number, page, stderr
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let page_comments: Vec<IssueComment> = serde_json::from_str(&stdout).map_err(|e| {
            GgError::Other(format!(
                "Failed to parse comments JSON for PR #{} (page {}): {}",
                pr_number, page, e
            ))
        })?;

        if page_comments.is_empty() {
            break;
        }
        let full_page = page_comments.len() == 100;
        all.extend(page_comments);
        if !full_page {
            break;
        }
        page += 1;
    }

    Ok(all)
}

/// Post a new comment on a PR.
pub fn create_issue_comment(pr_number: u64, body: &str) -> Result<()> {
    let endpoint = format!("repos/{{owner}}/{{repo}}/issues/{}/comments", pr_number);
    let output = Command::new("gh")
        .args([
            "api",
            "-X",
            "POST",
            &endpoint,
            "-f",
            &format!("body={}", body),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to create comment on PR #{}: {}",
            pr_number, stderr
        )));
    }
    Ok(())
}

/// Edit an existing PR comment by its comment id.
pub fn update_issue_comment(comment_id: u64, body: &str) -> Result<()> {
    let endpoint = format!("repos/{{owner}}/{{repo}}/issues/comments/{}", comment_id);
    let output = Command::new("gh")
        .args([
            "api",
            "-X",
            "PATCH",
            &endpoint,
            "-f",
            &format!("body={}", body),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to update comment {}: {}",
            comment_id, stderr
        )));
    }
    Ok(())
}

/// Delete a PR comment by its comment id.
pub fn delete_issue_comment(comment_id: u64) -> Result<()> {
    let endpoint = format!("repos/{{owner}}/{{repo}}/issues/comments/{}", comment_id);
    let output = Command::new("gh")
        .args(["api", "-X", "DELETE", &endpoint])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to delete comment {}: {}",
            comment_id, stderr
        )));
    }
    Ok(())
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
            head_branch: Some("user/stack--c-abc1234".to_string()),
            draft: false,
            approved: true,
            mergeable: true,
            changes_requested: false,
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
            "headRefName": "user/stack--c-abc1234",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "reviews": [{"state": "APPROVED"}]
        }"#;

        let parsed: GhPrJson = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.number, 123);
        assert_eq!(parsed.title, "My PR");
        assert_eq!(parsed.state, "OPEN");
        assert_eq!(
            parsed.head_ref_name.as_deref(),
            Some("user/stack--c-abc1234")
        );
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
    fn inbox_snapshot_parses_review_and_ci_from_one_response() {
        let json = br#"{
            "number": 42,
            "title": "Add login",
            "state": "OPEN",
            "url": "https://github.com/acme/app/pull/42",
            "headRefName": "nacho/auth/c-abc1234",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "reviewDecision": "APPROVED",
            "statusCheckRollup": [
                {"status": "COMPLETED", "conclusion": "SUCCESS"}
            ]
        }"#;

        let snapshot = parse_inbox_snapshot(json).unwrap();
        assert_eq!(snapshot.state, PrState::Open);
        assert!(snapshot.approved);
        assert!(!snapshot.changes_requested);
        assert!(snapshot.mergeable);
        assert_eq!(snapshot.ci_status, Some(CiStatus::Success));
    }

    #[test]
    fn inbox_snapshot_treats_empty_rollup_as_no_ci() {
        let json = br#"{
            "number": 43,
            "title": "No checks",
            "state": "OPEN",
            "url": "https://github.com/acme/app/pull/43",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "reviewDecision": "CHANGES_REQUESTED",
            "statusCheckRollup": []
        }"#;

        let snapshot = parse_inbox_snapshot(json).unwrap();
        assert!(snapshot.changes_requested);
        assert_eq!(snapshot.ci_status, None);
    }

    #[test]
    fn inbox_snapshot_treats_null_review_decision_as_no_review_required() {
        let json = br#"{
            "number": 44,
            "title": "No required reviews",
            "state": "OPEN",
            "url": "https://github.com/acme/app/pull/44",
            "isDraft": false,
            "mergeable": "MERGEABLE",
            "reviewDecision": null,
            "statusCheckRollup": []
        }"#;

        let snapshot = parse_inbox_snapshot(json).unwrap();
        assert!(snapshot.approved);
        assert!(!snapshot.changes_requested);
    }

    #[test]
    fn inbox_snapshot_aggregates_rollup_statuses_by_priority() {
        let cases = [
            (
                r#"[{"status":"COMPLETED","conclusion":"SUCCESS"},{"status":"COMPLETED","conclusion":"FAILURE"}]"#,
                CiStatus::Failed,
            ),
            (
                r#"[{"status":"COMPLETED","conclusion":"SUCCESS"},{"status":"COMPLETED","conclusion":"CANCELLED"}]"#,
                CiStatus::Canceled,
            ),
            (
                r#"[{"status":"IN_PROGRESS","conclusion":null}]"#,
                CiStatus::Pending,
            ),
            (r#"[{"state":"EXPECTED"}]"#, CiStatus::Pending),
            (
                r#"[{"status":"COMPLETED","state":"ERROR"}]"#,
                CiStatus::Failed,
            ),
            (
                r#"[{"status":"COMPLETED","conclusion":"STARTUP_FAILURE"}]"#,
                CiStatus::Failed,
            ),
            (
                r#"[{"status":"COMPLETED","conclusion":"STALE"}]"#,
                CiStatus::Failed,
            ),
        ];

        for (rollup, expected) in cases {
            let json = format!(
                r#"{{
                    "number": 44,
                    "title": "Checks",
                    "state": "OPEN",
                    "url": "https://github.com/acme/app/pull/44",
                    "isDraft": false,
                    "mergeable": "MERGEABLE",
                    "statusCheckRollup": {}
                }}"#,
                rollup
            );

            let snapshot = parse_inbox_snapshot(json.as_bytes()).unwrap();
            assert_eq!(snapshot.ci_status, Some(expected));
        }
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

    #[test]
    fn test_comment_helpers_exist() {
        // Compile-only test; ensures the new functions are wired up.
        // Real invocations require a live gh CLI and are tested manually / in CI.
        let _: fn(u64) -> Result<Vec<IssueComment>> = list_issue_comments;
        let _: fn(u64, &str) -> Result<()> = create_issue_comment;
        let _: fn(u64, &str) -> Result<()> = update_issue_comment;
        let _: fn(u64) -> Result<()> = delete_issue_comment;
    }

    #[test]
    fn test_issue_comment_deserialization() {
        let json = r#"{"id": 12345, "body": "This is a comment\n<!-- gg:stack-nav -->"}"#;
        let comment: IssueComment = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(comment.id, 12345);
        assert!(comment.body.contains("<!-- gg:stack-nav -->"));
    }

    #[test]
    fn test_issue_comment_list_deserialization() {
        let json = r#"[
            {"id": 1, "body": "first"},
            {"id": 2, "body": "second <!-- gg:stack-nav -->"}
        ]"#;
        let comments: Vec<IssueComment> =
            serde_json::from_str(json).expect("should deserialize list");
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].id, 1);
        assert_eq!(comments[1].body, "second <!-- gg:stack-nav -->");
    }
}
