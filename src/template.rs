//! PR/MR template support for git-gud
//!
//! Loads and processes optional template files from `.git/gg/pr_template.md`
//! with support for placeholder replacement.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Result;

/// Get the path to the PR template file
pub fn template_path(git_dir: &Path) -> PathBuf {
    git_dir.join("gg").join("pr_template.md")
}

/// Load and process PR template with placeholder replacement
///
/// Returns None if template file doesn't exist
pub fn load_and_process_template(
    git_dir: &Path,
    placeholders: &HashMap<&str, &str>,
) -> Result<Option<String>> {
    let path = template_path(git_dir);

    if !path.exists() {
        return Ok(None);
    }

    let template = fs::read_to_string(&path)?;
    let processed = replace_placeholders(&template, placeholders);
    Ok(Some(processed))
}

/// Replace placeholders in template string
///
/// Supported placeholders:
/// - `{{description}}` - commit description/body
/// - `{{stack_name}}` - name of the current stack
/// - `{{commit_sha}}` - short SHA of the commit
/// - `{{title}}` - the PR/MR title
fn replace_placeholders(template: &str, placeholders: &HashMap<&str, &str>) -> String {
    let mut result = template.to_string();

    for (key, value) in placeholders {
        let placeholder = format!("{{{{{}}}}}", key);
        result = result.replace(&placeholder, value);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_replace_placeholders() {
        let template =
            "Title: {{title}}\n\n{{description}}\n\nStack: {{stack_name}}\nCommit: {{commit_sha}}";
        let mut placeholders = HashMap::new();
        placeholders.insert("title", "Add feature");
        placeholders.insert("description", "This adds a new feature");
        placeholders.insert("stack_name", "my-feature");
        placeholders.insert("commit_sha", "abc1234");

        let result = replace_placeholders(template, &placeholders);
        assert_eq!(
            result,
            "Title: Add feature\n\nThis adds a new feature\n\nStack: my-feature\nCommit: abc1234"
        );
    }

    #[test]
    fn test_replace_placeholders_partial() {
        let template = "{{title}}\n\n{{description}}";
        let mut placeholders = HashMap::new();
        placeholders.insert("title", "Add feature");
        placeholders.insert("description", "Details here");
        placeholders.insert("unused", "ignored");

        let result = replace_placeholders(template, &placeholders);
        assert_eq!(result, "Add feature\n\nDetails here");
    }

    #[test]
    fn test_replace_placeholders_missing_values() {
        let template = "{{title}}\n\n{{description}}\n\n{{missing}}";
        let mut placeholders = HashMap::new();
        placeholders.insert("title", "Add feature");
        placeholders.insert("description", "Details here");

        let result = replace_placeholders(template, &placeholders);
        // Missing placeholders are left as-is
        assert_eq!(result, "Add feature\n\nDetails here\n\n{{missing}}");
    }

    #[test]
    fn test_load_template_not_exists() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        let placeholders = HashMap::new();
        let result = load_and_process_template(git_dir, &placeholders).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_and_process_template() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        // Create gg directory
        fs::create_dir_all(git_dir.join("gg")).unwrap();

        // Write template file
        let template_content = "## {{title}}\n\n{{description}}\n\n**Stack:** {{stack_name}}";
        fs::write(template_path(git_dir), template_content).unwrap();

        // Process template
        let mut placeholders = HashMap::new();
        placeholders.insert("title", "Feature PR");
        placeholders.insert("description", "Adds new functionality");
        placeholders.insert("stack_name", "my-stack");

        let result = load_and_process_template(git_dir, &placeholders).unwrap();
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "## Feature PR\n\nAdds new functionality\n\n**Stack:** my-stack"
        );
    }

    #[test]
    fn test_template_with_multiline_description() {
        let template = "# {{title}}\n\n## Description\n{{description}}\n\n## Stack Info\nStack: {{stack_name}}\nCommit: {{commit_sha}}";
        let mut placeholders = HashMap::new();
        placeholders.insert("title", "Add feature");
        placeholders.insert("description", "Line 1\nLine 2\nLine 3");
        placeholders.insert("stack_name", "feature");
        placeholders.insert("commit_sha", "abc1234");

        let result = replace_placeholders(template, &placeholders);
        assert!(result.contains("Line 1\nLine 2\nLine 3"));
    }

    #[test]
    fn test_template_with_special_characters() {
        let template = "{{title}}\n\n{{description}}";
        let mut placeholders = HashMap::new();
        placeholders.insert("title", "Fix bug #123");
        placeholders.insert("description", "Resolves issue with $ and @ symbols");

        let result = replace_placeholders(template, &placeholders);
        assert_eq!(
            result,
            "Fix bug #123\n\nResolves issue with $ and @ symbols"
        );
    }
}
