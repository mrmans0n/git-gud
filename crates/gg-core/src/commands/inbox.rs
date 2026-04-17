//! `gg inbox` - Actionable triage view across all stacks

use console::style;
use serde::Serialize;

use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::output::{print_json, OUTPUT_VERSION};
use crate::provider::{CiStatus, PrState, Provider};
use crate::stack::{self, StackEntry};

/// Action buckets for inbox triage — evaluated in priority order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionBucket {
    ReadyToLand,
    ChangesRequested,
    BlockedOnCi,
    AwaitingReview,
    BehindBase,
    Draft,
    Merged,
}

impl ActionBucket {
    pub fn label(self) -> &'static str {
        match self {
            ActionBucket::ReadyToLand => "Ready to land",
            ActionBucket::ChangesRequested => "Changes requested",
            ActionBucket::BlockedOnCi => "Blocked on CI",
            ActionBucket::AwaitingReview => "Awaiting review",
            ActionBucket::BehindBase => "Behind base",
            ActionBucket::Draft => "Draft",
            ActionBucket::Merged => "Merged",
        }
    }
}

/// Inputs for the bucketing function (decoupled from StackEntry for testability).
#[derive(Debug)]
pub struct BucketInput {
    pub mr_state: PrState,
    pub ci_status: Option<CiStatus>,
    pub approved: bool,
    pub changes_requested: bool,
    pub mergeable: bool,
    pub behind_base: bool,
}

/// Classify a PR into an action bucket.
///
/// Priority order (first match wins):
/// 1. Merged
/// 2. Closed → None (skip)
/// 3. Draft
/// 4. Changes requested
/// 5. Approved + CI green + mergeable → ReadyToLand
/// 6. CI failed/running/pending → BlockedOnCi
/// 7. Behind base → BehindBase
/// 8. Fallthrough → AwaitingReview
pub fn bucket(input: &BucketInput) -> Option<ActionBucket> {
    match input.mr_state {
        PrState::Merged => return Some(ActionBucket::Merged),
        PrState::Closed => return None,
        _ => {}
    }

    if input.mr_state == PrState::Draft {
        return Some(ActionBucket::Draft);
    }

    if input.changes_requested {
        return Some(ActionBucket::ChangesRequested);
    }

    if input.approved && input.mergeable {
        // No CI = treat as green (no branch protection CI requirement)
        let ci_green = matches!(input.ci_status, Some(CiStatus::Success) | None);
        if ci_green {
            return Some(ActionBucket::ReadyToLand);
        }
    }

    if matches!(
        input.ci_status,
        Some(CiStatus::Failed) | Some(CiStatus::Running) | Some(CiStatus::Pending)
    ) {
        return Some(ActionBucket::BlockedOnCi);
    }

    if input.behind_base {
        return Some(ActionBucket::BehindBase);
    }

    Some(ActionBucket::AwaitingReview)
}

/// A single item in the inbox view.
struct InboxItem {
    stack_name: String,
    position: usize,
    short_sha: String,
    title: String,
    mr_number: u64,
    mr_url: String,
    bucket: ActionBucket,
    ci_status: Option<CiStatus>,
}

// --- JSON output types ---

#[derive(Serialize)]
pub struct InboxResponse {
    pub version: u32,
    pub total_items: usize,
    pub buckets: InboxBucketsJson,
}

#[derive(Serialize)]
pub struct InboxBucketsJson {
    pub ready_to_land: Vec<InboxEntryJson>,
    pub changes_requested: Vec<InboxEntryJson>,
    pub blocked_on_ci: Vec<InboxEntryJson>,
    pub awaiting_review: Vec<InboxEntryJson>,
    pub behind_base: Vec<InboxEntryJson>,
    pub draft: Vec<InboxEntryJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub merged: Vec<InboxEntryJson>,
}

#[derive(Serialize)]
pub struct InboxEntryJson {
    pub stack_name: String,
    pub position: usize,
    pub sha: String,
    pub title: String,
    pub pr_number: u64,
    pub pr_url: String,
    pub ci_status: Option<String>,
}

