//! Immutability policy for history-rewriting commands.
//!
//! Some commits in a stack should not be rewritten casually:
//!
//! 1. Commits whose PR/MR is already merged — rewriting them produces a local
//!    duplicate of work that is already published, and can confuse later
//!    `gg sync` / `gg rebase` flows. This is the *only* rule that catches
//!    **squash-merged** PRs, because their merge commit on `origin/<base>`
//!    has a brand-new SHA that doesn't share ancestry with the local commit.
//! 2. Commits already reachable from `origin/<base>` — caught via git ancestry
//!    rather than provider state. Handles plain merges and rebases.
//!
//! This module centralises the policy so that every rewrite-style command
//! (squash, drop, reorder, split, absorb, rebase) applies the same check.
//! Users who genuinely need to rewrite immutable history can override with
//! `--force` / `--ignore-immutable`.
//!
//! Design choices:
//! - The policy is **pre-flight**: commands build a report, call [`guard`],
//!   and only proceed if the report is clear (or the user passed `force`).
//!   Nothing is mutated on the repository before the check runs.
//! - Base-ancestor lookups prefer `origin/<base>` because the local base may
//!   be stale; we fall back to the local base if no remote ref exists.
//! - Results are cached implicitly by resolving the base OID once when the
//!   policy is constructed from a stack.
//! - Because `Stack::load` does not refresh provider state, the merged-PR
//!   rule would otherwise rarely fire. [`refresh_mr_state_for_guard`] is a
//!   best-effort pre-flight refresh that callers run once per command after
//!   `Stack::load` to populate `mr_state` for the guard.
//!
//! See `docs/src/core-concepts.md` for the user-facing explanation.

use console::style;
use git2::Repository;

use crate::error::{GgError, Result};
use crate::provider::{PrState, Provider};
use crate::stack::{Stack, StackEntry};

/// Why a specific commit is considered immutable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImmutableReason {
    /// The commit's PR/MR is already merged.
    MergedPr {
        /// PR/MR number, if known.
        number: Option<u64>,
    },
    /// The commit is already reachable from the (remote) base branch.
    BaseAncestor {
        /// The ref we compared against, e.g. `origin/main` or `main`.
        base_ref: String,
    },
}

impl ImmutableReason {
    /// Human-readable short description used in error messages.
    pub fn describe(&self) -> String {
        match self {
            ImmutableReason::MergedPr { number: Some(n) } => format!("merged as !{}", n),
            ImmutableReason::MergedPr { number: None } => "PR/MR is merged".to_string(),
            ImmutableReason::BaseAncestor { base_ref } => {
                format!("already in {}", base_ref)
            }
        }
    }
}

/// An immutable entry and all the reasons it matched.
#[derive(Debug, Clone)]
pub struct ImmutableEntry {
    /// 1-indexed position in the stack.
    pub position: usize,
    /// Short SHA for display.
    pub short_sha: String,
    /// Commit title.
    pub title: String,
    /// Reasons the entry is immutable (non-empty).
    pub reasons: Vec<ImmutableReason>,
}

/// Report of all immutable entries among a set of rewrite targets.
#[derive(Debug, Clone, Default)]
pub struct ImmutabilityReport {
    /// Immutable entries that were checked (in ascending position order).
    pub entries: Vec<ImmutableEntry>,
}

impl ImmutabilityReport {
    /// True if no entries are immutable.
    pub fn is_clear(&self) -> bool {
        self.entries.is_empty()
    }

    /// Remove entries that have a `BaseAncestor` reason.
    ///
    /// If a commit is reachable from `origin/<base>`, `git rebase` skips it
    /// unconditionally (patch-already-applied detection is SHA-based). So
    /// base-ancestor entries never need guarding during rebase — even if the
    /// entry also carries a `MergedPr` reason, the commit won't be rewritten.
    ///
    /// Entries with only non-`BaseAncestor` reasons (e.g. squash-merged PRs
    /// whose SHA isn't on the base) are kept — rebase *would* reapply those.
    pub fn without_base_ancestors(self) -> Self {
        Self {
            entries: self
                .entries
                .into_iter()
                .filter(|e| {
                    !e.reasons
                        .iter()
                        .any(|r| matches!(r, ImmutableReason::BaseAncestor { .. }))
                })
                .collect(),
        }
    }

