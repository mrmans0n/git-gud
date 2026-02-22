//! PR/MR description template support
//!
//! Templates are stored in `.git/gg/pr_template.md` and support placeholders:
//! - `{{description}}` - the commit description
//! - `{{stack_name}}` - name of the current stack
//! - `{{commit_sha}}` - short SHA of the commit
//! - `{{title}}` - the PR/MR title

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

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
