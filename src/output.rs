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
