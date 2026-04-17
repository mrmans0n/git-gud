//! Inbox command — multi-stack actionable triage view.

use std::collections::HashMap;

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
use crate::provider::{CiStatus, PrState, Provider};
use crate::stack;

/// Action bucket for triage classification.
///
/// Evaluated in priority order — first match wins.
/// Ordering also controls display order (most urgent first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
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

/// Input fields for bucketing. Decoupled from StackEntry so the function is pure and testable.
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
/// 1. Merged → Merged
/// 2. Closed → None (skip)
/// 3. Draft → Draft
/// 4. Changes requested → ChangesRequested
/// 5. Approved + CI green + mergeable → ReadyToLand
/// 6. CI failed/running/pending → BlockedOnCi
/// 7. Behind base → BehindBase
/// 8. Fallthrough → AwaitingReview
pub fn bucket(input: &BucketInput) -> Option<ActionBucket> {
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
        let target = head_ref.symbolic_target()?;
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

struct StackLoadError {
    stack_name: String,
    error: String,
}

/// Internal item representing one triaged PR.
struct InboxItem {
    stack_name: String,
    position: usize,
    short_sha: String,
    title: String,
    mr_number: u64,
    mr_url: String,
    bucket: ActionBucket,
    ci_status: Option<CiStatus>,
    behind_base: Option<usize>,
}

/// Run the inbox command.
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

    if usernames.is_empty() {
        if let Ok(provider) = Provider::detect(repo) {
            if let Ok(username) = provider.whoami() {
                usernames.push(username);
            }
        }
    }

    Ok(usernames)
}

pub fn run(all: bool, json: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;

    let usernames = infer_stack_usernames(&repo, &config)?;
    if usernames.is_empty() {
        return Err(GgError::Config(
            "Missing branch_username and could not infer one from provider or local stack branches. Set defaults.branch_username in config.".to_string(),
        ));
    }

    for username in &usernames {
        git::validate_branch_username(username)?;
    }

    let mut stack_branches: Vec<(String, String)> = Vec::new();
    for username in &usernames {
        for stack_name in stack::list_all_stacks(&repo, &config, username)? {
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

    if stack_branches.is_empty() {
        if json {
            print_json_output(&[], &[]);
        } else {
            println!(
                "{}",
                style("Inbox is empty — nothing needs attention.").dim()
            );
        }
        return Ok(());
    }

    if !json {
        eprint!("{}", style("Refreshing PR status...").dim());
    }

    let mut items: Vec<InboxItem> = Vec::new();
    let mut stack_errors: Vec<StackLoadError> = Vec::new();

    for (stack_name, full_branch) in &stack_branches {
        let base = match resolve_base_branch(&repo, &config, stack_name) {
            Ok(base) => base,
            Err(err) => {
                stack_errors.push(StackLoadError {
                    stack_name: stack_name.clone(),
                    error: err.to_string(),
                });
                continue;
            }
        };
        let mut entries = match load_stack_entries(&repo, &base, full_branch) {
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

        let provider = if entries.iter().any(|entry| entry.mr_number.is_some()) {
            Provider::detect(&repo).ok()
        } else {
            None
        };

        // Refresh MR info from provider and cache URLs (T9 optimization)
        let mut mr_urls: HashMap<u64, String> = HashMap::new();
        for entry in &mut entries {
            if let (Some(pr_num), Some(provider)) = (entry.mr_number, provider) {
                if let Ok(info) = provider.get_pr_info(pr_num) {
                    entry.mr_state = Some(info.state);
                    entry.approved = info.approved;
                    entry.changes_requested = info.changes_requested;
                    entry.mergeable = info.mergeable;
                    mr_urls.insert(pr_num, info.url);
                }
                if let Ok(ci) = provider.get_pr_ci_status(pr_num) {
                    entry.ci_status = Some(ci);
                }
                if let Ok(approved) = provider.check_pr_approved(pr_num) {
                    if approved || !entry.approved {
                        entry.approved = approved;
                    }
                }
            }
        }

        // Compute behind-base from the actual stack tip, not the local base branch.
        // This avoids false positives when local `<base>` is stale but the stack
        // itself has already been rebased onto `origin/<base>`.
        let behind =
            git::count_branch_behind_upstream(&repo, full_branch, &format!("origin/{}", base))
                .ok()
                .filter(|&b| b > 0);

        // Bucket each entry with a PR
        for entry in &entries {
            if let Some(mr_num) = entry.mr_number {
                let mr_url = mr_urls.get(&mr_num).cloned().unwrap_or_default();

                // If provider refresh failed (mr_state is None), default to
                // PrState::Open so entries remain visible in the inbox rather
                // than silently disappearing during transient failures.
                let mr_state = entry.mr_state.clone().unwrap_or(PrState::Open);

                let input = BucketInput {
                    mr_state,
                    ci_status: entry.ci_status.clone(),
                    approved: entry.approved,
                    changes_requested: entry.changes_requested,
                    mergeable: entry.mergeable,
                    behind_base: behind.is_some(),
                };

                if let Some(b) = bucket(&input) {
                    items.push(InboxItem {
                        stack_name: stack_name.clone(),
                        position: entry.position,
                        short_sha: entry.short_sha.clone(),
                        title: entry.title.clone(),
                        mr_number: mr_num,
                        mr_url,
                        bucket: b,
                        ci_status: entry.ci_status.clone(),
                        behind_base: behind,
                    });
                }
            }
        }
    }

    if !json {
        eprintln!(" {}", style("done").green());
    }

    // Filter out merged unless --all
    if !all {
        items.retain(|item| item.bucket != ActionBucket::Merged);
    }

    if json {
        print_json_output(&items, &stack_errors);
    } else {
        print_human_output(&items, &stack_errors);
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
        ActionBucket::ReadyToLand,
        ActionBucket::ChangesRequested,
        ActionBucket::BlockedOnCi,
        ActionBucket::AwaitingReview,
        ActionBucket::BehindBase,
        ActionBucket::Draft,
        ActionBucket::Merged,
    ];

    for b in &bucket_order {
        let group: Vec<&InboxItem> = items.iter().filter(|i| &i.bucket == b).collect();
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
                "  {} {}  {}  {}  PR #{}{}",
                style(format!("{} #{}", item.stack_name, item.position)).dim(),
                style(&item.short_sha).dim(),
                item.title,
                style(format!("stack/{}", item.stack_name)).cyan(),
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
            mr_state: state,
            ci_status: ci,
            approved,
            changes_requested,
            mergeable,
            behind_base,
        }
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
