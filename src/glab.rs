//! GitLab CLI (glab) integration
//!
//! Wraps glab subprocess calls for MR management.

use std::process::Command;

use serde::Deserialize;

use crate::error::{GgError, Result};

/// MR state from GitLab
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MrState {
    Open,
    Merged,
    Closed,
    Draft,
}

/// MR information from glab
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used for display/future features
pub struct MrInfo {
    pub iid: u64,
    pub title: String,
    pub state: MrState,
    pub web_url: String,
    pub draft: bool,
    pub approved: bool,
    pub mergeable: bool,
}

/// JSON response from `glab mr view --json`
#[derive(Debug, Deserialize)]
struct GlabMrJson {
    iid: u64,
    title: String,
    state: String,
    web_url: String,
    draft: Option<bool>,
    work_in_progress: Option<bool>,
}

/// Check if glab is installed
pub fn check_glab_installed() -> Result<()> {
    let output = Command::new("glab").arg("--version").output();

    match output {
        Ok(o) if o.status.success() => Ok(()),
        _ => Err(GgError::GlabNotInstalled),
    }
}

/// Check if authenticated with GitLab
pub fn check_glab_auth() -> Result<()> {
    let output = Command::new("glab").args(["auth", "status"]).output()?;

    if output.status.success() {
        Ok(())
    } else {
        Err(GgError::GlabNotAuthenticated)
    }
}

/// Get the current GitLab username
pub fn whoami() -> Result<String> {
    let output = Command::new("glab")
        .args(["auth", "status", "-t"])
        .output()?;

    if !output.status.success() {
        return Err(GgError::GlabNotAuthenticated);
    }

    // Parse output to find username
    // Format: "Logged in to gitlab.com as <username> ..."
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    for line in combined.lines() {
        if line.contains("Logged in") && line.contains(" as ") {
            if let Some(username_part) = line.split(" as ").nth(1) {
                let username = username_part.split_whitespace().next().unwrap_or("");
                if !username.is_empty() {
                    return Ok(username.to_string());
                }
            }
        }
    }

    // Fallback: try `glab api user`
    let api_output = Command::new("glab")
        .args(["api", "user", "--jq", ".username"])
        .output()?;

    if api_output.status.success() {
        let username = String::from_utf8_lossy(&api_output.stdout)
            .trim()
            .to_string();
        if !username.is_empty() {
            return Ok(username);
        }
    }

    Err(GgError::GlabError(
        "Could not determine GitLab username".to_string(),
    ))
}

/// Create a new MR
pub fn create_mr(
    source_branch: &str,
    target_branch: &str,
    title: &str,
    description: &str,
    draft: bool,
) -> Result<u64> {
    let mut args = vec![
        "mr",
        "create",
        "--source-branch",
        source_branch,
        "--target-branch",
        target_branch,
        "--title",
        title,
        "--description",
        description,
        "--yes", // Skip confirmation
    ];

    if draft {
        args.push("--draft");
    }

    let output = Command::new("glab").args(&args).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::GlabError(format!(
            "Failed to create MR: {}",
            stderr
        )));
    }

    // Parse the output to get the MR number
    // glab outputs something like "!123" or a URL
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Try to extract MR number from output
    for word in stdout.split_whitespace() {
        if let Some(stripped) = word.strip_prefix('!') {
            if let Ok(num) = stripped.parse::<u64>() {
                return Ok(num);
            }
        }
        // Also check for URL pattern
        if word.contains("/merge_requests/") {
            if let Some(num_str) = word.split("/merge_requests/").nth(1) {
                let num_str = num_str.trim_end_matches(|c: char| !c.is_ascii_digit());
                if let Ok(num) = num_str.parse::<u64>() {
                    return Ok(num);
                }
            }
        }
    }

    Err(GgError::GlabError(
        "Could not parse MR number from glab output".to_string(),
    ))
}

/// View MR information
pub fn view_mr(mr_number: u64) -> Result<MrInfo> {
    let output = Command::new("glab")
        .args(["mr", "view", &mr_number.to_string(), "--output", "json"])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::GlabError(format!(
            "Failed to view MR !{}: {}",
            mr_number, stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mr_json: GlabMrJson = serde_json::from_str(&stdout)
        .map_err(|e| GgError::GlabError(format!("Failed to parse MR JSON: {}", e)))?;

    let draft = mr_json.draft.unwrap_or(false) || mr_json.work_in_progress.unwrap_or(false);

    let state = match mr_json.state.as_str() {
        "merged" => MrState::Merged,
        "closed" => MrState::Closed,
        _ if draft => MrState::Draft,
        _ => MrState::Open,
    };

    let mergeable = state == MrState::Open && !draft;

    Ok(MrInfo {
        iid: mr_json.iid,
        title: mr_json.title,
        state,
        web_url: mr_json.web_url,
        draft,
        approved: false, // Would need additional API call
        mergeable,
    })
}

/// Update MR target branch
pub fn update_mr_target(mr_number: u64, target_branch: &str) -> Result<()> {
    let output = Command::new("glab")
        .args([
            "mr",
            "update",
            &mr_number.to_string(),
            "--target-branch",
            target_branch,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::GlabError(format!(
            "Failed to update MR !{}: {}",
            mr_number, stderr
        )));
    }

    Ok(())
}

/// Merge an MR
pub fn merge_mr(mr_number: u64, squash: bool, delete_branch: bool) -> Result<()> {
    let mr_num_str = mr_number.to_string();
    let mut args = vec!["mr", "merge", &mr_num_str, "--yes"];

    if squash {
        args.push("--squash");
    }
    if delete_branch {
        args.push("--remove-source-branch");
    }

    let output = Command::new("glab").args(&args).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::GlabError(format!(
            "Failed to merge MR !{}: {}",
            mr_number, stderr
        )));
    }

    Ok(())
}

/// Check approvals for an MR
pub fn check_mr_approved(mr_number: u64) -> Result<bool> {
    // Use glab api to check approvals
    let output = Command::new("glab")
        .args([
            "api",
            &format!("projects/:id/merge_requests/{}/approvals", mr_number),
            "--jq",
            ".approved",
        ])
        .output()?;

    if !output.status.success() {
        // If the call fails, assume not approved
        return Ok(false);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(stdout == "true")
}

/// Get CI status for an MR
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiStatus {
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
    Unknown,
}

pub fn get_mr_ci_status(mr_number: u64) -> Result<CiStatus> {
    let output = Command::new("glab")
        .args(["mr", "view", &mr_number.to_string(), "--output", "json"])
        .output()?;

    if !output.status.success() {
        return Ok(CiStatus::Unknown);
    }

    // Parse pipeline status from the response
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Simple heuristic - look for pipeline status in output
    if stdout.contains("\"pipeline_status\":\"success\"")
        || stdout.contains("\"head_pipeline\":{") && stdout.contains("\"status\":\"success\"")
    {
        Ok(CiStatus::Success)
    } else if stdout.contains("\"status\":\"failed\"") {
        Ok(CiStatus::Failed)
    } else if stdout.contains("\"status\":\"running\"") {
        Ok(CiStatus::Running)
    } else if stdout.contains("\"status\":\"pending\"") {
        Ok(CiStatus::Pending)
    } else if stdout.contains("\"status\":\"canceled\"") {
        Ok(CiStatus::Canceled)
    } else {
        Ok(CiStatus::Unknown)
    }
}
