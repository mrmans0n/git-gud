//! Stack navigation comment rendering.
//!
//! Renders the body of the managed comment that `gg sync` posts on each
//! open PR/MR in a multi-entry stack. Pure — no I/O, no provider calls.

/// Hidden HTML comment used to identify git-gud-managed nav comments.
/// Present at the end of every comment body rendered by `render`.
pub const MARKER: &str = "<!-- gg:stack-nav -->";

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
    let mut out = String::new();
    out.push_str(&format!(
        "This change is part of the `{}` stack:\n\n",
        stack_name
    ));
    for entry in entries {
        if entry.is_current {
            out.push_str(&format!("- 👉 {}{}\n", number_prefix, entry.pr_number));
        } else {
            out.push_str(&format!("- {}{}\n", number_prefix, entry.pr_number));
        }
    }
    out.push_str("\n<sub>Managed by [git-gud](https://github.com/mrmans0n/git-gud).</sub>\n");
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
}
