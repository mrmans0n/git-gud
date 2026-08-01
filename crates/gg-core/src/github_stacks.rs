use serde::Serialize;

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

    if stack.entries[..active_start]
        .iter()
        .any(|entry| entry.state != RemotePullRequestState::Merged)
    {
        return diverged("Native stack has an invalid merged prefix");
    }

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
