//! PR/MR description template support
//!
//! Templates are stored in `.git/gg/pr_template.md` and support placeholders:
//! - `{{description}}` - the commit description
//! - `{{stack_name}}` - name of the current stack
//! - `{{commit_sha}}` - short SHA of the commit
//! - `{{title}}` - the PR/MR title
//!
//! Also provides stack breadcrumb rendering for PR/MR descriptions.

use std::fs;
use std::path::Path;

/// Default template filename
const TEMPLATE_FILENAME: &str = "pr_template.md";

/// Context for template rendering
pub struct TemplateContext<'a> {
    pub description: Option<&'a str>,
    pub stack_name: &'a str,
    pub commit_sha: &'a str,
    pub title: &'a str,
}

/// Load the PR template from `.git/gg/pr_template.md` if it exists
pub fn load_template(git_dir: &Path) -> Option<String> {
    let template_path = git_dir.join("gg").join(TEMPLATE_FILENAME);

    if template_path.exists() {
        fs::read_to_string(&template_path).ok()
    } else {
        None
    }
}

/// Render a template with the given context
///
/// Replaces placeholders:
/// - `{{description}}` - commit description (empty string if none)
/// - `{{stack_name}}` - stack name
/// - `{{commit_sha}}` - short commit SHA
/// - `{{title}}` - PR/MR title
pub fn render_template(template: &str, ctx: &TemplateContext) -> String {
    let description = ctx.description.unwrap_or("");

    template
        .replace("{{description}}", description)
        .replace("{{stack_name}}", ctx.stack_name)
        .replace("{{commit_sha}}", ctx.commit_sha)
        .replace("{{title}}", ctx.title)
}

// --- Stack breadcrumb support ---

const BREADCRUMBS_START: &str = "<!-- gg:breadcrumbs:start -->";
const BREADCRUMBS_END: &str = "<!-- gg:breadcrumbs:end -->";

/// Info about one entry in a stack, used for breadcrumb rendering.
pub struct BreadcrumbEntry {
    pub title: String,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
}

/// Render the breadcrumb block for a given entry within its stack.
///
/// - `stack_name`: name of the stack
/// - `entries`: all entries in the stack (ordered bottom→top)
/// - `current_index`: 0-based index of the entry these breadcrumbs are for
/// - `pr_label`: "PR" or "MR"
/// - `pr_prefix`: "#" or "!"
pub fn render_breadcrumbs(
    stack_name: &str,
    entries: &[BreadcrumbEntry],
    current_index: usize,
    pr_label: &str,
    pr_prefix: &str,
) -> String {
    let total = entries.len();
    let position = current_index + 1; // 1-indexed

    let mut lines: Vec<String> = Vec::new();
    lines.push(BREADCRUMBS_START.to_string());
    lines.push(format!(
        "**Stack:** `{}` — {}/{}\n",
        stack_name, position, total
    ));

    // Navigation: previous / next
    let prev = if current_index > 0 {
        format_pr_link(&entries[current_index - 1], pr_label, pr_prefix)
    } else {
        None
    };
    let next = if current_index + 1 < total {
        format_pr_link(&entries[current_index + 1], pr_label, pr_prefix)
    } else {
        None
    };

    let nav = match (prev, next) {
        (Some(p), Some(n)) => format!("{} {} {}", p, "←\u{a0}THIS\u{a0}→", n),
        (Some(p), None) => format!("{} ←\u{a0}THIS (top)", p),
        (None, Some(n)) => format!("THIS (bottom)\u{a0}→ {}", n),
        (None, None) => "THIS (only entry)".to_string(),
    };
    lines.push(nav);
    lines.push(String::new());

    // Compact stack listing
    for (i, entry) in entries.iter().enumerate() {
        let pos = i + 1;
        let marker = if i == current_index { " **⮜**" } else { "" };
        let pr_ref = match (&entry.pr_number, &entry.pr_url) {
            (Some(num), Some(url)) => format!(" — [{}{}]({})", pr_prefix, num, url),
            (Some(num), None) => format!(" — {}{}", pr_prefix, num),
            _ => String::new(),
        };
        lines.push(format!("{}. {}{}{}", pos, entry.title, pr_ref, marker));
    }

    lines.push(BREADCRUMBS_END.to_string());
    lines.join("\n")
}

/// Format a PR/MR link for navigation line.
fn format_pr_link(entry: &BreadcrumbEntry, pr_label: &str, pr_prefix: &str) -> Option<String> {
    match (&entry.pr_number, &entry.pr_url) {
        (Some(num), Some(url)) => Some(format!("[{} {}{}]({})", pr_label, pr_prefix, num, url)),
        (Some(num), None) => Some(format!("{} {}{}", pr_label, pr_prefix, num)),
        _ => None,
    }
}