    /// Format the report as a multi-line string suitable for error messages.
    pub fn format_for_error(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            let reasons: Vec<String> = entry.reasons.iter().map(|r| r.describe()).collect();
            out.push_str(&format!(
                "  #{pos}  {sha}  {title}  ({reasons})\n",
                pos = entry.position,
                sha = entry.short_sha,
                title = entry.title,
                reasons = reasons.join(", "),
            ));
        }
        // Trim trailing newline for nicer embedding in error strings.
        while out.ends_with('\n') {
            out.pop();
        }
        out
    }
}

/// Policy object bundled with the data it needs to answer "is this commit
/// immutable?" for a given stack.
pub struct ImmutabilityPolicy<'a> {
    repo: &'a Repository,
    /// The ref we compared against (e.g. `origin/main` or `main`).
    base_ref: String,
    /// OID of `base_ref`, used for ancestor checks.
    base_oid: Option<git2::Oid>,
}

impl<'a> ImmutabilityPolicy<'a> {
    /// Construct a policy for the given stack, resolving the remote base ref
    /// (falling back to the local base if the remote ref is not available).
    pub fn for_stack(repo: &'a Repository, stack: &Stack) -> Result<Self> {
        let remote_ref = format!("origin/{}", stack.base);
        let (base_ref, base_oid) =
            if let Ok(obj) = repo.revparse_single(&format!("refs/remotes/{}", remote_ref)) {
                (remote_ref, Some(obj.id()))
            } else if let Ok(obj) = repo.revparse_single(&stack.base) {
                (stack.base.clone(), Some(obj.id()))
            } else {
                // No base ref found — base-ancestor rule simply won't fire.
                (stack.base.clone(), None)
            };

        Ok(Self {
            repo,
            base_ref,
            base_oid,
        })
    }

    /// The ref name used for base-ancestor comparisons.
    pub fn base_ref(&self) -> &str {
        &self.base_ref
    }

    /// Compute the reasons this entry is immutable. An empty vector means the
    /// entry is mutable.
    pub fn reasons_for(&self, entry: &StackEntry) -> Vec<ImmutableReason> {
        let mut reasons = Vec::new();

        if matches!(entry.mr_state, Some(PrState::Merged)) {
            reasons.push(ImmutableReason::MergedPr {
                number: entry.mr_number,
            });
        }

        if let Some(base_oid) = self.base_oid {
            if entry.oid == base_oid
                || self
                    .repo
                    .graph_descendant_of(base_oid, entry.oid)
                    .unwrap_or(false)
            {
                reasons.push(ImmutableReason::BaseAncestor {
                    base_ref: self.base_ref.clone(),
                });
            }
        }

        reasons
    }

    /// Build a report for a slice of 1-indexed positions from the stack.
    /// Unknown positions are silently skipped (callers validate elsewhere).
    pub fn check_positions(&self, stack: &Stack, positions: &[usize]) -> ImmutabilityReport {
        let mut entries: Vec<ImmutableEntry> = Vec::new();
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();

        let mut sorted_positions: Vec<usize> = positions.to_vec();
        sorted_positions.sort_unstable();
        sorted_positions.dedup();

        for pos in sorted_positions {
            if !seen.insert(pos) {
                continue;
            }
            if let Some(entry) = stack.get_entry_by_position(pos) {
                let reasons = self.reasons_for(entry);
                if !reasons.is_empty() {
                    entries.push(ImmutableEntry {
                        position: entry.position,
                        short_sha: entry.short_sha.clone(),
                        title: entry.title.clone(),
                        reasons,
                    });
                }
            }
        }

        ImmutabilityReport { entries }
    }

