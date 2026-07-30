//! Inbox command — multi-stack actionable triage view.

use console::style;
use serde::Serialize;

use crate::config::Config;
use crate::error::GgError;
use crate::error::Result;
use crate::git;
use crate::output::{
    print_json, InboxBucketsJson, InboxEntryJson, InboxResponse, InboxStackErrorJson,
    OUTPUT_VERSION,
};
use crate::provider::{CiStatus, InboxSnapshot, PrState, Provider};
use crate::stack;

/// Action bucket for triage classification.
///
/// Evaluated in priority order — first match wins.
/// Ordering also controls display order (most urgent first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionBucket {
    RefreshFailed,
    ReadyToLand,
    ChangesRequested,
    BlockedOnCi,
    AwaitingReview,
    BehindBase,
    Draft,
    Merged,
}

/// Input fields for bucketing. Decoupled from StackEntry so the function is pure and testable.
pub struct BucketInput {
    pub refresh_failed: bool,
    pub mr_state: PrState,
    pub ci_status: Option<CiStatus>,
    pub approved: bool,
    pub changes_requested: bool,
    pub mergeable: bool,
    pub behind_base: bool,
}

/// Classify a PR/MR into an action bucket.
///
/// Priority order (first match wins):
/// 1. Refresh failure → RefreshFailed
/// 2. Merged → Merged
/// 3. Closed → None (skip)
/// 4. Draft → Draft
/// 5. Changes requested → ChangesRequested
/// 6. Approved + CI green + mergeable → ReadyToLand
/// 7. CI failed/running/pending → BlockedOnCi
/// 8. Behind base → BehindBase
/// 9. Fallthrough → AwaitingReview
pub fn bucket(input: &BucketInput) -> Option<ActionBucket> {
    if input.refresh_failed {
        return Some(ActionBucket::RefreshFailed);
    }

    match input.mr_state {
        PrState::Merged => return Some(ActionBucket::Merged),
        PrState::Closed => return None,
        PrState::Draft => return Some(ActionBucket::Draft),
        PrState::Open => {}
    }

    if input.changes_requested {
        return Some(ActionBucket::ChangesRequested);
    }

    if input.approved && input.mergeable {
        let ci_green = matches!(input.ci_status, Some(CiStatus::Success) | None);
        if ci_green {
            return Some(ActionBucket::ReadyToLand);
        }
    }

    match input.ci_status {
        Some(CiStatus::Failed)
        | Some(CiStatus::Running)
        | Some(CiStatus::Pending)
        | Some(CiStatus::Canceled) => {
            return Some(ActionBucket::BlockedOnCi);
        }
        _ => {}
    }

    if input.behind_base {
        return Some(ActionBucket::BehindBase);
    }

    Some(ActionBucket::AwaitingReview)
}

fn resolve_base_branch(
    repo: &git2::Repository,
    config: &Config,
    stack_name: &str,
) -> Result<String> {
    fn remote_head_base_branch(repo: &git2::Repository) -> Option<String> {
        let head_ref = repo.find_reference("refs/remotes/origin/HEAD").ok()?;
        let target = head_ref.symbolic_target().ok().flatten()?;
        let branch = target.strip_prefix("refs/remotes/origin/")?;
        repo.find_reference(target).ok()?;
        Some(branch.to_string())
    }

    config
        .get_base_for_stack(stack_name)
        .map(|base| base.to_string())
        .or_else(|| remote_head_base_branch(repo))
        .or_else(|| git::find_base_branch(repo).ok())
        .ok_or(GgError::NoBaseBranch)
}

fn load_stack_entries(
    repo: &git2::Repository,
    base: &str,
    full_branch: &str,
) -> Result<Vec<stack::StackEntry>> {
    let oids = git::get_stack_commit_oids(repo, base, Some(full_branch))?;

    oids.iter()
        .enumerate()
        .map(|(i, oid)| -> Result<stack::StackEntry> {
            let commit = repo.find_commit(*oid)?;
            Ok(stack::StackEntry::from_commit(&commit, i + 1))
        })
        .collect()
}

pub(super) struct StackLoadError {
    stack_name: String,
    error: String,
}

/// Internal item representing one triaged PR/MR.
struct InboxItem {
    stack_name: String,
    position: usize,
    short_sha: String,
    title: String,
    mr_number: u64,
    mr_url: String,
    mr_label: &'static str,
    mr_number_prefix: &'static str,
    ci_status: Option<CiStatus>,
    approved: bool,
    changes_requested: bool,
    mergeable: bool,
    behind_base: Option<usize>,
    remote_state: Option<PrState>,
    refresh_error: Option<String>,
}