/// Splice a breadcrumb block into a PR/MR description body.
///
/// If the body already contains the breadcrumb markers, replaces only that region.
/// Otherwise appends the breadcrumb block at the end (separated by a horizontal rule).
/// Returns `(new_body, changed)` where `changed` is false when the block was already identical.
pub fn splice_breadcrumbs(body: &str, breadcrumb_block: &str) -> (String, bool) {
    if let Some((before, after)) = extract_outside_markers(body) {
        let new_body = format!("{}{}{}", before, breadcrumb_block, after);
        let changed = new_body != body;
        (new_body, changed)
    } else {
        // No existing breadcrumb block — append
        let separator = if body.is_empty() || body.ends_with('\n') {
            "\n---\n\n"
        } else {
            "\n\n---\n\n"
        };
        let new_body = format!("{}{}{}\n", body, separator, breadcrumb_block);
        (new_body, true)
    }
}

/// Find content before the start marker and after the end marker.
/// Returns `None` if markers are not found (or malformed).
fn extract_outside_markers(body: &str) -> Option<(&str, &str)> {
    let start_idx = body.find(BREADCRUMBS_START)?;
    let end_marker_start = body.find(BREADCRUMBS_END)?;
    if end_marker_start < start_idx {
        return None;
    }
    let after_end = end_marker_start + BREADCRUMBS_END.len();
    Some((&body[..start_idx], &body[after_end..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // --- Breadcrumb tests ---

    fn sample_entries() -> Vec<BreadcrumbEntry> {
        vec![
            BreadcrumbEntry {
                title: "Add auth module".to_string(),
                pr_number: Some(10),
                pr_url: Some("https://github.com/org/repo/pull/10".to_string()),
            },
            BreadcrumbEntry {
                title: "Add login page".to_string(),
                pr_number: Some(11),
                pr_url: Some("https://github.com/org/repo/pull/11".to_string()),
            },
            BreadcrumbEntry {
                title: "Add logout button".to_string(),
                pr_number: Some(12),
                pr_url: Some("https://github.com/org/repo/pull/12".to_string()),
            },
        ]
    }

    #[test]
    fn test_render_breadcrumbs_middle_entry() {
        let entries = sample_entries();
        let block = render_breadcrumbs("auth-feature", &entries, 1, "PR", "#");

        assert!(block.starts_with(BREADCRUMBS_START));
        assert!(block.ends_with(BREADCRUMBS_END));
        assert!(block.contains("**Stack:** `auth-feature` — 2/3"));
        // Navigation links to adjacent PRs
        assert!(block.contains("[PR #10](https://github.com/org/repo/pull/10)"));
        assert!(block.contains("[PR #12](https://github.com/org/repo/pull/12)"));
        // Current entry marker
        assert!(
            block.contains("2. Add login page — [#11](https://github.com/org/repo/pull/11) **⮜**")
        );
    }

    #[test]
    fn test_render_breadcrumbs_first_entry() {
        let entries = sample_entries();
        let block = render_breadcrumbs("auth-feature", &entries, 0, "PR", "#");
        assert!(block.contains("— 1/3"));
        assert!(block.contains("THIS (bottom)"));
        assert!(block.contains("[PR #11](https://github.com/org/repo/pull/11)"));
    }

    #[test]
    fn test_render_breadcrumbs_last_entry() {
        let entries = sample_entries();
        let block = render_breadcrumbs("auth-feature", &entries, 2, "PR", "#");
        assert!(block.contains("— 3/3"));
        assert!(block.contains("←\u{a0}THIS (top)"));
    }

    #[test]
    fn test_render_breadcrumbs_single_entry() {
        let entries = vec![BreadcrumbEntry {
            title: "Only commit".to_string(),
            pr_number: Some(1),
            pr_url: Some("https://github.com/org/repo/pull/1".to_string()),
        }];
        let block = render_breadcrumbs("solo", &entries, 0, "PR", "#");
        assert!(block.contains("— 1/1"));
        assert!(block.contains("THIS (only entry)"));
    }

    #[test]
    fn test_render_breadcrumbs_gitlab_prefix() {
        let entries = vec![
            BreadcrumbEntry {
                title: "First".to_string(),
                pr_number: Some(100),
                pr_url: Some("https://gitlab.com/org/repo/-/merge_requests/100".to_string()),
            },
            BreadcrumbEntry {
                title: "Second".to_string(),
                pr_number: Some(101),
                pr_url: None,
            },
        ];
        let block = render_breadcrumbs("gl-stack", &entries, 0, "MR", "!");
        assert!(block.contains("MR !101"));
        assert!(block.contains("!100"));
    }

    #[test]
    fn test_render_breadcrumbs_entry_without_pr() {
        let entries = vec![
            BreadcrumbEntry {
                title: "Has PR".to_string(),
                pr_number: Some(10),
                pr_url: Some("https://example.com/10".to_string()),
            },
            BreadcrumbEntry {
                title: "No PR yet".to_string(),
                pr_number: None,
                pr_url: None,
            },
        ];
        let block = render_breadcrumbs("mixed", &entries, 0, "PR", "#");
        // Entry without PR should have no link
        assert!(block.contains("2. No PR yet\n"));
    }

    #[test]
    fn test_splice_breadcrumbs_append_to_empty() {
        let block = "<!-- gg:breadcrumbs:start -->\ntest\n<!-- gg:breadcrumbs:end -->";
        let (result, changed) = splice_breadcrumbs("", block);
        assert!(changed);
        assert!(result.contains(block));
    }

    #[test]
    fn test_splice_breadcrumbs_append_to_existing_body() {
        let body = "User-written description\n\nMore details.";
        let block = "<!-- gg:breadcrumbs:start -->\nstuff\n<!-- gg:breadcrumbs:end -->";
        let (result, changed) = splice_breadcrumbs(body, block);
        assert!(changed);
        // User content preserved
        assert!(result.starts_with("User-written description\n\nMore details."));
        // Separator between user content and breadcrumbs
        assert!(result.contains("\n\n---\n\n"));
        assert!(result.contains(block));
    }

    #[test]
    fn test_splice_breadcrumbs_replace_existing() {
        let old_block = "<!-- gg:breadcrumbs:start -->\nold stuff\n<!-- gg:breadcrumbs:end -->";
        let new_block = "<!-- gg:breadcrumbs:start -->\nnew stuff\n<!-- gg:breadcrumbs:end -->";
        let body = format!("User description\n\n---\n\n{}\n", old_block);
        let (result, changed) = splice_breadcrumbs(&body, new_block);
        assert!(changed);
        // Old block replaced
        assert!(!result.contains("old stuff"));
        assert!(result.contains("new stuff"));
        // User content preserved
        assert!(result.starts_with("User description\n\n---\n\n"));
    }

    #[test]
    fn test_splice_breadcrumbs_idempotent() {
        let block = "<!-- gg:breadcrumbs:start -->\nstuff\n<!-- gg:breadcrumbs:end -->";
        let body = format!("Description\n\n---\n\n{}\n", block);
        let (result, changed) = splice_breadcrumbs(&body, block);
        assert!(!changed);
        assert_eq!(result, body);
    }

    #[test]
    fn test_splice_breadcrumbs_preserves_content_after_markers() {
        let old_block = "<!-- gg:breadcrumbs:start -->\nold\n<!-- gg:breadcrumbs:end -->";
        let new_block = "<!-- gg:breadcrumbs:start -->\nnew\n<!-- gg:breadcrumbs:end -->";
        let body = format!("Before\n{}\nAfter", old_block);
        let (result, changed) = splice_breadcrumbs(&body, new_block);
        assert!(changed);
        assert!(result.starts_with("Before\n"));
        assert!(result.ends_with("\nAfter"));
        assert!(result.contains("new"));
    }

    // --- Template tests ---

    #[test]
    fn test_load_template_exists() {
        let temp = TempDir::new().unwrap();
        let gg_dir = temp.path().join("gg");
        fs::create_dir_all(&gg_dir).unwrap();
        fs::write(gg_dir.join("pr_template.md"), "Hello {{title}}").unwrap();

        let template = load_template(temp.path());
        assert!(template.is_some());
        assert_eq!(template.unwrap(), "Hello {{title}}");
    }

    #[test]
    fn test_load_template_not_exists() {
        let temp = TempDir::new().unwrap();
        let template = load_template(temp.path());
        assert!(template.is_none());
    }

    #[test]
    fn test_render_template_all_placeholders() {
        let template =
            "# {{title}}\n\n{{description}}\n\n---\nStack: {{stack_name}}\nCommit: {{commit_sha}}";
        let ctx = TemplateContext {
            description: Some("This is the description"),
            stack_name: "my-feature",
            commit_sha: "abc1234",
            title: "Add new feature",
        };

        let result = render_template(template, &ctx);
        assert_eq!(
            result,
            "# Add new feature\n\nThis is the description\n\n---\nStack: my-feature\nCommit: abc1234"
        );
    }

    #[test]
    fn test_render_template_no_description() {
        let template = "Title: {{title}}\nDesc: {{description}}";
        let ctx = TemplateContext {
            description: None,
            stack_name: "stack",
            commit_sha: "abc",
            title: "Test",
        };

        let result = render_template(template, &ctx);
        assert_eq!(result, "Title: Test\nDesc: ");
    }

    #[test]
    fn test_render_template_multiple_same_placeholder() {
        let template = "{{title}} - {{title}}";
        let ctx = TemplateContext {
            description: None,
            stack_name: "stack",
            commit_sha: "abc",
            title: "Test",
        };

        let result = render_template(template, &ctx);
        assert_eq!(result, "Test - Test");
    }
}
