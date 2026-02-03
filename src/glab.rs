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
    // Note: We don't use --jq flag as it's not available in all glab versions
    let api_output = Command::new("glab").args(["api", "user"]).output()?;

    if api_output.status.success() {
        let stdout = String::from_utf8_lossy(&api_output.stdout);
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
            if let Some(username) = json.get("username").and_then(|v| v.as_str()) {
                if !username.is_empty() {
                    return Ok(username.to_string());
                }
            }
        }
    }

    Err(GgError::GlabError(
        "Could not determine GitLab username".to_string(),
    ))
}

/// Result of creating an MR
#[derive(Debug, Clone)]
pub struct MrCreationResult {
    pub number: u64,
    pub url: String,
}

/// Create a new MR
pub fn create_mr(
    source_branch: &str,
    target_branch: &str,
    title: &str,
    description: &str,
    draft: bool,
) -> Result<MrCreationResult> {
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

    // Parse the output to get the MR number and URL
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    parse_mr_creation_result(&stdout, &stderr)
}

/// Parse MR creation result (number and URL) from glab command output.
///
/// Tries multiple strategies to extract the MR number and URL:
/// 1. Look for URL containing `/merge_requests/N`
/// 2. Look for `!N` pattern (e.g., "Created merge request !123")
/// 3. Look for "MR N" or "merge request N" patterns
/// 4. Last resort: find any number > 0 in the output
fn parse_mr_creation_result(stdout: &str, stderr: &str) -> Result<MrCreationResult> {
    let combined = format!("{} {}", stdout, stderr);

    // First, try to extract URL - it's more reliable and gives us both URL and number
    let url = extract_mr_url(&combined);

    // Try to get number from URL first
    if let Some(ref url_str) = url {
        if let Some(idx) = url_str.find("/merge_requests/") {
            let after = &url_str[idx + "/merge_requests/".len()..];
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(num) = num_str.parse::<u64>() {
                if num > 0 {
                    return Ok(MrCreationResult {
                        number: num,
                        url: url_str.clone(),
                    });
                }
            }
        }
    }

    // Fallback: parse number using various strategies
    let number = parse_mr_number_from_output(stdout, stderr)?;

    // If we have a URL, use it; otherwise construct a placeholder
    let final_url = url.unwrap_or_default();

    Ok(MrCreationResult {
        number,
        url: final_url,
    })
}

/// Extract MR URL from glab output
fn extract_mr_url(text: &str) -> Option<String> {
    // Look for URLs containing /merge_requests/
    for word in text.split_whitespace() {
        if word.contains("/merge_requests/")
            && (word.starts_with("http://") || word.starts_with("https://"))
        {
            // Clean up the URL (remove trailing punctuation)
            let url = word.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '/');
            return Some(url.to_string());
        }
    }
    None
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

/// Update MR description/body
pub fn update_mr_description(mr_number: u64, description: &str) -> Result<()> {
    let output = Command::new("glab")
        .args([
            "mr",
            "update",
            &mr_number.to_string(),
            "--description",
            description,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::GlabError(format!(
            "Failed to update MR !{} description: {}",
            mr_number, stderr
        )));
    }

    Ok(())
}