impl InboxItem {
    fn from_snapshot(
        candidate: InboxCandidate,
        snapshot: InboxSnapshot,
        provider: Provider,
    ) -> Self {
        Self {
            stack_name: candidate.stack_name,
            position: candidate.position,
            short_sha: candidate.short_sha,
            title: candidate.title,
            mr_number: candidate.pr_number,
            mr_url: snapshot.url,
            mr_label: provider.pr_label(),
            mr_number_prefix: provider.pr_number_prefix(),
            ci_status: snapshot.ci_status,
            approved: snapshot.approved,
            changes_requested: snapshot.changes_requested,
            mergeable: snapshot.mergeable,
            behind_base: candidate.behind_base,
            remote_state: Some(snapshot.state),
            refresh_error: None,
        }
    }

    fn from_refresh_error(candidate: InboxCandidate, error: String, provider: Provider) -> Self {
        Self {
            stack_name: candidate.stack_name,
            position: candidate.position,
            short_sha: candidate.short_sha,
            title: candidate.title,
            mr_number: candidate.pr_number,
            mr_url: String::new(),
            mr_label: provider.pr_label(),
            mr_number_prefix: provider.pr_number_prefix(),
            ci_status: None,
            approved: false,
            changes_requested: false,
            mergeable: false,
            behind_base: candidate.behind_base,
            remote_state: None,
            refresh_error: Some(error),
        }
    }