    /// Build a report covering every entry in the stack.
    pub fn check_all(&self, stack: &Stack) -> ImmutabilityReport {
        let positions: Vec<usize> = stack.entries.iter().map(|e| e.position).collect();
        self.check_positions(stack, &positions)
    }
}

/// Shared pre-flight guard used by every history-rewriting command.
///
/// If the report has no immutable entries, this is a no-op.
/// If the report has entries and `force` is false, returns
/// [`GgError::ImmutableTargets`] with a formatted list of offending commits.
/// If the report has entries and `force` is true, prints a warning to stderr
/// and proceeds.
pub fn guard(report: ImmutabilityReport, force: bool) -> Result<()> {
    if report.is_clear() {
        return Ok(());
    }

    if force {
        eprintln!(
            "{} overriding immutability check for {} commit(s):",
            style("Warning:").yellow().bold(),
            report.entries.len()
        );
        eprintln!("{}", report.format_for_error());
        return Ok(());
    }

    Err(GgError::ImmutableTargets(report.format_for_error()))
}

/// Best-effort refresh of `mr_state` for every entry that has an `mr_number`,
/// done once per command invocation immediately before the immutability guard
/// runs.
///
/// `Stack::load` does not call any provider, so without this helper the
/// merged-PR rule would essentially never fire — and squash-merged commits
/// (whose merge SHA on `origin/<base>` doesn't share ancestry with the local
/// commit) would slip past the guard entirely. By proactively populating
/// `mr_state` here we close that gap for any user with a working provider.
///
/// **Best-effort by design:**
/// - If no provider can be detected (no remote, missing config), this is a
///   no-op so offline / no-auth users see no behaviour change.
/// - Errors from individual `get_pr_info` calls are silently swallowed; the
///   guard then falls back to the base-ancestor rule for that entry. We do
///   not want a flaky API to block a `gg squash`.
/// - Only `mr_state` is touched. CI status, approval and merge-train info are
///   not needed by the guard, so we skip those calls to keep the latency
///   roughly "1 API call per open PR in the stack".
///
/// Cost: O(entries with `mr_number`) network round-trips, executed serially.
/// For typical stacks (a handful of open PRs) this is well below a second.
pub fn refresh_mr_state_for_guard(repo: &Repository, stack: &mut Stack) {
    let Ok(provider) = Provider::detect(repo) else {
        // No provider configured — offline or non-GitHub/GitLab repo. The
        // base-ancestor rule remains in effect.
        return;
    };

    for entry in &mut stack.entries {
        if let Some(pr_num) = entry.mr_number {
            if let Ok(info) = provider.get_pr_info(pr_num) {
                entry.mr_state = Some(info.state);
            }
            // On error: leave mr_state untouched. A missing/closed PR will
            // simply not trigger the merged-PR rule; base-ancestor still applies.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::StackEntry;

    fn mk_entry(pos: usize, sha: &str, title: &str) -> StackEntry {
        StackEntry {
            oid: git2::Oid::zero(),
            short_sha: sha.to_string(),
            title: title.to_string(),
            gg_id: None,
            gg_parent: None,
            mr_number: None,
            mr_state: None,
            approved: false,
            changes_requested: false,
            mergeable: false,
            ci_status: None,
            position: pos,
            in_merge_train: false,
            merge_train_position: None,
        }
    }

    #[test]
    fn report_is_clear_when_empty() {
        let report = ImmutabilityReport::default();
        assert!(report.is_clear());
    }

    #[test]
    fn report_formats_multiple_reasons_per_entry() {
        let report = ImmutabilityReport {
            entries: vec![ImmutableEntry {
                position: 2,
                short_sha: "abc1234".to_string(),
                title: "Fix typo".to_string(),
                reasons: vec![
                    ImmutableReason::MergedPr { number: Some(123) },
                    ImmutableReason::BaseAncestor {
                        base_ref: "origin/main".to_string(),
                    },
                ],
            }],
        };
        let formatted = report.format_for_error();
        assert!(formatted.contains("#2"));
        assert!(formatted.contains("abc1234"));
        assert!(formatted.contains("Fix typo"));
        assert!(formatted.contains("merged as !123"));
        assert!(formatted.contains("already in origin/main"));
    }

    #[test]
    fn reason_describe_without_number() {
        let r = ImmutableReason::MergedPr { number: None };
        assert_eq!(r.describe(), "PR/MR is merged");
    }

    #[test]
    fn guard_passes_when_report_is_clear() {
        let report = ImmutabilityReport::default();
        assert!(guard(report, false).is_ok());
    }

    #[test]
    fn guard_fails_when_report_has_entries_and_force_is_false() {
        let report = ImmutabilityReport {
            entries: vec![ImmutableEntry {
                position: 1,
                short_sha: "abc".to_string(),
                title: "x".to_string(),
                reasons: vec![ImmutableReason::MergedPr { number: Some(7) }],
            }],
        };
        let err = guard(report, false).expect_err("should fail");
        match err {
            GgError::ImmutableTargets(msg) => {
                assert!(msg.contains("#1"));
                assert!(msg.contains("merged as !7"));
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn guard_proceeds_when_forced_even_if_entries_present() {
        let report = ImmutabilityReport {
            entries: vec![ImmutableEntry {
                position: 1,
                short_sha: "abc".to_string(),
                title: "x".to_string(),
                reasons: vec![ImmutableReason::MergedPr { number: Some(7) }],
            }],
        };
        assert!(guard(report, true).is_ok());
    }

    #[test]
    fn reasons_for_merged_pr_returns_merged_pr_reason() {
        // Use an in-memory bare repo so `graph_descendant_of` can run on OIDs
        // that don't match any real commit (Oid::zero() is unknown).
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let stack = Stack {
            name: "s".to_string(),
            username: "u".to_string(),
            base: "main".to_string(),
            entries: vec![StackEntry {
                mr_number: Some(42),
                mr_state: Some(PrState::Merged),
                ..mk_entry(1, "aaa0000", "merged commit")
            }],
            current_position: Some(0),
        };

        let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();
        let reasons = policy.reasons_for(&stack.entries[0]);
        assert!(
            reasons.contains(&ImmutableReason::MergedPr { number: Some(42) }),
            "expected MergedPr reason, got {:?}",
            reasons
        );
    }

    #[test]
    fn reasons_for_mutable_commit_is_empty() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let stack = Stack {
            name: "s".to_string(),
            username: "u".to_string(),
            base: "nonexistent".to_string(),
            entries: vec![mk_entry(1, "aaa0000", "unpushed")],
            current_position: Some(0),
        };

        let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();
        let reasons = policy.reasons_for(&stack.entries[0]);
        assert!(reasons.is_empty(), "expected no reasons, got {:?}", reasons);
    }

    #[test]
    fn check_positions_collects_multiple_immutables() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let stack = Stack {
            name: "s".to_string(),
            username: "u".to_string(),
            base: "nonexistent".to_string(),
            entries: vec![
                StackEntry {
                    mr_number: Some(1),
                    mr_state: Some(PrState::Merged),
                    ..mk_entry(1, "aaa", "a")
                },
                mk_entry(2, "bbb", "b"),
                StackEntry {
                    mr_number: Some(3),
                    mr_state: Some(PrState::Merged),
                    ..mk_entry(3, "ccc", "c")
                },
            ],
            current_position: Some(2),
        };

        let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();
        let report = policy.check_positions(&stack, &[1, 2, 3]);
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.entries[0].position, 1);
        assert_eq!(report.entries[1].position, 3);
    }

    #[test]
    fn check_positions_dedups_duplicate_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let stack = Stack {
            name: "s".to_string(),
            username: "u".to_string(),
            base: "nonexistent".to_string(),
            entries: vec![StackEntry {
                mr_number: Some(1),
                mr_state: Some(PrState::Merged),
                ..mk_entry(1, "aaa", "a")
            }],
            current_position: Some(0),
        };

        let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();
        let report = policy.check_positions(&stack, &[1, 1, 1]);
        assert_eq!(report.entries.len(), 1);
    }

    #[test]
    fn check_all_covers_every_entry() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let stack = Stack {
            name: "s".to_string(),
            username: "u".to_string(),
            base: "nonexistent".to_string(),
            entries: vec![
                StackEntry {
                    mr_number: Some(1),
                    mr_state: Some(PrState::Merged),
                    ..mk_entry(1, "aaa", "a")
                },
                StackEntry {
                    mr_number: Some(2),
                    mr_state: Some(PrState::Merged),
                    ..mk_entry(2, "bbb", "b")
                },
            ],
            current_position: Some(1),
        };

        let policy = ImmutabilityPolicy::for_stack(&repo, &stack).unwrap();
        let report = policy.check_all(&stack);
        assert_eq!(report.entries.len(), 2);
    }

    #[test]
    fn without_base_ancestors_drops_base_only_entries() {
        let report = ImmutabilityReport {
            entries: vec![ImmutableEntry {
                position: 1,
                short_sha: "aaa".to_string(),
                title: "merged commit".to_string(),
                reasons: vec![ImmutableReason::BaseAncestor {
                    base_ref: "origin/main".to_string(),
                }],
            }],
        };
        let filtered = report.without_base_ancestors();
        assert!(filtered.is_clear());
    }

    #[test]
    fn without_base_ancestors_keeps_merged_pr_entries() {
        let report = ImmutabilityReport {
            entries: vec![ImmutableEntry {
                position: 1,
                short_sha: "aaa".to_string(),
                title: "squash-merged".to_string(),
                reasons: vec![ImmutableReason::MergedPr { number: Some(42) }],
            }],
        };
        let filtered = report.without_base_ancestors();
        assert_eq!(filtered.entries.len(), 1);
    }

    #[test]
    fn without_base_ancestors_drops_dual_reason_entries() {
        // If a commit has both MergedPr and BaseAncestor, it IS on the base,
        // so git rebase will skip it regardless — safe to drop.
        let report = ImmutabilityReport {
            entries: vec![ImmutableEntry {
                position: 1,
                short_sha: "aaa".to_string(),
                title: "merged and on base".to_string(),
                reasons: vec![
                    ImmutableReason::MergedPr { number: Some(10) },
                    ImmutableReason::BaseAncestor {
                        base_ref: "origin/main".to_string(),
                    },
                ],
            }],
        };
        let filtered = report.without_base_ancestors();
        assert!(filtered.is_clear());
    }

    #[test]
    fn without_base_ancestors_mixed_stack() {
        let report = ImmutabilityReport {
            entries: vec![
                // Entry 1: base-ancestor only → should be dropped
                ImmutableEntry {
                    position: 1,
                    short_sha: "aaa".to_string(),
                    title: "on base".to_string(),
                    reasons: vec![ImmutableReason::BaseAncestor {
                        base_ref: "origin/main".to_string(),
                    }],
                },
                // Entry 2: merged-pr only → should be kept
                ImmutableEntry {
                    position: 2,
                    short_sha: "bbb".to_string(),
                    title: "squash-merged".to_string(),
                    reasons: vec![ImmutableReason::MergedPr { number: Some(99) }],
                },
                // Entry 3: both reasons → dropped (BaseAncestor present, rebase skips it)
                ImmutableEntry {
                    position: 3,
                    short_sha: "ccc".to_string(),
                    title: "merged and on base".to_string(),
                    reasons: vec![
                        ImmutableReason::MergedPr { number: Some(100) },
                        ImmutableReason::BaseAncestor {
                            base_ref: "origin/main".to_string(),
                        },
                    ],
                },
            ],
        };
        let filtered = report.without_base_ancestors();
        // Only entry #2 (MergedPr-only, squash-merge) survives.
        assert_eq!(filtered.entries.len(), 1);
        assert_eq!(filtered.entries[0].position, 2);
    }
}
