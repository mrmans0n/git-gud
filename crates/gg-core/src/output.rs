//! Structured output helpers.

use serde::Serialize;

pub const OUTPUT_VERSION: u32 = 1;

pub fn print_json<T: Serialize>(data: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(data).expect("failed to serialize JSON output")
    );
}

#[derive(Serialize)]
pub struct ErrorJson<'a> {
    pub version: u32,
    pub error: &'a str,
}

pub fn print_json_error(message: &str) {
    print_json(&ErrorJson {
        version: OUTPUT_VERSION,
        error: message,
    });
}

#[derive(Serialize)]
pub struct SingleStackResponse {
    pub version: u32,
    pub stack: StackJson,
}

#[derive(Serialize)]
pub struct StackJson {
    pub name: String,
    pub base: String,
    pub total_commits: usize,
    pub synced_commits: usize,
    pub current_position: Option<usize>,
    pub behind_base: Option<usize>,
    pub entries: Vec<StackEntryJson>,
}

#[derive(Serialize)]
pub struct StackEntryJson {
    pub position: usize,
    pub sha: String,
    pub title: String,
    pub gg_id: Option<String>,
    pub gg_parent: Option<String>,
    pub pr_number: Option<u64>,
    pub pr_state: Option<String>,
    pub approved: bool,
    pub ci_status: Option<String>,
    pub is_current: bool,
    pub in_merge_train: bool,
    pub merge_train_position: Option<usize>,
}

#[derive(Serialize)]
pub struct AllStacksResponse {
    pub version: u32,
    pub current_stack: Option<String>,
    pub stacks: Vec<StackSummaryJson>,
}

#[derive(Serialize)]
pub struct StackSummaryJson {
    pub name: String,
    pub base: String,
    pub commit_count: usize,
    pub is_current: bool,
    pub has_worktree: bool,
    pub behind_base: Option<usize>,
    pub commits: Vec<StackCommitJson>,
}

#[derive(Serialize)]
pub struct StackCommitJson {
    pub position: usize,
    pub sha: String,
    pub title: String,
}

#[derive(Serialize)]
pub struct RemoteStacksResponse {
    pub version: u32,
    pub stacks: Vec<RemoteStackJson>,
}

#[derive(Serialize)]
pub struct RemoteStackJson {
    pub name: String,
    pub commit_count: usize,
    pub pr_numbers: Vec<u64>,
}

#[derive(Serialize)]
pub struct SyncResponse {
    pub version: u32,
    pub sync: SyncResultJson,
}

#[derive(Serialize)]
pub struct SyncResultJson {
    pub stack: String,
    pub base: String,
    pub rebased_before_sync: bool,
    pub warnings: Vec<String>,
    pub metadata: SyncMetadataJson,
    pub entries: Vec<SyncEntryResultJson>,
}

#[derive(Serialize, Default)]
pub struct SyncMetadataJson {
    pub gg_ids_added: usize,
    pub gg_parents_updated: usize,
    pub gg_parents_removed: usize,
}

#[derive(Serialize)]
pub struct SyncEntryResultJson {
    pub position: usize,
    pub sha: String,
    pub title: String,
    pub gg_id: String,
    pub branch: String,
    pub action: String,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
    pub draft: bool,
    pub pushed: bool,
    pub error: Option<String>,
    /// Optional: action taken on the managed nav comment for this entry's PR.
    /// One of "created", "updated", "unchanged", "deleted", or "error".
    /// Omitted when the feature is disabled and no cleanup was required.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nav_comment_action: Option<String>,
}

#[derive(Serialize)]
pub struct LintResponse {
    pub version: u32,
    pub lint: LintResultJson,
}

#[derive(Serialize)]
pub struct LintResultJson {
    pub results: Vec<LintCommitResult>,
    pub all_passed: bool,
}

#[derive(Serialize)]
pub struct LintCommitResult {
    pub position: usize,
    pub sha: String,
    pub title: String,
    pub passed: bool,
    pub commands: Vec<LintCommandResult>,
}

#[derive(Serialize)]
pub struct LintCommandResult {
    pub command: String,
    pub passed: bool,
    pub output: Option<String>,
}

#[derive(Serialize)]
pub struct RunResponse {
    pub version: u32,
    pub run: RunResultJson,
}

#[derive(Serialize)]
pub struct RunResultJson {
    pub results: Vec<RunCommitResult>,
    pub all_passed: bool,
}

#[derive(Serialize)]
pub struct RunCommitResult {
    pub position: usize,
    pub sha: String,
    pub title: String,
    pub passed: bool,
    pub commands: Vec<RunCommandResult>,
}

#[derive(Serialize)]
pub struct RunCommandResult {
    pub command: String,
    pub passed: bool,
    pub output: Option<String>,
}

#[derive(Serialize)]
pub struct LandResponse {
    pub version: u32,
    pub land: LandResultJson,
}

