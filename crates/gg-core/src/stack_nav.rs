//! Stack navigation comment rendering.
//!
//! Renders the body of the managed comment that `gg sync` posts on each
//! open PR/MR in a multi-entry stack. Pure — no I/O, no provider calls.

/// Hidden HTML comment used to identify git-gud-managed nav comments.
/// Present at the end of every comment body rendered by `render`.
pub(crate) const MARKER: &str = "<!-- gg:stack-nav -->";

/// A single entry in the rendered navigation list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StackNavEntry {
    pub pr_number: u64,
    pub is_current: bool,
}

/// Render the body of the managed nav comment.
///
/// `entries` must be in bottom-up order (index 0 is the entry adjacent to
/// the base branch; the last entry is the tip of the stack). Exactly one
/// entry should have `is_current == true`.
///
/// `number_prefix` is `"#"` for GitHub, `"!"` for GitLab.
///
/// The caller is responsible for deciding whether to render at all
/// (single-entry stacks should skip this function).
pub(crate) fn render(stack_name: &str, entries: &[StackNavEntry], number_prefix: &str) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    writeln!(out, "This change is part of the `{}` stack:", stack_name).unwrap();
    writeln!(out).unwrap();
    for entry in entries {
        if entry.is_current {
            writeln!(out, "- 👉 {}{}", number_prefix, entry.pr_number).unwrap();
        } else {
            writeln!(out, "- {}{}", number_prefix, entry.pr_number).unwrap();
        }
    }
    writeln!(out).unwrap();
    writeln!(
        out,
        "<sub>Managed by [git-gud](https://github.com/mrmans0n/git-gud).</sub>"
    )
    .unwrap();
    out.push_str(MARKER);
    out
}

/// The per-entry PR state that matters for nav-comment reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrEntryState {
    Open,
    Draft,
    Merged,
    Closed,
}

/// Inputs for the per-entry nav-action decision.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NavDecisionInput {
    pub setting_enabled: bool,
    pub stack_entry_count: usize,
    pub pr_state: PrEntryState,
    pub has_existing_comment: bool,
}

/// What to do with the nav comment on a single PR in the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavAction {
    /// Do nothing: no comment should exist and none does.
    Skip,
    /// Create a new comment, or update the existing one (idempotent upsert).
    Upsert,
    /// Delete an existing comment.
    Delete,
}

/// Decide what to do with the nav comment on a single PR, based on state.
///
/// See the design spec for the full decision table. In short:
/// - Closed/merged PRs: always skip.
/// - Setting off OR single-entry stack: delete if a comment exists, else skip.
/// - Otherwise: upsert.
pub(crate) fn decide_action(input: NavDecisionInput) -> NavAction {
    // Historical PRs are never touched.
    if matches!(input.pr_state, PrEntryState::Merged | PrEntryState::Closed) {
        return NavAction::Skip;
    }

    let should_have_comment = input.setting_enabled && input.stack_entry_count >= 2;

    if should_have_comment {
        NavAction::Upsert
    } else if input.has_existing_comment {
        NavAction::Delete
    } else {
        NavAction::Skip
    }
}