/// Run the inbox command.
pub fn run(all: bool, json: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.commondir();
    let config = Config::load_with_global(git_dir)?;
    let provider = Provider::detect(&repo)?;

    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| provider.whoami().ok())
        .unwrap_or_else(|| "unknown".to_string());

    git::validate_branch_username(&username)?;

    let stack_names = stack::list_all_stacks(&repo, &config, &username)?;

    if !json {
        eprint!("Refreshing {} status... ", provider.pr_label());
    }

    let mut items: Vec<InboxItem> = Vec::new();

    for stack_name in &stack_names {
        // Try to load the stack by resolving its tip ref
        let full_branch = git::format_stack_branch(&username, stack_name);
        let base = config
            .get_base_for_stack(stack_name)
            .unwrap_or("main")
            .to_string();

        // Get commit OIDs for this stack
        let oids = match git::get_stack_commit_oids(&repo, &base, Some(&full_branch)) {
            Ok(oids) => oids,
            Err(_) => continue, // Stack branch doesn't exist or can't resolve
        };

        if oids.is_empty() {
            continue;
        }

        // Build stack entries
        let mut entries: Vec<StackEntry> = Vec::with_capacity(oids.len());
        for (i, oid) in oids.iter().enumerate() {
            let commit = repo.find_commit(*oid)?;
            entries.push(StackEntry::from_commit(&commit, i + 1));
        }

        // Enrich with MR numbers from config
        if let Some(stack_config) = config.get_stack(stack_name) {
            for entry in &mut entries {
                if let Some(gg_id) = &entry.gg_id {
                    if let Some(mr_num) = stack_config.mrs.get(gg_id) {
                        entry.mr_number = Some(*mr_num);
                    }
                }
            }
        }

        // Compute behind_base: how many commits on origin/base are not
        // reachable from the stack branch (i.e., does the stack need rebasing?)
        let is_behind_base =
            git::count_branch_behind_upstream(&repo, &full_branch, &format!("origin/{}", base))
                .unwrap_or(0)
                > 0;

        // Refresh MR info and bucket each entry
        for entry in &mut entries {
            if let Some(pr_num) = entry.mr_number {
                // Fetch PR info
                if let Ok(info) = provider.get_pr_info(pr_num) {
                    entry.mr_state = Some(info.state);
                    entry.approved = info.approved;
                    entry.changes_requested = info.changes_requested;
                    entry.mergeable = info.mergeable;
                    entry.mr_url = Some(info.url);
                }

                // Get CI status
                if let Ok(ci) = provider.get_pr_ci_status(pr_num) {
                    entry.ci_status = Some(ci);
                }

                // Bucket the entry
                if let Some(ref mr_state) = entry.mr_state {
                    let input = BucketInput {
                        mr_state: mr_state.clone(),
                        ci_status: entry.ci_status.clone(),
                        approved: entry.approved,
                        changes_requested: entry.changes_requested,
                        mergeable: entry.mergeable,
                        behind_base: is_behind_base,
                    };

                    if let Some(b) = bucket(&input) {
                        // Skip merged items unless --all
                        if b == ActionBucket::Merged && !all {
                            continue;
                        }

                        items.push(InboxItem {
                            stack_name: stack_name.clone(),
                            position: entry.position,
                            short_sha: entry.short_sha.clone(),
                            title: entry.title.clone(),
                            mr_number: pr_num,
                            mr_url: entry.mr_url.clone().unwrap_or_default(),
                            bucket: b,
                            ci_status: entry.ci_status.clone(),
                        });
                    }
                }
            }
        }
    }

    if !json {
        eprintln!("{}", style("done").green());
    }

    if json {
        print_inbox_json(&items);
    } else {
        print_inbox_human(&items, &provider);
    }

    Ok(())
}

fn print_inbox_json(items: &[InboxItem]) {
    let mut buckets = InboxBucketsJson {
        ready_to_land: Vec::new(),
        changes_requested: Vec::new(),
        blocked_on_ci: Vec::new(),
        awaiting_review: Vec::new(),
        behind_base: Vec::new(),
        draft: Vec::new(),
        merged: Vec::new(),
    };

    for item in items {
        let entry = InboxEntryJson {
            stack_name: item.stack_name.clone(),
            position: item.position,
            sha: item.short_sha.clone(),
            title: item.title.clone(),
            pr_number: item.mr_number,
            pr_url: item.mr_url.clone(),
            ci_status: item.ci_status.as_ref().map(ci_status_str),
        };

        match item.bucket {
            ActionBucket::ReadyToLand => buckets.ready_to_land.push(entry),
            ActionBucket::ChangesRequested => buckets.changes_requested.push(entry),
            ActionBucket::BlockedOnCi => buckets.blocked_on_ci.push(entry),
            ActionBucket::AwaitingReview => buckets.awaiting_review.push(entry),
            ActionBucket::BehindBase => buckets.behind_base.push(entry),
            ActionBucket::Draft => buckets.draft.push(entry),
            ActionBucket::Merged => buckets.merged.push(entry),
        }
    }

    let response = InboxResponse {
        version: OUTPUT_VERSION,
        total_items: items.len(),
        buckets,
    };

    print_json(&response);
}

