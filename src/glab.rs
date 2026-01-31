//! GitLab CLI (glab) integration
//!
//! Wraps glab subprocess calls for MR management.
//! NOTE: This module is kept for future GitLab support but is not currently used.

#![allow(dead_code)]

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
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    parse_mr_number_from_output(&stdout, &stderr)
}

/// Parse MR number from glab command output.
///
/// Tries multiple strategies to extract the MR number:
/// 1. Look for `!N` pattern (e.g., "Created merge request !123")
/// 2. Look for `/merge_requests/N` in URLs
/// 3. Look for "MR N" or "merge request N" patterns
/// 4. Last resort: find any number > 0 in the output
fn parse_mr_number_from_output(stdout: &str, stderr: &str) -> Result<u64> {
    let combined = format!("{} {}", stdout, stderr);

    // Strategy 1: Look for !N pattern anywhere in output
    for word in combined.split_whitespace() {
        if let Some(stripped) = word.strip_prefix('!') {
            // Strip any trailing punctuation
            let num_str = stripped.trim_end_matches(|c: char| !c.is_ascii_digit());
            if let Ok(num) = num_str.parse::<u64>() {
                if num > 0 {
                    return Ok(num);
                }
            }
        }
    }

    // Strategy 2: Look for /merge_requests/N in URLs
    if let Some(idx) = combined.find("/merge_requests/") {
        let after = &combined[idx + "/merge_requests/".len()..];
        let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(num) = num_str.parse::<u64>() {
            if num > 0 {
                return Ok(num);
            }
        }
    }

    // Strategy 3: Look for any number after "MR" or "merge request" (case insensitive)
    let lower = combined.to_lowercase();
    for pattern in ["mr ", "mr!", "merge request "] {
        if let Some(idx) = lower.find(pattern) {
            let after = &combined[idx + pattern.len()..];
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(num) = num_str.parse::<u64>() {
                if num > 0 {
                    return Ok(num);
                }
            }
        }
    }

    // Strategy 4: Last resort - find any number > 0 in the output (likely the MR number)
    for word in combined.split(|c: char| !c.is_ascii_digit()) {
        if !word.is_empty() {
            if let Ok(num) = word.parse::<u64>() {
                if num > 0 {
                    return Ok(num);
                }
            }
        }
    }

    Err(GgError::GlabError(format!(
        "Could not parse MR number from glab output: {}",
        stdout.trim()
    )))
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

/// Alias for view_mr for compatibility with gh module
pub fn get_mr_info(mr_number: u64) -> Result<MrInfo> {
    view_mr(mr_number)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mr_number_exclamation_format() {
        // Standard format: !123
        assert_eq!(
            parse_mr_number_from_output("Created merge request !123", "").unwrap(),
            123
        );
        assert_eq!(
            parse_mr_number_from_output("!456 created", "").unwrap(),
            456
        );
        assert_eq!(
            parse_mr_number_from_output("MR !789 is ready", "").unwrap(),
            789
        );
    }

    #[test]
    fn test_parse_mr_number_exclamation_with_punctuation() {
        // Format with trailing punctuation: !123.
        assert_eq!(
            parse_mr_number_from_output("Created !123.", "").unwrap(),
            123
        );
        assert_eq!(parse_mr_number_from_output("See !456!", "").unwrap(), 456);
        assert_eq!(parse_mr_number_from_output("Done: !789,", "").unwrap(), 789);
    }

    #[test]
    fn test_parse_mr_number_url_format() {
        // URL format
        assert_eq!(
            parse_mr_number_from_output("https://gitlab.com/user/repo/-/merge_requests/123", "")
                .unwrap(),
            123
        );
        assert_eq!(
            parse_mr_number_from_output(
                "View at https://gitlab.example.com/group/project/-/merge_requests/456 for details",
                ""
            )
            .unwrap(),
            456
        );
    }

    #[test]
    fn test_parse_mr_number_mr_prefix() {
        // "MR N" format
        assert_eq!(
            parse_mr_number_from_output("Created MR 123", "").unwrap(),
            123
        );
        assert_eq!(parse_mr_number_from_output("MR!456 done", "").unwrap(), 456);
    }

    #[test]
    fn test_parse_mr_number_merge_request_text() {
        // "merge request N" format
        assert_eq!(
            parse_mr_number_from_output("Created merge request 123 successfully", "").unwrap(),
            123
        );
    }

    #[test]
    fn test_parse_mr_number_from_stderr() {
        // Number in stderr
        assert_eq!(
            parse_mr_number_from_output("", "Created !789").unwrap(),
            789
        );
        assert_eq!(
            parse_mr_number_from_output("Some output", "MR 456 created").unwrap(),
            456
        );
    }

    #[test]
    fn test_parse_mr_number_fallback_to_any_number() {
        // Last resort: any number in output
        assert_eq!(parse_mr_number_from_output("Success: 42", "").unwrap(), 42);
    }

    #[test]
    fn test_parse_mr_number_ignores_zero() {
        // Should not return 0
        assert!(parse_mr_number_from_output("Nothing here", "").is_err());
        // But should find non-zero after zero
        assert_eq!(
            parse_mr_number_from_output("0 errors, created 123", "").unwrap(),
            123
        );
    }

    #[test]
    fn test_parse_mr_number_empty_output() {
        assert!(parse_mr_number_from_output("", "").is_err());
        assert!(parse_mr_number_from_output("   ", "   ").is_err());
    }

    #[test]
    fn test_parse_mr_number_no_numbers() {
        assert!(parse_mr_number_from_output("No numbers here", "none here either").is_err());
    }

    #[test]
    fn test_parse_mr_number_real_glab_output() {
        // Real glab output examples
        assert_eq!(
            parse_mr_number_from_output(
                "Creating merge request for feature-branch into main in user/repo\n\n!42 Feature: Add new thing\nhttps://gitlab.com/user/repo/-/merge_requests/42",
                ""
            ).unwrap(),
            42
        );
    }
}