#[derive(Serialize)]
pub struct LandResultJson {
    pub stack: String,
    pub base: String,
    pub landed: Vec<LandedEntryJson>,
    pub remaining: usize,
    pub cleaned: bool,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct LandedEntryJson {
    pub position: usize,
    pub sha: String,
    pub title: String,
    pub gg_id: String,
    pub pr_number: u64,
    pub action: String,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_response_serializes() {
        let response = RunResponse {
            version: OUTPUT_VERSION,
            run: RunResultJson {
                all_passed: false,
                results: vec![RunCommitResult {
                    position: 1,
                    sha: "abc1234".to_string(),
                    title: "Test commit".to_string(),
                    passed: false,
                    commands: vec![RunCommandResult {
                        command: "cargo test".to_string(),
                        passed: false,
                        output: Some("test failed".to_string()),
                    }],
                }],
            },
        };

        let value = serde_json::to_value(&response).expect("should serialize");
        assert_eq!(value["version"], OUTPUT_VERSION);
        assert_eq!(value["run"]["all_passed"], false);
        assert_eq!(value["run"]["results"][0]["position"], 1);
        assert_eq!(value["run"]["results"][0]["commands"][0]["passed"], false);
        assert_eq!(
            value["run"]["results"][0]["commands"][0]["output"],
            "test failed"
        );
    }

    #[test]
    fn lint_response_serializes() {
        let response = LintResponse {
            version: OUTPUT_VERSION,
            lint: LintResultJson {
                all_passed: false,
                results: vec![LintCommitResult {
                    position: 1,
                    sha: "abc1234".to_string(),
                    title: "Test commit".to_string(),
                    passed: false,
                    commands: vec![LintCommandResult {
                        command: "cargo clippy".to_string(),
                        passed: false,
                        output: Some("error: warning denied".to_string()),
                    }],
                }],
            },
        };

        let value = serde_json::to_value(&response).expect("should serialize");
        assert_eq!(value["version"], OUTPUT_VERSION);
        assert_eq!(value["lint"]["all_passed"], false);
        assert_eq!(value["lint"]["results"][0]["position"], 1);
        assert_eq!(value["lint"]["results"][0]["commands"][0]["passed"], false);
        assert_eq!(
            value["lint"]["results"][0]["commands"][0]["output"],
            "error: warning denied"
        );
    }

    #[test]
    fn test_sync_entry_nav_comment_action_omitted_when_none() {
        let entry = SyncEntryResultJson {
            position: 1,
            sha: "abc".to_string(),
            title: "t".to_string(),
            gg_id: "c-1234567".to_string(),
            branch: "b".to_string(),
            action: "created".to_string(),
            pr_number: Some(1),
            pr_url: None,
            draft: false,
            pushed: true,
            error: None,
            nav_comment_action: None,
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert!(
            json.get("nav_comment_action").is_none(),
            "field should be omitted when None"
        );
    }

    #[test]
    fn restack_response_serializes() {
        let response = RestackResponse {
            version: OUTPUT_VERSION,
            restack: RestackResultJson {
                stack_name: "my-feature".to_string(),
                total_entries: 4,
                entries_restacked: 2,
                entries_ok: 2,
                dry_run: false,
                steps: vec![
                    RestackStepJson {
                        position: 1,
                        gg_id: "c-aaa1111".to_string(),
                        title: "Add login form".to_string(),
                        action: "ok".to_string(),
                        current_parent: None,
                        expected_parent: None,
                    },
                    RestackStepJson {
                        position: 2,
                        gg_id: "c-bbb2222".to_string(),
                        title: "Add validation".to_string(),
                        action: "reattach".to_string(),
                        current_parent: Some("c-old1111".to_string()),
                        expected_parent: Some("c-aaa1111".to_string()),
                    },
                ],
            },
        };

        let value = serde_json::to_value(&response).expect("should serialize");
        assert_eq!(value["version"], OUTPUT_VERSION);
        assert_eq!(value["restack"]["stack_name"], "my-feature");
        assert_eq!(value["restack"]["entries_restacked"], 2);
        assert_eq!(value["restack"]["entries_ok"], 2);
        assert_eq!(value["restack"]["dry_run"], false);
        assert_eq!(value["restack"]["steps"][0]["action"], "ok");
        assert_eq!(value["restack"]["steps"][1]["action"], "reattach");
        assert_eq!(
            value["restack"]["steps"][1]["current_parent"],
            "c-old1111"
        );
        assert_eq!(
            value["restack"]["steps"][1]["expected_parent"],
            "c-aaa1111"
        );
    }

    #[test]
    fn test_sync_entry_nav_comment_action_serializes_when_some() {
        let entry = SyncEntryResultJson {
            position: 1,
            sha: "abc".to_string(),
            title: "t".to_string(),
            gg_id: "c-1234567".to_string(),
            branch: "b".to_string(),
            action: "created".to_string(),
            pr_number: Some(1),
            pr_url: None,
            draft: false,
            pushed: true,
            error: None,
            nav_comment_action: Some("created".to_string()),
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["nav_comment_action"], "created");
    }
}

#[derive(Serialize)]
pub struct CleanResponse {
    pub version: u32,
    pub clean: CleanResultJson,
}

#[derive(Serialize)]
pub struct CleanResultJson {
    pub cleaned: Vec<String>,
    pub skipped: Vec<String>,
}

#[derive(Serialize)]
pub struct DropResponse {
    pub version: u32,
    pub drop: DropResultJson,
}

#[derive(Serialize)]
pub struct DropResultJson {
    pub dropped: Vec<DroppedEntryJson>,
    pub remaining: usize,
}

#[derive(Serialize)]
pub struct DroppedEntryJson {
    pub position: usize,
    pub sha: String,
    pub title: String,
}

#[derive(Serialize)]
pub struct RestackResponse {
    pub version: u32,
    pub restack: RestackResultJson,
}

#[derive(Serialize)]
pub struct RestackResultJson {
    pub stack_name: String,
    pub total_entries: usize,
    pub entries_restacked: usize,
    pub entries_ok: usize,
    pub dry_run: bool,
    pub steps: Vec<RestackStepJson>,
}

#[derive(Serialize)]
pub struct RestackStepJson {
    pub position: usize,
    pub gg_id: String,
    pub title: String,
    pub action: String,
    pub current_parent: Option<String>,
    pub expected_parent: Option<String>,
}