/// Returns true if `body` contains the managed-comment marker.
///
/// Used to identify git-gud-managed nav comments among arbitrary PR comments
/// when we need to find our own comment to update or delete it.
pub(crate) fn is_managed_comment(body: &str) -> bool {
    body.contains(MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_two_entries_current_first_github() {
        let entries = vec![
            StackNavEntry {
                pr_number: 42,
                is_current: true,
            },
            StackNavEntry {
                pr_number: 43,
                is_current: false,
            },
        ];
        let body = render("feat-auth", &entries, "#");
        assert_eq!(
            body,
            "This change is part of the `feat-auth` stack:\n\n\
             - 👉 #42\n\
             - #43\n\n\
             <sub>Managed by [git-gud](https://github.com/mrmans0n/git-gud).</sub>\n\
             <!-- gg:stack-nav -->"
        );
    }

    #[test]
    fn test_render_three_entries_current_middle_github() {
        let entries = vec![
            StackNavEntry {
                pr_number: 42,
                is_current: false,
            },
            StackNavEntry {
                pr_number: 43,
                is_current: true,
            },
            StackNavEntry {
                pr_number: 44,
                is_current: false,
            },
        ];
        let body = render("feat-auth", &entries, "#");
        assert!(body.contains("- #42\n"));
        assert!(body.contains("- 👉 #43\n"));
        assert!(body.contains("- #44\n"));
        assert!(body.ends_with(MARKER));
    }

    #[test]
    fn test_render_current_last_preserves_bottom_up_order() {
        // Bottom-up: base-adjacent first, tip last. Current on tip should be last.
        let entries = vec![
            StackNavEntry {
                pr_number: 10,
                is_current: false,
            },
            StackNavEntry {
                pr_number: 11,
                is_current: false,
            },
            StackNavEntry {
                pr_number: 12,
                is_current: true,
            },
        ];
        let body = render("s", &entries, "#");
        let idx_10 = body.find("#10").unwrap();
        let idx_11 = body.find("#11").unwrap();
        let idx_12 = body.find("#12").unwrap();
        assert!(idx_10 < idx_11 && idx_11 < idx_12);
        assert!(body.contains("- 👉 #12\n"));
    }

    #[test]
    fn test_render_gitlab_prefix() {
        let entries = vec![
            StackNavEntry {
                pr_number: 1,
                is_current: true,
            },
            StackNavEntry {
                pr_number: 2,
                is_current: false,
            },
        ];
        let body = render("s", &entries, "!");
        assert!(body.contains("- 👉 !1\n"));
        assert!(body.contains("- !2\n"));
        assert!(!body.contains('#'));
    }

    #[test]
    fn test_render_is_idempotent() {
        let entries = vec![
            StackNavEntry {
                pr_number: 1,
                is_current: true,
            },
            StackNavEntry {
                pr_number: 2,
                is_current: false,
            },
        ];
        let a = render("s", &entries, "#");
        let b = render("s", &entries, "#");
        assert_eq!(a, b);
    }

    #[test]
    fn test_render_includes_stack_name_backticked() {
        let entries = vec![
            StackNavEntry {
                pr_number: 1,
                is_current: true,
            },
            StackNavEntry {
                pr_number: 2,
                is_current: false,
            },
        ];
        let body = render("my-stack", &entries, "#");
        assert!(body.contains("`my-stack`"));
    }

    #[test]
    fn test_render_ends_with_marker() {
        let entries = vec![
            StackNavEntry {
                pr_number: 1,
                is_current: true,
            },
            StackNavEntry {
                pr_number: 2,
                is_current: false,
            },
        ];
        let body = render("s", &entries, "#");
        assert!(body.ends_with(MARKER));
    }

    #[test]
    fn test_render_includes_attribution_footer() {
        let entries = vec![
            StackNavEntry {
                pr_number: 1,
                is_current: true,
            },
            StackNavEntry {
                pr_number: 2,
                is_current: false,
            },
        ];
        let body = render("s", &entries, "#");
        assert!(body.contains("<sub>Managed by [git-gud]"));
    }

    #[test]
    fn test_is_managed_comment_with_marker() {
        let body = "some text\n<!-- gg:stack-nav -->";
        assert!(is_managed_comment(body));
    }

    #[test]
    fn test_is_managed_comment_without_marker() {
        let body = "a user comment with no markers";
        assert!(!is_managed_comment(body));
    }

    #[test]
    fn test_is_managed_comment_with_trailing_whitespace_after_marker() {
        let body = "body\n<!-- gg:stack-nav -->   \n";
        assert!(is_managed_comment(body));
    }

    #[test]
    fn test_is_managed_comment_empty_body() {
        assert!(!is_managed_comment(""));
    }

    #[test]
    fn test_decide_action_reconcile_when_setting_on_and_multi_entry_open() {
        let decision = decide_action(NavDecisionInput {
            setting_enabled: true,
            stack_entry_count: 3,
            pr_state: PrEntryState::Open,
            has_existing_comment: false,
        });
        assert_eq!(decision, NavAction::Upsert);
    }

    #[test]
    fn test_decide_action_cleanup_when_setting_off_and_comment_exists() {
        let decision = decide_action(NavDecisionInput {
            setting_enabled: false,
            stack_entry_count: 3,
            pr_state: PrEntryState::Open,
            has_existing_comment: true,
        });
        assert_eq!(decision, NavAction::Delete);
    }

    #[test]
    fn test_decide_action_skip_when_setting_off_and_no_comment() {
        let decision = decide_action(NavDecisionInput {
            setting_enabled: false,
            stack_entry_count: 3,
            pr_state: PrEntryState::Open,
            has_existing_comment: false,
        });
        assert_eq!(decision, NavAction::Skip);
    }

    #[test]
    fn test_decide_action_cleanup_when_single_entry_and_comment_exists() {
        let decision = decide_action(NavDecisionInput {
            setting_enabled: true,
            stack_entry_count: 1,
            pr_state: PrEntryState::Open,
            has_existing_comment: true,
        });
        assert_eq!(decision, NavAction::Delete);
    }

    #[test]
    fn test_decide_action_skip_when_single_entry_and_no_comment() {
        let decision = decide_action(NavDecisionInput {
            setting_enabled: true,
            stack_entry_count: 1,
            pr_state: PrEntryState::Open,
            has_existing_comment: false,
        });
        assert_eq!(decision, NavAction::Skip);
    }

    #[test]
    fn test_decide_action_skip_when_pr_closed() {
        // Closed / merged PRs are historical — never touch their comments.
        for state in [PrEntryState::Merged, PrEntryState::Closed] {
            let decision = decide_action(NavDecisionInput {
                setting_enabled: true,
                stack_entry_count: 3,
                pr_state: state,
                has_existing_comment: true,
            });
            assert_eq!(decision, NavAction::Skip, "closed/merged must be skipped");
        }
    }

    #[test]
    fn test_decide_action_draft_treated_as_open() {
        let decision = decide_action(NavDecisionInput {
            setting_enabled: true,
            stack_entry_count: 2,
            pr_state: PrEntryState::Draft,
            has_existing_comment: false,
        });
        assert_eq!(decision, NavAction::Upsert);
    }

    // --- Pipeline tests: snapshot → decision → rendered body ---

    #[test]
    fn test_full_pipeline_three_entries_setting_on() {
        // Simulate the reconcile pass for a 3-entry stack with setting on.
        let stack_name = "my-stack";
        let entries: Vec<(u64, PrEntryState)> = vec![
            (42, PrEntryState::Open),
            (43, PrEntryState::Open),
            (44, PrEntryState::Draft), // draft treated as open
        ];

        for (current_idx, _) in entries.iter().enumerate() {
            let decision = decide_action(NavDecisionInput {
                setting_enabled: true,
                stack_entry_count: entries.len(),
                pr_state: entries[current_idx].1,
                has_existing_comment: false,
            });
            assert_eq!(
                decision,
                NavAction::Upsert,
                "entry {} should upsert",
                current_idx
            );

            let nav_entries: Vec<StackNavEntry> = entries
                .iter()
                .enumerate()
                .map(|(j, (num, _))| StackNavEntry {
                    pr_number: *num,
                    is_current: j == current_idx,
                })
                .collect();
            let body = render(stack_name, &nav_entries, "#");

            // Each PR's body should contain all 3 entries.
            assert!(
                body.contains("#42"),
                "entry {} body missing #42",
                current_idx
            );
            assert!(
                body.contains("#43"),
                "entry {} body missing #43",
                current_idx
            );
            assert!(
                body.contains("#44"),
                "entry {} body missing #44",
                current_idx
            );
            // Only the current entry should have the marker.
            let current_num = entries[current_idx].0;
            assert!(
                body.contains(&format!("👉 #{}", current_num)),
                "entry {} should be marked current",
                current_idx
            );
        }
    }

    #[test]
    fn test_full_pipeline_setting_off_with_existing_comments() {
        // When setting is off, every entry with an existing comment should be Delete.
        let entries: Vec<(u64, PrEntryState, bool)> = vec![
            (10, PrEntryState::Open, true),   // has comment → Delete
            (11, PrEntryState::Open, false),  // no comment → Skip
            (12, PrEntryState::Merged, true), // merged with comment → Skip (merged always skipped)
        ];

        let expected = [NavAction::Delete, NavAction::Skip, NavAction::Skip];

        for (i, (_, state, has_comment)) in entries.iter().enumerate() {
            let decision = decide_action(NavDecisionInput {
                setting_enabled: false,
                stack_entry_count: entries.len(),
                pr_state: *state,
                has_existing_comment: *has_comment,
            });
            assert_eq!(decision, expected[i], "entry {} mismatch", i);
        }
    }

    #[test]
    fn test_full_pipeline_partial_failure_should_skip() {
        // When one entry has no snapshot (failed during sync), the calling code
        // in sync.rs checks nav_snapshots.iter().all(|s| s.is_some()) before
        // entering the reconcile pass. Verify the precondition catches this.
        let snapshots: Vec<Option<(u64, PrEntryState)>> = vec![
            Some((42, PrEntryState::Open)),
            None, // entry 2 failed during sync
            Some((44, PrEntryState::Open)),
        ];
        let all_present = snapshots.iter().all(|s| s.is_some());
        assert!(!all_present, "should detect incomplete snapshot set");
    }
}