    fn bucket(&self) -> Option<ActionBucket> {
        bucket(&BucketInput {
            refresh_failed: self.refresh_error.is_some(),
            mr_state: self.remote_state.clone().unwrap_or(PrState::Open),
            ci_status: self.ci_status.clone(),
            approved: self.approved,
            changes_requested: self.changes_requested,
            mergeable: self.mergeable,
            behind_base: self.behind_base.is_some(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InboxCandidate {
    pub discovery_index: usize,
    pub stack_name: String,
    pub position: usize,
    pub short_sha: String,
    pub title: String,
    pub pr_number: u64,
    pub behind_base: Option<usize>,
}

pub(super) struct InboxDiscovery {
    pub candidates: Vec<InboxCandidate>,
    pub stack_errors: Vec<StackLoadError>,
}

fn infer_stack_usernames(repo: &git2::Repository, config: &Config) -> Result<Vec<String>> {
    let mut usernames = Vec::new();

    if let Some(username) = config.defaults.branch_username.clone() {
        usernames.push(username);
    }

    for branch_result in repo.branches(Some(git2::BranchType::Local))? {
        let (branch, _) = branch_result?;
        if let Some(name) = branch.name()? {
            if let Some((branch_user, _)) = git::parse_stack_branch(name) {
                if !usernames.contains(&branch_user) {
                    usernames.push(branch_user);
                }
            } else if let Some((branch_user, _, _)) = git::parse_entry_branch(name) {
                if !usernames.contains(&branch_user) {
                    usernames.push(branch_user);
                }
            }
        }
    }

    Ok(usernames)
}

fn discover_candidates(repo: &git2::Repository, config: &Config) -> Result<InboxDiscovery> {
    let usernames = infer_stack_usernames(repo, config)?;
    let valid_usernames: Vec<String> = usernames
        .into_iter()
        .filter(|username| git::validate_branch_username(username).is_ok())
        .collect();

    let mut stack_branches: Vec<(String, String)> = Vec::new();
    for username in &valid_usernames {
        for stack_name in stack::list_all_stacks(repo, config, username)? {
            let full_branch = git::format_stack_branch(username, &stack_name);
            if repo
                .find_branch(&full_branch, git2::BranchType::Local)
                .is_err()
            {
                continue;
            }
            if !stack_branches
                .iter()
                .any(|(name, branch)| name == &stack_name && branch == &full_branch)
            {
                stack_branches.push((stack_name, full_branch));
            }
        }
    }

    let mut stack_errors: Vec<StackLoadError> = Vec::new();
    let mut candidates = Vec::new();

    for (stack_name, full_branch) in &stack_branches {
        let base = match resolve_base_branch(repo, config, stack_name) {
            Ok(base) => base,
            Err(err) => {
                stack_errors.push(StackLoadError {
                    stack_name: stack_name.clone(),
                    error: err.to_string(),
                });
                continue;
            }
        };
        let mut entries = match load_stack_entries(repo, &base, full_branch) {
            Ok(entries) => entries,
            Err(err) => {
                stack_errors.push(StackLoadError {
                    stack_name: stack_name.clone(),
                    error: err.to_string(),
                });
                continue;
            }
        };

        if let Some(stack_config) = config.get_stack(stack_name) {
            for entry in &mut entries {
                if let Some(gg_id) = &entry.gg_id {
                    if let Some(mr_num) = stack_config.mrs.get(gg_id) {
                        entry.mr_number = Some(*mr_num);
                    }
                }
            }
        }

        // Compute behind-base from the actual stack tip, not the local base branch.
        // This avoids false positives when local `<base>` is stale but the stack
        // itself has already been rebased onto `origin/<base>`.
        let behind =
            git::count_branch_behind_upstream(repo, full_branch, &format!("origin/{}", base))
                .ok()
                .filter(|&b| b > 0);

        // Collect each entry with a mapped PR/MR for a later remote refresh.
        for entry in &entries {
            if let Some(mr_num) = entry.mr_number {
                candidates.push(InboxCandidate {
                    discovery_index: candidates.len(),
                    stack_name: stack_name.clone(),
                    position: entry.position,
                    short_sha: entry.short_sha.clone(),
                    title: entry.title.clone(),
                    pr_number: mr_num,
                    behind_base: behind,
                });
            }
        }
    }

    Ok(InboxDiscovery {
        candidates,
        stack_errors,
    })
}

/// Run the inbox command.
pub fn run(all: bool, json: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;
    let discovery = discover_candidates(&repo, &config)?;

    if discovery.candidates.is_empty() {
        if json {
            print_json_output(&[], &discovery.stack_errors);
        } else {
            print_human_output(&[], &discovery.stack_errors);
        }
        return Ok(());
    }

    let provider = Provider::detect(&repo)?;
    provider.check_installed()?;

    if !json {
        eprint!(
            "{}",
            style(format!("Refreshing {} status...", provider.pr_label())).dim()
        );
    }

    let mut items = Vec::with_capacity(discovery.candidates.len());
    for candidate in discovery.candidates {
        let item = match provider.get_inbox_snapshot(candidate.pr_number) {
            Ok(snapshot) => InboxItem::from_snapshot(candidate, snapshot, provider),
            Err(error) => InboxItem::from_refresh_error(candidate, error.to_string(), provider),
        };
        items.push(item);
    }

    if !json {
        eprintln!(" {}", style("done").green());
    }

    // Closed entries are intentionally omitted from the inbox.
    items.retain(|item| item.bucket().is_some());

    // Filter out merged unless --all
    if !all {
        items.retain(|item| item.bucket() != Some(ActionBucket::Merged));
    }

    if json {
        print_json_output(&items, &discovery.stack_errors);
    } else {
        print_human_output(&items, &discovery.stack_errors);
    }

    Ok(())
}

fn print_human_output(items: &[InboxItem], stack_errors: &[StackLoadError]) {
    if items.is_empty() {
        println!(
            "{}",
            style("Inbox is empty — nothing needs attention.").dim()
        );
        if !stack_errors.is_empty() {
            println!();
            println!("{}", style("Skipped stacks:").yellow().bold());
            for stack_error in stack_errors {
                println!(
                    "  {} {}",
                    style(&stack_error.stack_name).dim(),
                    stack_error.error
                );
            }
        }
        return;
    }

    // Count unique stacks
    let mut stack_names: Vec<&str> = items.iter().map(|i| i.stack_name.as_str()).collect();
    stack_names.sort();
    stack_names.dedup();

    println!(
        "\n{} ({} {} across {} {})\n",
        style("Inbox").bold(),
        items.len(),
        if items.len() == 1 { "item" } else { "items" },
        stack_names.len(),
        if stack_names.len() == 1 {
            "stack"
        } else {
            "stacks"
        },
    );

    let bucket_order = [
        ActionBucket::RefreshFailed,
        ActionBucket::ReadyToLand,
        ActionBucket::ChangesRequested,
        ActionBucket::BlockedOnCi,
        ActionBucket::AwaitingReview,
        ActionBucket::BehindBase,
        ActionBucket::Draft,
        ActionBucket::Merged,
    ];

    for b in &bucket_order {
        let group: Vec<&InboxItem> = items
            .iter()
            .filter(|item| item.bucket() == Some(*b))
            .collect();
        if group.is_empty() {
            continue;
        }

        println!("{} ({}):", styled_bucket_label(*b), group.len());

        for item in &group {
            let ci_icon = match &item.ci_status {
                Some(CiStatus::Running) | Some(CiStatus::Pending) => " ⏳",
                Some(CiStatus::Failed) => " ✗",
                _ => "",
            };

            println!(
                "  {} {}  {}  {}  {} {}{}{}",
                style(format!("{} #{}", item.stack_name, item.position)).dim(),
                style(&item.short_sha).dim(),
                item.title,
                style(format!("stack/{}", item.stack_name)).cyan(),
                item.mr_label,
                item.mr_number_prefix,
                item.mr_number,
                ci_icon,
            );
        }
        println!();
    }

    if !stack_errors.is_empty() {
        println!("{}", style("Skipped stacks:").yellow().bold());
        for stack_error in stack_errors {
            println!(
                "  {} {}",
                style(&stack_error.stack_name).dim(),
                stack_error.error
            );
        }
        println!();
    }
}

fn bucket_label(b: ActionBucket) -> &'static str {
    match b {
        ActionBucket::RefreshFailed => "Refresh failed",
        ActionBucket::ReadyToLand => "Ready to land",
        ActionBucket::ChangesRequested => "Changes requested",
        ActionBucket::BlockedOnCi => "Blocked on CI",
        ActionBucket::AwaitingReview => "Awaiting review",
        ActionBucket::BehindBase => "Behind base",
        ActionBucket::Draft => "Draft",
        ActionBucket::Merged => "Merged",
    }
}

fn styled_bucket_label(b: ActionBucket) -> console::StyledObject<&'static str> {
    let label = bucket_label(b);
    match b {
        ActionBucket::RefreshFailed => style(label).red().bold(),
        ActionBucket::ReadyToLand => style(label).green().bold(),
        ActionBucket::ChangesRequested => style(label).red().bold(),
        ActionBucket::BlockedOnCi => style(label).yellow().bold(),
        ActionBucket::AwaitingReview => style(label).cyan().bold(),
        ActionBucket::BehindBase => style(label).magenta().bold(),
        ActionBucket::Draft | ActionBucket::Merged => style(label).dim().bold(),
    }
}

fn print_json_output(items: &[InboxItem], stack_errors: &[StackLoadError]) {
    let mut buckets = InboxBucketsJson {
        refresh_failed: vec![],
        ready_to_land: vec![],
        changes_requested: vec![],
        blocked_on_ci: vec![],
        awaiting_review: vec![],
        behind_base: vec![],
        draft: vec![],
        merged: vec![],
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
            behind_base: item.behind_base,
            refresh_error: item.refresh_error.clone(),
        };

        match item.bucket() {
            Some(ActionBucket::RefreshFailed) => buckets.refresh_failed.push(entry),
            Some(ActionBucket::ReadyToLand) => buckets.ready_to_land.push(entry),
            Some(ActionBucket::ChangesRequested) => buckets.changes_requested.push(entry),
            Some(ActionBucket::BlockedOnCi) => buckets.blocked_on_ci.push(entry),
            Some(ActionBucket::AwaitingReview) => buckets.awaiting_review.push(entry),
            Some(ActionBucket::BehindBase) => buckets.behind_base.push(entry),
            Some(ActionBucket::Draft) => buckets.draft.push(entry),
            Some(ActionBucket::Merged) => buckets.merged.push(entry),
            None => continue,
        }
    }

    print_json(&InboxResponse {
        version: OUTPUT_VERSION,
        total_items: items.len(),
        buckets,
        stack_errors: stack_errors
            .iter()
            .map(|stack_error| InboxStackErrorJson {
                stack_name: stack_error.stack_name.clone(),
                error: stack_error.error.clone(),
            })
            .collect(),
    });
}

fn ci_status_str(ci: &CiStatus) -> String {
    match ci {
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
    use crate::provider::{CiStatus, PrState};

    fn make_input(
        state: PrState,
        ci: Option<CiStatus>,
        approved: bool,
        changes_requested: bool,
        mergeable: bool,
        behind_base: bool,
    ) -> BucketInput {
        BucketInput {
            refresh_failed: false,
            mr_state: state,
            ci_status: ci,
            approved,
            changes_requested,
            mergeable,
            behind_base,
        }
    }

    #[test]
    fn refresh_failure_has_highest_priority() {
        let input = BucketInput {
            refresh_failed: true,
            mr_state: PrState::Merged,
            ci_status: Some(CiStatus::Success),
            approved: true,
            changes_requested: true,
            mergeable: true,
            behind_base: true,
        };

        assert_eq!(bucket(&input), Some(ActionBucket::RefreshFailed));
    }

    #[test]
    fn merged_always_wins() {
        let input = make_input(
            PrState::Merged,
            Some(CiStatus::Failed),
            true,
            true,
            false,
            true,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::Merged));
    }

    #[test]
    fn closed_returns_none() {
        let input = make_input(PrState::Closed, None, false, false, false, false);
        assert_eq!(bucket(&input), None);
    }

    #[test]
    fn draft_beats_changes_requested() {
        let input = make_input(PrState::Draft, None, false, true, false, false);
        assert_eq!(bucket(&input), Some(ActionBucket::Draft));
    }

    #[test]
    fn changes_requested_beats_ready_to_land() {
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
    fn ready_to_land_approved_ci_green_mergeable() {
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
    fn approved_but_not_mergeable_is_not_ready() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Success),
            true,
            false,
            false,
            false,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::AwaitingReview));
    }