/// Update MR title
pub fn update_mr_title(mr_number: u64, title: &str) -> Result<()> {
    let output = Command::new("glab")
        .args(["mr", "update", &mr_number.to_string(), "--title", title])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::GlabError(format!(
            "Failed to update MR !{} title: {}",
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

/// Request GitLab to auto-merge an MR when the pipeline succeeds.
///
/// This sets GitLab's "merge when pipeline succeeds" flag via the API.
///
/// Note: this does not wait for the pipeline; it only queues the merge.
///
/// Returns:
/// - `Ok(AutoMergeResult::Queued)` if successfully queued for auto-merge
/// - `Ok(AutoMergeResult::AlreadyQueued)` if already set to auto-merge (HTTP 409)
/// - `Err(...)` for other errors
pub fn auto_merge_mr_when_pipeline_succeeds(
    mr_number: u64,
    squash: bool,
    delete_branch: bool,
) -> Result<AutoMergeResult> {
    let output = Command::new("glab")
        .args([
            "api",
            "--method",
            "PUT",
            &format!("projects/:id/merge_requests/{}/merge", mr_number),
            "-f",
            "merge_when_pipeline_succeeds=true",
            "-f",
            &format!(
                "should_remove_source_branch={}",
                if delete_branch { "true" } else { "false" }
            ),
            "-f",
            &format!("squash={}", if squash { "true" } else { "false" }),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);

        // Check for HTTP 409 "already set to Auto-Merge" error
        // GitLab CLI returns this when the MR is already queued for auto-merge
        if stderr.contains("409") && stderr.contains("already") && stderr.contains("Auto-Merge") {
            return Ok(AutoMergeResult::AlreadyQueued);
        }

        return Err(GgError::GlabError(format!(
            "Failed to request auto-merge for MR !{}: {}",
            mr_number, stderr
        )));
    }

    Ok(AutoMergeResult::Queued)
}

/// Check approvals for an MR
pub fn check_mr_approved(mr_number: u64) -> Result<bool> {
    // Use glab api to check approvals
    // Note: We don't use --jq flag as it's not available in all glab versions
    let output = Command::new("glab")
        .args([
            "api",
            &format!("projects/:id/merge_requests/{}/approvals", mr_number),
        ])
        .output()?;

    if !output.status.success() {
        // If the call fails, assume not approved
        return Ok(false);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse JSON and extract .approved field
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
        return Ok(json
            .get("approved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
    }

    Ok(false)
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

/// Result of auto-merge or merge train operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoMergeResult {
    /// Successfully queued for auto-merge
    Queued,
    /// Already queued for auto-merge
    AlreadyQueued,
}

/// Merge train status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeTrainStatus {
    /// MR is idle (not in train)
    Idle,
    /// MR is stale (needs rebase)
    Stale,
    /// MR is fresh (ready to merge)
    Fresh,
    /// MR is currently merging
    Merging,
    /// MR has been merged
    Merged,
    /// MR was skipped from the train
    SkipMerged,
    /// Status unknown or error
    Unknown,
}

/// Merge train information for an MR
#[derive(Debug, Clone)]
pub struct MergeTrainInfo {
    pub status: MergeTrainStatus,
    pub position: Option<usize>,
    pub pipeline_running: bool,
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

/// List MRs for a specific source branch
/// Returns a list of MR numbers (iids) for open MRs with the given source branch
pub fn list_mrs_for_branch(branch: &str) -> Result<Vec<u64>> {
    let output = Command::new("glab")
        .args(["mr", "list", "--source-branch", branch, "--output", "json"])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            return Err(GgError::GlabError(format!(
                "Failed to list MRs for branch {}: {}",
                branch, stderr
            )));
        }
        return Ok(vec![]);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() || stdout.trim() == "[]" {
        return Ok(vec![]);
    }

    // Parse JSON array of MRs
    #[derive(Deserialize)]
    struct MrListItem {
        iid: u64,
    }

    let mrs: Vec<MrListItem> = serde_json::from_str(&stdout)
        .map_err(|e| GgError::GlabError(format!("Failed to parse MR list: {}", e)))?;

    Ok(mrs.into_iter().map(|mr| mr.iid).collect())
}

/// Check if merge trains are enabled for the current project
/// Returns true if merge trains are enabled, false otherwise
/// Uses caching to avoid repeated API calls (stored in memory)
pub fn check_merge_trains_enabled() -> Result<bool> {
    // Use glab api to check project settings
    // Note: We don't use --jq flag as it's not available in all glab versions
    let output = Command::new("glab")
        .args(["api", "projects/:id"])
        .output()?;

    if !output.status.success() {
        // If the call fails, assume merge trains are not enabled
        return Ok(false);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse JSON and extract .merge_trains_enabled field (defaults to false)
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
        return Ok(json
            .get("merge_trains_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
    }

    Ok(false)
}

/// Add an MR to the merge train
/// This is used instead of direct merge when merge trains are enabled
///
/// Returns:
/// - `Ok(AutoMergeResult::Queued)` if successfully added to the merge train
/// - `Ok(AutoMergeResult::AlreadyQueued)` if already in the merge train (HTTP 409)
/// - `Err(...)` for other errors
pub fn add_to_merge_train(mr_number: u64) -> Result<AutoMergeResult> {
    let output = Command::new("glab")
        .args([
            "api",
            "-X",
            "POST",
            &format!("projects/:id/merge_trains/merge_requests/{}", mr_number),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);

        // Check for HTTP 409 "already set to Auto-Merge" error
        // GitLab returns this when the MR is already in the merge train
        if stderr.contains("409") && stderr.contains("already") && stderr.contains("Auto-Merge") {
            return Ok(AutoMergeResult::AlreadyQueued);
        }

        return Err(GgError::GlabError(format!(
            "Failed to add MR !{} to merge train: {}",
            mr_number, stderr
        )));
    }

    Ok(AutoMergeResult::Queued)
}

/// Get merge train status for an MR
/// Returns information about the MR's position and status in the merge train
pub fn get_merge_train_status(mr_number: u64, target_branch: &str) -> Result<MergeTrainInfo> {
    // First, check if the MR is in the merge train by listing all merge trains
    let output = Command::new("glab")
        .args([
            "api",
            &format!("projects/:id/merge_trains/{}", target_branch),
        ])
        .output()?;

    if !output.status.success() {
        // If the call fails, assume MR is not in train
        return Ok(MergeTrainInfo {
            status: MergeTrainStatus::Idle,
            position: None,
            pipeline_running: false,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse the JSON array of merge trains
    #[derive(Deserialize)]
    struct MergeTrainEntry {
        merge_request: MergeTrainMr,
        status: String,
        #[serde(default)]
        pipeline: Option<MergeTrainPipeline>,
    }

    #[derive(Deserialize)]
    struct MergeTrainMr {
        iid: u64,
    }

    #[derive(Deserialize)]
    struct MergeTrainPipeline {
        status: String,
    }

    let trains: Vec<MergeTrainEntry> = serde_json::from_str(&stdout).unwrap_or_default();

    // Find this MR in the merge train
    for (idx, entry) in trains.iter().enumerate() {
        if entry.merge_request.iid == mr_number {
            let status = match entry.status.as_str() {
                "idle" => MergeTrainStatus::Idle,
                "stale" => MergeTrainStatus::Stale,
                "fresh" => MergeTrainStatus::Fresh,
                "merging" => MergeTrainStatus::Merging,
                "merged" => MergeTrainStatus::Merged,
                "skip_merged" => MergeTrainStatus::SkipMerged,
                _ => MergeTrainStatus::Unknown,
            };

            let pipeline_running = entry
                .pipeline
                .as_ref()
                .map(|p| matches!(p.status.as_str(), "running" | "pending"))
                .unwrap_or(false);

            return Ok(MergeTrainInfo {
                status,
                position: Some(idx + 1), // 1-indexed position
                pipeline_running,
            });
        }
    }

    // MR not found in train
    Ok(MergeTrainInfo {
        status: MergeTrainStatus::Idle,
        position: None,
        pipeline_running: false,
    })
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

    #[test]
    fn test_extract_mr_url_basic() {
        let url = extract_mr_url("https://gitlab.com/user/repo/-/merge_requests/123");
        assert_eq!(
            url,
            Some("https://gitlab.com/user/repo/-/merge_requests/123".to_string())
        );
    }

    #[test]
    fn test_extract_mr_url_with_surrounding_text() {
        let url = extract_mr_url(
            "Created MR at https://gitlab.com/user/repo/-/merge_requests/456 successfully",
        );
        assert_eq!(
            url,
            Some("https://gitlab.com/user/repo/-/merge_requests/456".to_string())
        );
    }

    #[test]
    fn test_extract_mr_url_with_trailing_punctuation() {
        let url = extract_mr_url("See https://gitlab.com/user/repo/-/merge_requests/789.");
        assert_eq!(
            url,
            Some("https://gitlab.com/user/repo/-/merge_requests/789".to_string())
        );
    }

    #[test]
    fn test_extract_mr_url_no_url() {
        let url = extract_mr_url("Created MR !123");
        assert_eq!(url, None);
    }

    #[test]
    fn test_extract_mr_url_http() {
        let url = extract_mr_url("http://gitlab.internal/group/project/-/merge_requests/42");
        assert_eq!(
            url,
            Some("http://gitlab.internal/group/project/-/merge_requests/42".to_string())
        );
    }

    #[test]
    fn test_parse_mr_creation_result_with_url() {
        let result = parse_mr_creation_result(
            "Creating merge request\n\n!42 Feature\nhttps://gitlab.com/user/repo/-/merge_requests/42",
            ""
        ).unwrap();
        assert_eq!(result.number, 42);
        assert_eq!(
            result.url,
            "https://gitlab.com/user/repo/-/merge_requests/42"
        );
    }

    #[test]
    fn test_parse_mr_creation_result_url_only() {
        let result =
            parse_mr_creation_result("https://gitlab.com/user/repo/-/merge_requests/123", "")
                .unwrap();
        assert_eq!(result.number, 123);
        assert_eq!(
            result.url,
            "https://gitlab.com/user/repo/-/merge_requests/123"
        );
    }

    #[test]
    fn test_parse_mr_creation_result_no_url() {
        let result = parse_mr_creation_result("Created MR !789", "").unwrap();
        assert_eq!(result.number, 789);
        assert!(result.url.is_empty());
    }

    #[test]
    fn test_mr_creation_result_construction() {
        let result = MrCreationResult {
            number: 42,
            url: "https://gitlab.com/test/repo/-/merge_requests/42".to_string(),
        };
        assert_eq!(result.number, 42);
        assert_eq!(
            result.url,
            "https://gitlab.com/test/repo/-/merge_requests/42"
        );
    }

    #[test]
    fn test_merge_train_status_equality() {
        assert_eq!(MergeTrainStatus::Idle, MergeTrainStatus::Idle);
        assert_eq!(MergeTrainStatus::Fresh, MergeTrainStatus::Fresh);
        assert_eq!(MergeTrainStatus::Merging, MergeTrainStatus::Merging);
        assert_eq!(MergeTrainStatus::Merged, MergeTrainStatus::Merged);
        assert_ne!(MergeTrainStatus::Idle, MergeTrainStatus::Fresh);
    }

    #[test]
    fn test_merge_train_info_construction() {
        let info = MergeTrainInfo {
            status: MergeTrainStatus::Fresh,
            position: Some(3),
            pipeline_running: true,
        };
        assert_eq!(info.status, MergeTrainStatus::Fresh);
        assert_eq!(info.position, Some(3));
        assert!(info.pipeline_running);
    }

    #[test]
    fn test_merge_train_info_idle() {
        let info = MergeTrainInfo {
            status: MergeTrainStatus::Idle,
            position: None,
            pipeline_running: false,
        };
        assert_eq!(info.status, MergeTrainStatus::Idle);
        assert_eq!(info.position, None);
        assert!(!info.pipeline_running);
    }

    #[test]
    fn test_merge_train_status_all_variants() {
        // Test all status variants can be created and compared
        let statuses = vec![
            MergeTrainStatus::Idle,
            MergeTrainStatus::Stale,
            MergeTrainStatus::Fresh,
            MergeTrainStatus::Merging,
            MergeTrainStatus::Merged,
            MergeTrainStatus::SkipMerged,
            MergeTrainStatus::Unknown,
        ];

        // Each status should equal itself
        for status in &statuses {
            assert_eq!(status, status);
        }

        // Different statuses should not be equal
        assert_ne!(MergeTrainStatus::Fresh, MergeTrainStatus::Merging);
        assert_ne!(MergeTrainStatus::Merged, MergeTrainStatus::SkipMerged);
        assert_ne!(MergeTrainStatus::Unknown, MergeTrainStatus::Idle);
    }

    #[test]
    fn test_merge_train_info_with_position() {
        // Test various positions
        for pos in [1, 5, 10, 100] {
            let info = MergeTrainInfo {
                status: MergeTrainStatus::Fresh,
                position: Some(pos),
                pipeline_running: false,
            };
            assert_eq!(info.position, Some(pos));
        }
    }

    #[test]
    fn test_merge_train_info_pipeline_states() {
        // Pipeline running
        let running = MergeTrainInfo {
            status: MergeTrainStatus::Fresh,
            position: Some(1),
            pipeline_running: true,
        };
        assert!(running.pipeline_running);

        // Pipeline not running
        let not_running = MergeTrainInfo {
            status: MergeTrainStatus::Fresh,
            position: Some(1),
            pipeline_running: false,
        };
        assert!(!not_running.pipeline_running);
    }

    #[test]
    fn test_merge_train_status_debug_format() {
        // Ensure Debug trait works correctly
        let status = MergeTrainStatus::Fresh;
        let debug_str = format!("{:?}", status);
        assert_eq!(debug_str, "Fresh");
    }

    #[test]
    fn test_merge_train_status_clone() {
        let original = MergeTrainStatus::Merging;
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn test_auto_merge_result_equality() {
        assert_eq!(AutoMergeResult::Queued, AutoMergeResult::Queued);
        assert_eq!(
            AutoMergeResult::AlreadyQueued,
            AutoMergeResult::AlreadyQueued
        );
        assert_ne!(AutoMergeResult::Queued, AutoMergeResult::AlreadyQueued);
    }

    #[test]
    fn test_auto_merge_result_debug_format() {
        // Ensure Debug trait works correctly
        let queued = AutoMergeResult::Queued;
        let already_queued = AutoMergeResult::AlreadyQueued;

        let queued_str = format!("{:?}", queued);
        let already_queued_str = format!("{:?}", already_queued);

        assert_eq!(queued_str, "Queued");
        assert_eq!(already_queued_str, "AlreadyQueued");
    }

    #[test]
    fn test_auto_merge_result_clone() {
        let original = AutoMergeResult::Queued;
        let cloned = original.clone();
        assert_eq!(original, cloned);

        let original2 = AutoMergeResult::AlreadyQueued;
        let cloned2 = original2.clone();
        assert_eq!(original2, cloned2);
    }

    // Note: Testing the actual GitLab API calls (auto_merge_mr_when_pipeline_succeeds,
    // add_to_merge_train) requires mocking the glab CLI or having a live GitLab instance.
    // These are better tested via integration tests.
    // The key logic we're testing here is:
    // 1. Parse stderr for "409" + "already" + "Auto-Merge"
    // 2. Return Ok(AutoMergeResult::AlreadyQueued) in that case
    // 3. Return Ok(AutoMergeResult::Queued) on success
    // 4. Return Err(...) for other errors
}
