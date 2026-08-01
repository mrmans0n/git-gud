use std::process::Command;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::config::GithubStacksIntegration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemotePullRequestState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteStackEntry {
    pub number: u64,
    pub state: RemotePullRequestState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteStackSnapshot {
    pub number: u64,
    pub entries: Vec<RemoteStackEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcilePlan {
    Create,
    Append { stack_number: u64, delta: Vec<u64> },
    Unchanged { stack_number: u64 },
    Diverged { message: String },
}

pub fn plan_reconciliation(
    local_pr_numbers: &[u64],
    remote_stacks: &[RemoteStackSnapshot],
) -> ReconcilePlan {
    let matching_stacks: Vec<_> = remote_stacks
        .iter()
        .filter(|stack| {
            stack
                .entries
                .iter()
                .any(|entry| local_pr_numbers.contains(&entry.number))
        })
        .collect();

    let [stack] = matching_stacks.as_slice() else {
        return match matching_stacks.len() {
            0 => ReconcilePlan::Create,
            _ => diverged("Local pull requests belong to multiple native stacks"),
        };
    };

    let active_start = stack
        .entries
        .iter()
        .position(|entry| entry.state != RemotePullRequestState::Merged)
        .unwrap_or(stack.entries.len());

    let active_entries = &stack.entries[active_start..];
    if active_entries.iter().any(|entry| {
        !matches!(
            entry.state,
            RemotePullRequestState::Open | RemotePullRequestState::Draft
        )
    }) {
        return diverged("Native stack has a closed or merged entry after its active prefix");
    }

    let active_pr_numbers: Vec<_> = active_entries.iter().map(|entry| entry.number).collect();
    if active_pr_numbers == local_pr_numbers {
        ReconcilePlan::Unchanged {
            stack_number: stack.number,
        }
    } else if local_pr_numbers.starts_with(&active_pr_numbers) {
        ReconcilePlan::Append {
            stack_number: stack.number,
            delta: local_pr_numbers[active_pr_numbers.len()..].to_vec(),
        }
    } else {
        diverged("Local pull requests do not match the native stack active sequence")
    }
}

fn diverged(message: impl Into<String>) -> ReconcilePlan {
    ReconcilePlan::Diverged {
        message: message.into(),
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubStackAction {
    Created,
    Appended,
    Unchanged,
    Skipped,
    Warning,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubStackReason {
    Disabled,
    PartialSync,
    InsufficientPrs,
    UnresolvedPrs,
    MissingExtension,
    OutdatedExtension,
    UnsupportedRepository,
    Diverged,
    BackendFailed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GithubStackSyncResult {
    pub mode: GithubStacksIntegration,
    pub action: GithubStackAction,
    pub reason: Option<GithubStackReason>,
    pub stack_number: Option<u64>,
    pub pr_numbers: Vec<u64>,
    pub message: Option<String>,
}

impl GithubStackSyncResult {
    pub fn skipped(
        mode: GithubStacksIntegration,
        reason: GithubStackReason,
        pr_numbers: Vec<u64>,
    ) -> Self {
        Self {
            mode,
            action: GithubStackAction::Skipped,
            reason: Some(reason),
            stack_number: None,
            pr_numbers,
            message: None,
        }
    }

    pub fn warning(
        mode: GithubStacksIntegration,
        reason: GithubStackReason,
        pr_numbers: Vec<u64>,
        message: String,
    ) -> Self {
        Self {
            mode,
            action: GithubStackAction::Warning,
            reason: Some(reason),
            stack_number: None,
            pr_numbers,
            message: Some(message),
        }
    }

    pub fn is_warning(&self) -> bool {
        self.action == GithubStackAction::Warning
    }
}

const MIN_GH_STACK_VERSION: &str = "0.1.0";

pub struct GithubStackReconcileOutcome {
    pub result: GithubStackSyncResult,
    pub mutation_attempted: bool,
}

struct GhCommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

trait GhCommandRunner {
    fn run(&self, args: &[String]) -> std::io::Result<GhCommandOutput>;
}

struct SystemGhRunner;

impl GhCommandRunner for SystemGhRunner {
    fn run(&self, args: &[String]) -> std::io::Result<GhCommandOutput> {
        let output = Command::new("gh").args(args).output()?;
        Ok(GhCommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[derive(Deserialize)]
struct GhStackJson {
    number: u64,
    pull_requests: Vec<GhStackPullRequestJson>,
}

#[derive(Deserialize)]
struct GhStackPullRequestJson {
    number: u64,
    state: String,
    draft: bool,
    merged_at: Option<String>,
}

pub fn reconcile_with_gh(
    mode: GithubStacksIntegration,
    base: &str,
    pr_numbers: &[u64],
) -> GithubStackReconcileOutcome {
    reconcile_with_gh_before_mutation(mode, base, pr_numbers, || {})
}

/// Reconcile GitHub's native stack state and run `before_mutation` immediately
/// before invoking a mutating `gh stack link` command.
pub fn reconcile_with_gh_before_mutation(
    mode: GithubStacksIntegration,
    base: &str,
    pr_numbers: &[u64],
    before_mutation: impl FnMut(),
) -> GithubStackReconcileOutcome {
    reconcile_with_runner_before_mutation(&SystemGhRunner, mode, base, pr_numbers, before_mutation)
}

#[cfg(test)]
fn reconcile_with_runner<R: GhCommandRunner>(
    runner: &R,
    mode: GithubStacksIntegration,
    base: &str,
    pr_numbers: &[u64],
) -> GithubStackReconcileOutcome {
    reconcile_with_runner_before_mutation(runner, mode, base, pr_numbers, || {})
}

fn reconcile_with_runner_before_mutation<R: GhCommandRunner>(
    runner: &R,
    mode: GithubStacksIntegration,
    base: &str,
    pr_numbers: &[u64],
    mut before_mutation: impl FnMut(),
) -> GithubStackReconcileOutcome {
    if pr_numbers.is_empty() {
        return outcome(GithubStackSyncResult::skipped(
            mode,
            GithubStackReason::InsufficientPrs,
            Vec::new(),
        ));
    }

    if mode == GithubStacksIntegration::Off {
        return outcome(GithubStackSyncResult::skipped(
            mode,
            GithubStackReason::Disabled,
            pr_numbers.to_vec(),
        ));
    }

    let version_output = match runner.run(&args(["stack", "--version"])) {
        Ok(output) => output,
        Err(error) => return backend_warning(mode, pr_numbers, error.to_string()),
    };
    if !version_output.success {
        return capability_outcome(
            mode,
            GithubStackReason::MissingExtension,
            pr_numbers,
            output_message(&version_output),
        );
    }
    let version = match parse_gh_stack_version(&version_output.stdout) {
        Ok(version) => version,
        Err(message) => return backend_warning(mode, pr_numbers, message),
    };
    let minimum = Version::parse(MIN_GH_STACK_VERSION).expect("minimum version is valid");
    if version < minimum {
        return capability_outcome(
            mode,
            GithubStackReason::OutdatedExtension,
            pr_numbers,
            format!("gh-stack {version} is older than required {minimum}"),
        );
    }

    let remote_stacks = match discover_stacks(runner, pr_numbers) {
        Ok(stacks) => stacks,
        Err(DiscoveryError::Unsupported(message)) => {
            return capability_outcome(
                mode,
                GithubStackReason::UnsupportedRepository,
                pr_numbers,
                message,
            );
        }
        Err(DiscoveryError::Backend(message)) => return backend_warning(mode, pr_numbers, message),
    };

    match plan_reconciliation(pr_numbers, &remote_stacks) {
        ReconcilePlan::Create => {
            let mut link_args = args(["stack", "link", "--base", base]);
            link_args.extend(pr_numbers.iter().map(ToString::to_string));
            before_mutation();
            match run_link(runner, &link_args) {
                Ok(()) => {
                    let confirmation = discover_stacks(runner, &pr_numbers[..1]);
                    let (stack_number, message) = match confirmation {
                        Ok(stacks) => {
                            let stack_number = stacks
                                .iter()
                                .find(|stack| {
                                    stack
                                        .entries
                                        .iter()
                                        .any(|entry| entry.number == pr_numbers[0])
                                })
                                .map(|stack| stack.number);
                            let message = stack_number
                                .is_none()
                                .then(|| "Created native stack could not be confirmed".to_string());
                            (stack_number, message)
                        }
                        Err(error) => (None, Some(error.message())),
                    };
                    GithubStackReconcileOutcome {
                        result: GithubStackSyncResult {
                            mode,
                            action: GithubStackAction::Created,
                            reason: None,
                            stack_number,
                            pr_numbers: pr_numbers.to_vec(),
                            message,
                        },
                        mutation_attempted: true,
                    }
                }
                Err(message) => mutation_warning(mode, pr_numbers, message),
            }
        }
        ReconcilePlan::Append {
            stack_number,
            delta,
        } => {
            let mut link_args = args(["stack", "link", &stack_number.to_string()]);
            link_args.extend(delta.iter().map(ToString::to_string));
            before_mutation();
            match run_link(runner, &link_args) {
                Ok(()) => GithubStackReconcileOutcome {
                    result: GithubStackSyncResult {
                        mode,
                        action: GithubStackAction::Appended,
                        reason: None,
                        stack_number: Some(stack_number),
                        pr_numbers: pr_numbers.to_vec(),
                        message: None,
                    },
                    mutation_attempted: true,
                },
                Err(message) => mutation_warning(mode, pr_numbers, message),
            }
        }
        ReconcilePlan::Unchanged { stack_number } => GithubStackReconcileOutcome {
            result: GithubStackSyncResult {
                mode,
                action: GithubStackAction::Unchanged,
                reason: None,
                stack_number: Some(stack_number),
                pr_numbers: pr_numbers.to_vec(),
                message: None,
            },
            mutation_attempted: false,
        },
        ReconcilePlan::Diverged { message } => GithubStackReconcileOutcome {
            result: GithubStackSyncResult::warning(
                mode,
                GithubStackReason::Diverged,
                pr_numbers.to_vec(),
                message,
            ),
            mutation_attempted: false,
        },
    }
}

fn args<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(ToString::to_string).collect()
}

fn outcome(result: GithubStackSyncResult) -> GithubStackReconcileOutcome {
    GithubStackReconcileOutcome {
        result,
        mutation_attempted: false,
    }
}

fn capability_outcome(
    mode: GithubStacksIntegration,
    reason: GithubStackReason,
    pr_numbers: &[u64],
    message: String,
) -> GithubStackReconcileOutcome {
    if mode == GithubStacksIntegration::Force {
        outcome(GithubStackSyncResult::warning(
            mode,
            reason,
            pr_numbers.to_vec(),
            message,
        ))
    } else {
        outcome(GithubStackSyncResult::skipped(
            mode,
            reason,
            pr_numbers.to_vec(),
        ))
    }
}

fn backend_warning(
    mode: GithubStacksIntegration,
    pr_numbers: &[u64],
    message: String,
) -> GithubStackReconcileOutcome {
    outcome(GithubStackSyncResult::warning(
        mode,
        GithubStackReason::BackendFailed,
        pr_numbers.to_vec(),
        message,
    ))
}

fn mutation_warning(
    mode: GithubStacksIntegration,
    pr_numbers: &[u64],
    message: String,
) -> GithubStackReconcileOutcome {
    GithubStackReconcileOutcome {
        result: GithubStackSyncResult::warning(
            mode,
            GithubStackReason::BackendFailed,
            pr_numbers.to_vec(),
            message,
        ),
        mutation_attempted: true,
    }
}

fn parse_gh_stack_version(output: &str) -> Result<Version, String> {
    output
        .split_whitespace()
        .find_map(|token| Version::parse(token.trim_start_matches('v')).ok())
        .ok_or_else(|| "Could not parse gh-stack version output".to_string())
}

fn parse_stack_list(output: &str) -> Result<Vec<RemoteStackSnapshot>, serde_json::Error> {
    serde_json::from_str::<Vec<GhStackJson>>(output).map(|stacks| {
        stacks
            .into_iter()
            .map(|stack| RemoteStackSnapshot {
                number: stack.number,
                entries: stack
                    .pull_requests
                    .into_iter()
                    .map(|pull_request| RemoteStackEntry {
                        number: pull_request.number,
                        state: remote_state(pull_request),
                    })
                    .collect(),
            })
            .collect()
    })
}

fn remote_state(pull_request: GhStackPullRequestJson) -> RemotePullRequestState {
    if pull_request.merged_at.is_some() {
        RemotePullRequestState::Merged
    } else if pull_request.draft && pull_request.state.eq_ignore_ascii_case("open") {
        RemotePullRequestState::Draft
    } else if pull_request.state.eq_ignore_ascii_case("open") {
        RemotePullRequestState::Open
    } else {
        RemotePullRequestState::Closed
    }
}

enum DiscoveryError {
    Unsupported(String),
    Backend(String),
}

impl DiscoveryError {
    fn message(self) -> String {
        match self {
            Self::Unsupported(message) | Self::Backend(message) => message,
        }
    }
}

fn discover_stacks<R: GhCommandRunner>(
    runner: &R,
    pr_numbers: &[u64],
) -> Result<Vec<RemoteStackSnapshot>, DiscoveryError> {
    let mut stacks = Vec::new();
    for number in pr_numbers {
        let endpoint = format!("repos/{{owner}}/{{repo}}/stacks?pull_request={number}");
        let output = runner
            .run(&args(["api", endpoint.as_str()]))
            .map_err(|error| DiscoveryError::Backend(error.to_string()))?;
        if !output.success {
            let message = output_message(&output);
            return if message.contains("HTTP 404") {
                Err(DiscoveryError::Unsupported(message))
            } else {
                Err(DiscoveryError::Backend(message))
            };
        }
        let response_stacks = parse_stack_list(&output.stdout)
            .map_err(|error| DiscoveryError::Backend(error.to_string()))?;
        for stack in response_stacks {
            if !stacks
                .iter()
                .any(|existing: &RemoteStackSnapshot| existing.number == stack.number)
            {
                stacks.push(stack);
            }
        }
    }
    Ok(stacks)
}

fn run_link<R: GhCommandRunner>(runner: &R, args: &[String]) -> Result<(), String> {
    let output = runner.run(args).map_err(|error| error.to_string())?;
    if output.success {
        Ok(())
    } else {
        Err(output_message(&output))
    }
}

fn output_message(output: &GhCommandOutput) -> String {
    let message = if !output.stderr.trim().is_empty() {
        output.stderr.trim()
    } else if !output.stdout.trim().is_empty() {
        output.stdout.trim()
    } else {
        "gh command exited unsuccessfully"
    };
    message.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    const STACK_7: &str = r#"[{
  "number": 7,
  "pull_requests": [
    {"number": 40, "state": "closed", "draft": false, "merged_at": "2026-07-01T00:00:00Z"},
    {"number": 41, "state": "open", "draft": false, "merged_at": null}
  ]
}]"#;

    const STACK_7_ACTIVE: &str = r#"[{
  "number": 7,
  "pull_requests": [
    {"number": 40, "state": "closed", "draft": false, "merged_at": "2026-07-01T00:00:00Z"},
    {"number": 41, "state": "open", "draft": false, "merged_at": null}
  ]
}]"#;

    const STACK_7_CREATED: &str = r#"[{
  "number": 7,
  "pull_requests": [
    {"number": 41, "state": "open", "draft": false, "merged_at": null},
    {"number": 42, "state": "open", "draft": false, "merged_at": null}
  ]
}]"#;

    struct ExpectedCommand {
        args: Vec<String>,
        output: GhCommandOutput,
    }

    struct FakeRunner {
        commands: RefCell<VecDeque<ExpectedCommand>>,
    }

    impl FakeRunner {
        fn new(commands: impl IntoIterator<Item = ExpectedCommand>) -> Self {
            Self {
                commands: RefCell::new(commands.into_iter().collect()),
            }
        }

        fn assert_exhausted(&self) {
            assert!(
                self.commands.borrow().is_empty(),
                "unexpected commands remain"
            );
        }
    }

    impl GhCommandRunner for FakeRunner {
        fn run(&self, args: &[String]) -> std::io::Result<GhCommandOutput> {
            let expected = self
                .commands
                .borrow_mut()
                .pop_front()
                .expect("unexpected gh command");
            assert_eq!(args, expected.args.as_slice());
            Ok(expected.output)
        }
    }

    struct CallbackOrderRunner {
        inner: FakeRunner,
        callback_seen: Rc<Cell<bool>>,
    }

    impl GhCommandRunner for CallbackOrderRunner {
        fn run(&self, args: &[String]) -> std::io::Result<GhCommandOutput> {
            if args.first().map(String::as_str) == Some("stack")
                && args.get(1).map(String::as_str) == Some("link")
            {
                assert!(
                    self.callback_seen.get(),
                    "pre-mutation callback must run before gh stack link"
                );
            }
            self.inner.run(args)
        }
    }

    fn response(args: &[&str], success: bool, stdout: &str, stderr: &str) -> ExpectedCommand {
        ExpectedCommand {
            args: args.iter().map(ToString::to_string).collect(),
            output: GhCommandOutput {
                success,
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
            },
        }
    }

    fn entry(number: u64, state: RemotePullRequestState) -> RemoteStackEntry {
        RemoteStackEntry { number, state }
    }

    #[test]
    fn plans_create_when_no_local_pr_is_stacked() {
        assert_eq!(plan_reconciliation(&[41, 42], &[]), ReconcilePlan::Create);
    }

    #[test]
    fn plans_unchanged_for_exact_active_sequence_after_merged_prefix() {
        let remote = RemoteStackSnapshot {
            number: 7,
            entries: vec![
                entry(40, RemotePullRequestState::Merged),
                entry(41, RemotePullRequestState::Open),
                entry(42, RemotePullRequestState::Draft),
            ],
        };
        assert_eq!(
            plan_reconciliation(&[41, 42], &[remote]),
            ReconcilePlan::Unchanged { stack_number: 7 }
        );
    }

    #[test]
    fn plans_append_with_only_the_new_top_delta() {
        let remote = RemoteStackSnapshot {
            number: 7,
            entries: vec![entry(41, RemotePullRequestState::Open)],
        };
        assert_eq!(
            plan_reconciliation(&[41, 42, 43], &[remote]),
            ReconcilePlan::Append {
                stack_number: 7,
                delta: vec![42, 43],
            }
        );
    }

    #[test]
    fn rejects_reordered_active_prs() {
        let remote = RemoteStackSnapshot {
            number: 7,
            entries: vec![
                entry(41, RemotePullRequestState::Open),
                entry(42, RemotePullRequestState::Open),
            ],
        };
        assert!(matches!(
            plan_reconciliation(&[42, 41], &[remote]),
            ReconcilePlan::Diverged { .. }
        ));
    }

    #[test]
    fn rejects_closed_unmerged_remote_entry() {
        let remote = RemoteStackSnapshot {
            number: 7,
            entries: vec![
                entry(41, RemotePullRequestState::Open),
                entry(42, RemotePullRequestState::Closed),
            ],
        };
        assert!(matches!(
            plan_reconciliation(&[41, 43], &[remote]),
            ReconcilePlan::Diverged { .. }
        ));
    }

    #[test]
    fn rejects_membership_in_multiple_native_stacks() {
        let first = RemoteStackSnapshot {
            number: 7,
            entries: vec![entry(41, RemotePullRequestState::Open)],
        };
        let second = RemoteStackSnapshot {
            number: 8,
            entries: vec![entry(42, RemotePullRequestState::Open)],
        };
        assert!(matches!(
            plan_reconciliation(&[41, 42], &[first, second]),
            ReconcilePlan::Diverged { .. }
        ));
    }

    #[test]
    fn rejects_local_removal_from_active_suffix() {
        let remote = RemoteStackSnapshot {
            number: 7,
            entries: vec![
                entry(41, RemotePullRequestState::Open),
                entry(42, RemotePullRequestState::Draft),
            ],
        };
        assert!(matches!(
            plan_reconciliation(&[41], &[remote]),
            ReconcilePlan::Diverged { .. }
        ));
    }

    #[test]
    fn rejects_merged_entry_after_an_active_entry() {
        let remote = RemoteStackSnapshot {
            number: 7,
            entries: vec![
                entry(41, RemotePullRequestState::Open),
                entry(42, RemotePullRequestState::Merged),
            ],
        };
        assert!(matches!(
            plan_reconciliation(&[41, 42], &[remote]),
            ReconcilePlan::Diverged { .. }
        ));
    }

    #[test]
    fn plans_create_when_only_unrelated_prior_stack_is_fully_merged() {
        let remote = RemoteStackSnapshot {
            number: 7,
            entries: vec![
                entry(40, RemotePullRequestState::Merged),
                entry(41, RemotePullRequestState::Merged),
            ],
        };
        assert_eq!(
            plan_reconciliation(&[42, 43], &[remote]),
            ReconcilePlan::Create
        );
    }

    #[test]
    fn skipped_result_normalizes_skip_state() {
        assert_eq!(
            GithubStackSyncResult::skipped(
                GithubStacksIntegration::Auto,
                GithubStackReason::Disabled,
                vec![41, 42],
            ),
            GithubStackSyncResult {
                mode: GithubStacksIntegration::Auto,
                action: GithubStackAction::Skipped,
                reason: Some(GithubStackReason::Disabled),
                stack_number: None,
                pr_numbers: vec![41, 42],
                message: None,
            }
        );
    }

    #[test]
    fn warning_result_normalizes_warning_state() {
        let result = GithubStackSyncResult::warning(
            GithubStacksIntegration::Force,
            GithubStackReason::BackendFailed,
            vec![41, 42],
            "GitHub extension failed".to_string(),
        );
        assert!(result.is_warning());
        assert_eq!(result.action, GithubStackAction::Warning);
        assert_eq!(result.reason, Some(GithubStackReason::BackendFailed));
        assert_eq!(result.message.as_deref(), Some("GitHub extension failed"));
    }

    #[test]
    fn accepts_minimum_gh_stack_version() {
        assert_eq!(
            parse_gh_stack_version("gh stack version 0.1.0\n").unwrap(),
            semver::Version::new(0, 1, 0)
        );
    }

    #[test]
    fn rejects_outdated_gh_stack_version() {
        let runner = FakeRunner::new([response(
            &["stack", "--version"],
            true,
            "gh stack version 0.0.8\n",
            "",
        )]);
        let outcome =
            reconcile_with_runner(&runner, GithubStacksIntegration::Auto, "main", &[41, 42]);
        assert_eq!(outcome.result.action, GithubStackAction::Skipped);
        assert_eq!(
            outcome.result.reason,
            Some(GithubStackReason::OutdatedExtension)
        );
        assert!(!outcome.mutation_attempted);
        runner.assert_exhausted();
    }

    #[test]
    fn force_warns_when_gh_stack_version_is_outdated() {
        let runner = FakeRunner::new([response(
            &["stack", "--version"],
            true,
            "gh stack version 0.0.8\n",
            "",
        )]);
        let outcome =
            reconcile_with_runner(&runner, GithubStacksIntegration::Force, "main", &[41, 42]);
        assert_eq!(outcome.result.action, GithubStackAction::Warning);
        assert_eq!(
            outcome.result.reason,
            Some(GithubStackReason::OutdatedExtension)
        );
        assert!(!outcome.mutation_attempted);
    }

    #[test]
    fn force_warns_when_gh_stack_extension_is_missing() {
        let runner = FakeRunner::new([response(
            &["stack", "--version"],
            false,
            "",
            "unknown command \"stack\"",
        )]);
        let outcome =
            reconcile_with_runner(&runner, GithubStacksIntegration::Force, "main", &[41, 42]);
        assert_eq!(outcome.result.action, GithubStackAction::Warning);
        assert_eq!(
            outcome.result.reason,
            Some(GithubStackReason::MissingExtension)
        );
        assert!(!outcome.mutation_attempted);
        runner.assert_exhausted();
    }

    #[test]
    fn empty_pr_numbers_skip_without_running_gh() {
        let runner = FakeRunner::new([]);
        let outcome = reconcile_with_runner(&runner, GithubStacksIntegration::Auto, "main", &[]);
        assert_eq!(outcome.result.action, GithubStackAction::Skipped);
        assert_eq!(
            outcome.result.reason,
            Some(GithubStackReason::InsufficientPrs)
        );
        assert!(!outcome.mutation_attempted);
        runner.assert_exhausted();
    }

    #[test]
    fn parses_merged_prefix_and_open_entry() {
        let stacks = parse_stack_list(STACK_7).unwrap();
        assert_eq!(stacks[0].entries[0].state, RemotePullRequestState::Merged);
        assert_eq!(stacks[0].entries[1].state, RemotePullRequestState::Open);
    }

    #[test]
    fn parses_draft_and_closed_entries() {
        let stacks = parse_stack_list(
            r#"[{"number":7,"pull_requests":[
                {"number":41,"state":"open","draft":true,"merged_at":null},
                {"number":42,"state":"closed","draft":false,"merged_at":null}
            ]}]"#,
        )
        .unwrap();
        assert_eq!(stacks[0].entries[0].state, RemotePullRequestState::Draft);
        assert_eq!(stacks[0].entries[1].state, RemotePullRequestState::Closed);
    }

    #[test]
    fn malformed_stack_api_response_warns_backend_failed() {
        let runner = FakeRunner::new([
            response(
                &["stack", "--version"],
                true,
                "gh stack version 0.1.0\n",
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                true,
                "not json",
                "",
            ),
        ]);
        let outcome = reconcile_with_runner(&runner, GithubStacksIntegration::Auto, "main", &[41]);
        assert_eq!(outcome.result.action, GithubStackAction::Warning);
        assert_eq!(
            outcome.result.reason,
            Some(GithubStackReason::BackendFailed)
        );
        assert!(!outcome.mutation_attempted);
    }

    #[test]
    fn stack_api_404_marks_repository_unsupported() {
        let runner = FakeRunner::new([
            response(
                &["stack", "--version"],
                true,
                "gh stack version 0.1.0\n",
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                false,
                "",
                "gh: HTTP 404: Not Found",
            ),
        ]);
        let outcome = reconcile_with_runner(&runner, GithubStacksIntegration::Auto, "main", &[41]);
        assert_eq!(outcome.result.action, GithubStackAction::Skipped);
        assert_eq!(
            outcome.result.reason,
            Some(GithubStackReason::UnsupportedRepository)
        );
    }

    #[test]
    fn create_uses_base_and_numeric_prs_only() {
        let runner = FakeRunner::new([
            response(
                &["stack", "--version"],
                true,
                "gh stack version 0.1.0\n",
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                true,
                "[]",
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=42"],
                true,
                "[]",
                "",
            ),
            response(
                &["stack", "link", "--base", "main", "41", "42"],
                true,
                "Created stack\n",
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                true,
                STACK_7_CREATED,
                "",
            ),
        ]);
        let outcome =
            reconcile_with_runner(&runner, GithubStacksIntegration::Auto, "main", &[41, 42]);
        assert_eq!(outcome.result.action, GithubStackAction::Created);
        assert_eq!(outcome.result.stack_number, Some(7));
        assert!(outcome.mutation_attempted);
        runner.assert_exhausted();
    }

    #[test]
    fn create_runs_pre_mutation_callback_before_stack_link() {
        let callback_seen = Rc::new(Cell::new(false));
        let runner = CallbackOrderRunner {
            inner: FakeRunner::new([
                response(
                    &["stack", "--version"],
                    true,
                    "gh stack version 0.1.0\n",
                    "",
                ),
                response(
                    &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                    true,
                    "[]",
                    "",
                ),
                response(
                    &["api", "repos/{owner}/{repo}/stacks?pull_request=42"],
                    true,
                    "[]",
                    "",
                ),
                response(
                    &["stack", "link", "--base", "main", "41", "42"],
                    true,
                    "",
                    "",
                ),
                response(
                    &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                    true,
                    STACK_7_CREATED,
                    "",
                ),
            ]),
            callback_seen: Rc::clone(&callback_seen),
        };

        let outcome = reconcile_with_runner_before_mutation(
            &runner,
            GithubStacksIntegration::Auto,
            "main",
            &[41, 42],
            || callback_seen.set(true),
        );

        assert_eq!(outcome.result.action, GithubStackAction::Created);
        assert!(outcome.mutation_attempted);
        assert!(callback_seen.get());
        runner.inner.assert_exhausted();
    }

    #[test]
    fn append_uses_stack_number_and_delta_only() {
        let runner = FakeRunner::new([
            response(
                &["stack", "--version"],
                true,
                "gh stack version 0.1.0\n",
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                true,
                STACK_7_ACTIVE,
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=42"],
                true,
                "[]",
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=43"],
                true,
                "[]",
                "",
            ),
            response(&["stack", "link", "7", "42", "43"], true, "", ""),
        ]);
        let outcome = reconcile_with_runner(
            &runner,
            GithubStacksIntegration::Auto,
            "main",
            &[41, 42, 43],
        );
        assert_eq!(outcome.result.action, GithubStackAction::Appended);
        assert_eq!(outcome.result.stack_number, Some(7));
        assert!(outcome.mutation_attempted);
        runner.assert_exhausted();
    }

    #[test]
    fn unchanged_does_not_invoke_stack_link() {
        let runner = FakeRunner::new([
            response(
                &["stack", "--version"],
                true,
                "gh stack version 0.1.0\n",
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                true,
                STACK_7_CREATED,
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=42"],
                true,
                STACK_7_CREATED,
                "",
            ),
        ]);
        let outcome =
            reconcile_with_runner(&runner, GithubStacksIntegration::Auto, "main", &[41, 42]);
        assert_eq!(outcome.result.action, GithubStackAction::Unchanged);
        assert_eq!(outcome.result.stack_number, Some(7));
        assert!(!outcome.mutation_attempted);
        runner.assert_exhausted();
    }

    #[test]
    fn unchanged_does_not_run_pre_mutation_callback() {
        let callback_seen = Rc::new(Cell::new(false));
        let runner = FakeRunner::new([
            response(
                &["stack", "--version"],
                true,
                "gh stack version 0.1.0\n",
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                true,
                STACK_7_CREATED,
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=42"],
                true,
                STACK_7_CREATED,
                "",
            ),
        ]);

        let outcome = reconcile_with_runner_before_mutation(
            &runner,
            GithubStacksIntegration::Auto,
            "main",
            &[41, 42],
            || callback_seen.set(true),
        );

        assert_eq!(outcome.result.action, GithubStackAction::Unchanged);
        assert!(!outcome.mutation_attempted);
        assert!(!callback_seen.get());
        runner.assert_exhausted();
    }

    #[test]
    fn link_failure_warns_and_records_mutation_attempt() {
        let runner = FakeRunner::new([
            response(
                &["stack", "--version"],
                true,
                "gh stack version 0.1.0\n",
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                true,
                "[]",
                "",
            ),
            response(
                &["stack", "link", "--base", "main", "41"],
                false,
                "",
                "link failed",
            ),
        ]);
        let outcome = reconcile_with_runner(&runner, GithubStacksIntegration::Auto, "main", &[41]);
        assert_eq!(outcome.result.action, GithubStackAction::Warning);
        assert_eq!(
            outcome.result.reason,
            Some(GithubStackReason::BackendFailed)
        );
        assert!(outcome.mutation_attempted);
        assert_eq!(outcome.result.message.as_deref(), Some("link failed"));
    }

    #[test]
    fn create_confirmation_failure_preserves_created_result_with_diagnostic() {
        let runner = FakeRunner::new([
            response(
                &["stack", "--version"],
                true,
                "gh stack version 0.1.0\n",
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                true,
                "[]",
                "",
            ),
            response(&["stack", "link", "--base", "main", "41"], true, "", ""),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                false,
                "",
                "read failed",
            ),
        ]);
        let outcome = reconcile_with_runner(&runner, GithubStacksIntegration::Auto, "main", &[41]);
        assert_eq!(outcome.result.action, GithubStackAction::Created);
        assert_eq!(outcome.result.stack_number, None);
        assert!(outcome.mutation_attempted);
        assert_eq!(outcome.result.message.as_deref(), Some("read failed"));
    }

    #[test]
    fn create_confirmation_without_stack_records_diagnostic() {
        let runner = FakeRunner::new([
            response(
                &["stack", "--version"],
                true,
                "gh stack version 0.1.0\n",
                "",
            ),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                true,
                "[]",
                "",
            ),
            response(&["stack", "link", "--base", "main", "41"], true, "", ""),
            response(
                &["api", "repos/{owner}/{repo}/stacks?pull_request=41"],
                true,
                "[]",
                "",
            ),
        ]);
        let outcome = reconcile_with_runner(&runner, GithubStacksIntegration::Auto, "main", &[41]);
        assert_eq!(outcome.result.action, GithubStackAction::Created);
        assert_eq!(outcome.result.stack_number, None);
        assert!(outcome.mutation_attempted);
        assert_eq!(
            outcome.result.message.as_deref(),
            Some("Created native stack could not be confirmed")
        );
        runner.assert_exhausted();
    }
}