    #[test]
    fn blocked_on_ci_failed() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Failed),
            false,
            false,
            true,
            false,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::BlockedOnCi));
    }

    #[test]
    fn blocked_on_ci_running() {
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
    fn blocked_on_ci_pending() {
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
    fn behind_base_when_ci_green() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Success),
            false,
            false,
            false,
            true,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::BehindBase));
    }

    #[test]
    fn ci_failure_beats_behind_base() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Failed),
            false,
            false,
            false,
            true,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::BlockedOnCi));
    }

    #[test]
    fn fallthrough_awaiting_review() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Success),
            false,
            false,
            true,
            false,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::AwaitingReview));
    }

    #[test]
    fn awaiting_review_no_ci_no_approval() {
        let input = make_input(PrState::Open, None, false, false, false, false);
        assert_eq!(bucket(&input), Some(ActionBucket::AwaitingReview));
    }

    #[test]
    fn blocked_on_ci_canceled() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Canceled),
            false,
            false,
            true,
            false,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::BlockedOnCi));
    }

    #[test]
    fn unknown_ci_is_not_treated_like_green_for_ready_to_land() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Unknown),
            true,
            false,
            true,
            false,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::AwaitingReview));
    }

    #[test]
    fn unknown_ci_is_treated_like_absent_ci_for_review_bucket() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Unknown),
            false,
            false,
            false,
            false,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::AwaitingReview));
    }

    #[test]
    fn canceled_ci_beats_behind_base() {
        let input = make_input(
            PrState::Open,
            Some(CiStatus::Canceled),
            false,
            false,
            false,
            true,
        );
        assert_eq!(bucket(&input), Some(ActionBucket::BlockedOnCi));
    }

    #[test]
    fn action_bucket_display_order() {
        assert!(ActionBucket::RefreshFailed < ActionBucket::ReadyToLand);
        assert!(ActionBucket::ReadyToLand < ActionBucket::ChangesRequested);
        assert!(ActionBucket::ChangesRequested < ActionBucket::BlockedOnCi);
        assert!(ActionBucket::BlockedOnCi < ActionBucket::AwaitingReview);
        assert!(ActionBucket::AwaitingReview < ActionBucket::BehindBase);
        assert!(ActionBucket::BehindBase < ActionBucket::Draft);
        assert!(ActionBucket::Draft < ActionBucket::Merged);
    }

    #[test]
    fn resolve_base_branch_prefers_stack_config() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = Config::default();
        config.defaults.base = Some("develop".to_string());
        config.get_or_create_stack("feature").base = Some("release".to_string());

        let base = resolve_base_branch(&repo, &config, "feature").unwrap();
        assert_eq!(base, "release");
    }

    #[test]
    fn resolve_base_branch_falls_back_to_detected_repo_base() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        let tree_oid = {
            let mut index = repo.index().unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        let config = Config::default();
        let base = resolve_base_branch(&repo, &config, "feature").unwrap();
        assert_eq!(base, "master");
    }

    #[test]
    fn resolve_base_branch_uses_origin_head_for_custom_default_branch() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        let tree_oid = {
            let mut index = repo.index().unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_oid).unwrap();
        let commit_oid = repo
            .commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
        let commit = repo.find_commit(commit_oid).unwrap();

        repo.reference("refs/remotes/origin/develop", commit.id(), true, "test")
            .unwrap();
        repo.reference_symbolic(
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/develop",
            true,
            "test",
        )
        .unwrap();

        let config = Config::default();
        let base = resolve_base_branch(&repo, &config, "feature").unwrap();
        assert_eq!(base, "develop");
    }

    #[test]
    fn resolve_base_branch_ignores_stale_origin_head_target() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        let tree_oid = {
            let mut index = repo.index().unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        repo.reference_symbolic(
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/develop",
            true,
            "test",
        )
        .unwrap();

        let config = Config::default();
        let base = resolve_base_branch(&repo, &config, "feature").unwrap();
        assert_eq!(base, "master");
    }
}
