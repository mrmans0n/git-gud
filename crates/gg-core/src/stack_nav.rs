//! Stack navigation comment rendering.
//!
//! Renders the body of the managed comment that `gg sync` posts on each
//! open PR/MR in a multi-entry stack. Pure — no I/O, no provider calls.

/// Hidden HTML comment used to identify git-gud-managed nav comments.
/// Present at the end of every comment body rendered by `render`.
pub(crate) const MARKER: &str = "<!-- gg:stack-nav -->";

/// A single entry in the rendered navigation list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackNavEntry {
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
pub fn render(stack_name: &str, entries: &[StackNavEntry], number_prefix: &str) -> String {
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
}
