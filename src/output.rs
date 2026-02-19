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
    pub entries: Vec<SyncEntryResultJson>,
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
}

#[allow(dead_code)]
#[derive(Serialize)]
pub struct CleanResponse {
    pub version: u32,
    pub clean: CleanResultJson,
}

#[allow(dead_code)]
#[derive(Serialize)]
pub struct CleanResultJson {
    pub cleaned: Vec<String>,
    pub skipped: Vec<String>,
}