fn print_inbox_human(items: &[InboxItem], provider: &Provider) {
    if items.is_empty() {
        println!(
            "{}",
            style("Inbox is empty — nothing needs attention.").dim()
        );
        return;
    }

    // Count unique stacks
    let mut stack_names: Vec<&str> = items.iter().map(|i| i.stack_name.as_str()).collect();
    stack_names.sort();
    stack_names.dedup();

    println!();
    println!(
        "{}",
        style(format!(
            "Inbox ({} items across {} stacks)",
            items.len(),
            stack_names.len()
        ))
        .bold()
    );

    let bucket_order = [
        ActionBucket::ReadyToLand,
        ActionBucket::ChangesRequested,
        ActionBucket::BlockedOnCi,
        ActionBucket::AwaitingReview,
        ActionBucket::BehindBase,
        ActionBucket::Draft,
        ActionBucket::Merged,
    ];

    let pr_prefix = provider.pr_number_prefix();

    for &b in &bucket_order {
        let bucket_items: Vec<&InboxItem> = items.iter().filter(|i| i.bucket == b).collect();
        if bucket_items.is_empty() {
            continue;
        }

        println!();
        println!("{} ({}):", bucket_style(b, b.label()), bucket_items.len());

        for item in &bucket_items {
            let ci_icon = match (&item.ci_status, b) {
                (Some(CiStatus::Running), _) => " ⏳",
                (Some(CiStatus::Pending), _) => " ⏳",
                (Some(CiStatus::Failed), _) => " ✗",
                _ => "",
            };

            println!(
                "  {} {}  {}  {}  {}{}",
                style(&item.stack_name).cyan(),
                style(format!("#{}", item.position)).dim(),
                style(&item.short_sha).yellow(),
                item.title,
                style(format!("{}{}", pr_prefix, item.mr_number)).blue(),
                ci_icon,
            );
        }
    }

    println!();
}

fn bucket_style(bucket: ActionBucket, text: &str) -> console::StyledObject<String> {
    let s = text.to_string();
    match bucket {
        ActionBucket::ReadyToLand => style(s).green().bold(),
        ActionBucket::ChangesRequested => style(s).red().bold(),
        ActionBucket::BlockedOnCi => style(s).yellow().bold(),
        ActionBucket::AwaitingReview => style(s).cyan().bold(),
        ActionBucket::BehindBase => style(s).magenta().bold(),
        ActionBucket::Draft => style(s).dim().bold(),
        ActionBucket::Merged => style(s).dim(),
    }
}

fn ci_status_str(status: &CiStatus) -> String {
    match status {
        CiStatus::Pending => "pending".to_string(),
        CiStatus::Running => "running".to_string(),
        CiStatus::Success => "success".to_string(),
        CiStatus::Failed => "failed".to_string(),
        CiStatus::Canceled => "canceled".to_string(),
        CiStatus::Unknown => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_input(
        state: PrState,
        ci: Option<CiStatus>,
        approved: bool,
        changes_requested: bool,
        mergeable: bool,
        behind_base: bool,
    ) -> BucketInput {
        BucketInput {
            mr_state: state,
            ci_status: ci,
            approved,
            changes_requested,
            mergeable,
            behind_base,
        }
    }

    #[test]
    fn merged_always_merged() {
        let input = make_input(
            PrState::Merged,
            Some(CiStatus::Failed),
            true,
            true,
            true,
            true,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::Merged));
    }

    #[test]
    fn closed_is_skipped() {
        let input = make_input(PrState::Closed, None, false, false, false, false);
        assert_eq!(bucket(&input), None);
    }

    #[test]
    fn draft_takes_priority_over_changes_requested() {
        let input = make_input(PrState::Draft, None, false, true, false, false);
        assert_eq!(bucket(&input), Some(ActionBucket::Draft));
    }

    #[test]
    fn changes_requested_takes_priority_over_ready_to_land() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Success),
            true,
            true,
            true,
            false,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::ChangesRequested));
    }

    #[test]
    fn ready_to_land_requires_approved_ci_green_mergeable() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Success),
            true,
            false,
            true,
            false,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::ReadyToLand));
    }

    #[test]
    fn ready_to_land_approved_no_ci_mergeable() {
        // No CI = treat as green (no branch protection CI requirement)
        let input = make_input(PrState::Open, None, true, false, true, false);
        assert_eq!(bucket(&input), Some(ActionBucket::ReadyToLand));
    }

    #[test]
    fn not_ready_if_not_approved() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Success),
            false,
            false,
            true,
            false,
        );
        // Not approved, CI green → AwaitingReview (no CI blocker)
        assert_eq!(bucket(&input), Some(ActionBucket::AwaitingReview));
    }

    #[test]
    fn not_ready_if_not_mergeable() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Success),
            true,
            false,
            false,
            false,
        );
        // Approved, CI green, not mergeable → AwaitingReview
        assert_eq!(bucket(&input), Some(ActionBucket::AwaitingReview));
    }

    #[test]
    fn ci_failed_blocks() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Failed),
            true,
            false,
            true,
            false,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::BlockedOnCi));
    }

    #[test]
    fn ci_running_blocks() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Running),
            false,
            false,
            true,
            false,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::BlockedOnCi));
    }

    #[test]
    fn ci_pending_blocks() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Pending),
            false,
            false,
            true,
            false,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::BlockedOnCi));
    }

    #[test]
    fn behind_base_when_ci_unknown() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Unknown),
            false,
            false,
            true,
            true,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::BehindBase));
    }

    #[test]
    fn behind_base_when_no_ci() {
        let input = make_input(PrState::Open, None, false, false, true, true);
        assert_eq!(bucket(&input), Some(ActionBucket::BehindBase));
    }

    #[test]
    fn awaiting_review_fallthrough() {
        let input = make_input(PrState::Open, None, false, false, true, false);
        assert_eq!(bucket(&input), Some(ActionBucket::AwaitingReview));
    }

    #[test]
    fn ci_failure_takes_priority_over_behind_base() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Failed),
            false,
            false,
            true,
            true,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::BlockedOnCi));
    }

    // JSON serialization tests

    #[test]
    fn inbox_response_serializes() {
        let response = InboxResponse {
            version: OUTPUT_VERSION,
            total_items: 2,
            buckets: InboxBucketsJson {
                ready_to_land: vec![InboxEntryJson {
                    stack_name: "auth".to_string(),
                    position: 1,
                    sha: "abc1234".to_string(),
                    title: "Add auth".to_string(),
                    pr_number: 42,
                    pr_url: "https://github.com/user/repo/pull/42".to_string(),
                    ci_status: Some("success".to_string()),
                }],
                changes_requested: Vec::new(),
                blocked_on_ci: Vec::new(),
                awaiting_review: vec![InboxEntryJson {
                    stack_name: "auth".to_string(),
                    position: 2,
                    sha: "def5678".to_string(),
                    title: "Add middleware".to_string(),
                    pr_number: 43,
                    pr_url: "https://github.com/user/repo/pull/43".to_string(),
                    ci_status: None,
                }],
                behind_base: Vec::new(),
                draft: Vec::new(),
                merged: Vec::new(),
            },
        };

        let value = serde_json::to_value(&response).expect("should serialize");
        assert_eq!(value["version"], OUTPUT_VERSION);
        assert_eq!(value["total_items"], 2);
        assert_eq!(value["buckets"]["ready_to_land"][0]["pr_number"], 42);
        assert_eq!(value["buckets"]["awaiting_review"][0]["pr_number"], 43);
        // merged should be omitted when empty
        assert!(value["buckets"].get("merged").is_none());
    }

    #[test]
    fn action_bucket_label() {
        assert_eq!(ActionBucket::ReadyToLand.label(), "Ready to land");
        assert_eq!(ActionBucket::ChangesRequested.label(), "Changes requested");
        assert_eq!(ActionBucket::BlockedOnCi.label(), "Blocked on CI");
        assert_eq!(ActionBucket::AwaitingReview.label(), "Awaiting review");
        assert_eq!(ActionBucket::BehindBase.label(), "Behind base");
        assert_eq!(ActionBucket::Draft.label(), "Draft");
        assert_eq!(ActionBucket::Merged.label(), "Merged");
    }
}
