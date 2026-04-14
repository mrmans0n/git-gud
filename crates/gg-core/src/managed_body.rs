//! Managed PR/MR body ownership.
//!
//! Provides helpers to wrap generated PR/MR descriptions in managed markers
//! so that `gg sync` can update only the generated section and preserve
//! user edits made outside that block.

/// Start marker for git-gud managed content.
const MANAGED_START: &str = "<!-- gg:managed:start -->";
/// End marker for git-gud managed content.
const MANAGED_END: &str = "<!-- gg:managed:end -->";

/// Wrap generated content in managed markers.
///
/// Returns a string with the content enclosed between start and end markers.
pub fn wrap(content: &str) -> String {
    format!("{}\n{}\n{}", MANAGED_START, content, MANAGED_END)
}

/// Extract the content between managed markers, if present.
///
/// Returns `None` if the body does not contain both markers in order.
pub fn extract_managed(body: &str) -> Option<&str> {
    let start_idx = body.rfind(MANAGED_START)?;
    let mut content_start = start_idx + MANAGED_START.len();
    // Skip the newline after the start marker if present
    if body.as_bytes().get(content_start) == Some(&b'\n') {
        content_start += 1;
    }
    let end_idx = body.get(content_start..)?.find(MANAGED_END)?;
    let content = &body[content_start..content_start + end_idx];
    // Trim trailing newline before end marker
    Some(content.strip_suffix('\n').unwrap_or(content))
}

/// Replace the managed block in an existing body with new generated content,
/// preserving all text outside the markers.
///
/// Returns `None` if the body does not contain managed markers (legacy body).
pub fn replace_managed(existing_body: &str, new_content: &str) -> Option<String> {
    let start_idx = existing_body.rfind(MANAGED_START)?;
    let after_start = start_idx + MANAGED_START.len();
    let end_idx = existing_body[after_start..].find(MANAGED_END)?;
    let absolute_end = after_start + end_idx + MANAGED_END.len();

    let before = &existing_body[..start_idx];
    let after = &existing_body[absolute_end..];

    Some(format!("{}{}{}", before, wrap(new_content), after))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_basic() {
        let result = wrap("Hello world");
        assert_eq!(
            result,
            "<!-- gg:managed:start -->\nHello world\n<!-- gg:managed:end -->"
        );
    }

    #[test]
    fn test_wrap_multiline() {
        let content = "Line 1\nLine 2\nLine 3";
        let result = wrap(content);
        assert_eq!(
            result,
            "<!-- gg:managed:start -->\nLine 1\nLine 2\nLine 3\n<!-- gg:managed:end -->"
        );
    }

    #[test]
    fn test_wrap_empty() {
        let result = wrap("");
        assert_eq!(
            result,
            "<!-- gg:managed:start -->\n\n<!-- gg:managed:end -->"
        );
    }

    #[test]
    fn test_extract_managed_basic() {
        let body = "<!-- gg:managed:start -->\nHello world\n<!-- gg:managed:end -->";
        assert_eq!(extract_managed(body), Some("Hello world"));
    }

    #[test]
    fn test_extract_managed_multiline() {
        let body = "<!-- gg:managed:start -->\nLine 1\nLine 2\n<!-- gg:managed:end -->";
        assert_eq!(extract_managed(body), Some("Line 1\nLine 2"));
    }

    #[test]
    fn test_extract_managed_with_surrounding_content() {
        let body = "User notes above\n\n<!-- gg:managed:start -->\nGenerated\n<!-- gg:managed:end -->\n\nUser notes below";
        assert_eq!(extract_managed(body), Some("Generated"));
    }

    #[test]
    fn test_extract_managed_empty_content() {
        let body = "<!-- gg:managed:start -->\n\n<!-- gg:managed:end -->";
        assert_eq!(extract_managed(body), Some(""));
    }

    #[test]
    fn test_extract_managed_no_markers() {
        let body = "Just a plain PR body with no markers";
        assert_eq!(extract_managed(body), None);
    }

    #[test]
    fn test_extract_managed_only_start_marker() {
        let body = "<!-- gg:managed:start -->\nContent without end";
        assert_eq!(extract_managed(body), None);
    }

    #[test]
    fn test_replace_managed_basic() {
        let existing = "<!-- gg:managed:start -->\nOld content\n<!-- gg:managed:end -->";
        let result = replace_managed(existing, "New content").unwrap();
        assert_eq!(
            result,
            "<!-- gg:managed:start -->\nNew content\n<!-- gg:managed:end -->"
        );
    }

    #[test]
    fn test_replace_managed_preserves_surrounding_content() {
        let existing = "User header\n\n<!-- gg:managed:start -->\nOld generated\n<!-- gg:managed:end -->\n\nUser footer";
        let result = replace_managed(existing, "New generated").unwrap();
        assert_eq!(
            result,
            "User header\n\n<!-- gg:managed:start -->\nNew generated\n<!-- gg:managed:end -->\n\nUser footer"
        );
    }

    #[test]
    fn test_replace_managed_preserves_checked_checkboxes_outside_block() {
        let existing = "## Review checklist\n- [x] Tests pass\n- [x] Code reviewed\n\n<!-- gg:managed:start -->\nOld description\n<!-- gg:managed:end -->\n\n## Notes\nLGTM";
        let result = replace_managed(existing, "Updated description").unwrap();
        assert!(result.contains("- [x] Tests pass"));
        assert!(result.contains("- [x] Code reviewed"));
        assert!(result.contains("Updated description"));
        assert!(result.contains("## Notes\nLGTM"));
    }

    #[test]
    fn test_replace_managed_no_markers_returns_none() {
        let existing = "Plain body with no markers";
        assert!(replace_managed(existing, "New content").is_none());
    }

    #[test]
    fn test_replace_managed_only_start_returns_none() {
        let existing = "<!-- gg:managed:start -->\nContent but no end marker";
        assert!(replace_managed(existing, "New content").is_none());
    }

    #[test]
    fn test_roundtrip_wrap_then_extract() {
        let content = "# PR Title\n\nSome description\n\nStack: my-stack | Commit: abc1234";
        let wrapped = wrap(content);
        let extracted = extract_managed(&wrapped).unwrap();
        assert_eq!(extracted, content);
    }

    #[test]
    fn test_roundtrip_wrap_then_replace() {
        let original = "Original content";
        let wrapped = wrap(original);
        let replaced = replace_managed(&wrapped, "Updated content").unwrap();
        let extracted = extract_managed(&replaced).unwrap();
        assert_eq!(extracted, "Updated content");
    }

    #[test]
    fn test_scenario_checklist_survives_sync() {
        // Simulate: PR created with managed body, user adds checklist outside block
        let initial_body = wrap("Generated description v1");

        // User edits the body on GitHub, adding content before and after
        let user_edited = format!(
            "## Review checklist\n- [x] Tests pass\n- [ ] Performance checked\n\n{}\n\n## Reviewer notes\nLooks good!",
            initial_body
        );

        // Sync happens with updated description
        let synced = replace_managed(&user_edited, "Generated description v2").unwrap();

        // User's checklist is preserved
        assert!(synced.contains("- [x] Tests pass"));
        assert!(synced.contains("- [ ] Performance checked"));
        assert!(synced.contains("## Reviewer notes\nLooks good!"));
        // Generated content is updated
        assert!(synced.contains("Generated description v2"));
        assert!(!synced.contains("Generated description v1"));
    }

    #[test]
    fn test_scenario_multiple_syncs_preserve_edits() {
        let body = wrap("v1");

        // User adds content
        let edited = format!("User header\n\n{}\n\nUser footer", body);

        // First re-sync
        let synced1 = replace_managed(&edited, "v2").unwrap();
        assert!(synced1.contains("User header"));
        assert!(synced1.contains("User footer"));
        assert!(synced1.contains("v2"));

        // Second re-sync
        let synced2 = replace_managed(&synced1, "v3").unwrap();
        assert!(synced2.contains("User header"));
        assert!(synced2.contains("User footer"));
        assert!(synced2.contains("v3"));
        assert!(!synced2.contains("v2"));
    }

    #[test]
    fn test_extract_managed_no_newline_after_start() {
        // Start marker immediately followed by content (no \n)
        let body = "<!-- gg:managed:start -->content\n<!-- gg:managed:end -->";
        assert_eq!(extract_managed(body), Some("content"));
    }

    #[test]
    fn test_extract_managed_start_marker_at_end() {
        // Start marker at end of string with nothing after it
        let body = "some text\n<!-- gg:managed:start -->";
        assert_eq!(extract_managed(body), None);
    }

    #[test]
    fn test_scenario_user_deletes_markers_treated_as_legacy() {
        // If user removes markers, we treat it as legacy and don't touch it
        let body = "User completely rewrote the PR body";
        assert!(replace_managed(body, "New generated").is_none());
    }

    #[test]
    fn test_marker_text_in_user_content_before_managed_block() {
        // If a user references marker syntax in their PR body before the actual
        // managed block, rfind ensures we find the real managed block, not the
        // false positive in user text.
        let managed = wrap("Real generated content");
        let body = format!(
            "See docs — markers look like `<!-- gg:managed:start -->`.\n\n{}",
            managed
        );
        let extracted = extract_managed(&body).unwrap();
        assert_eq!(extracted, "Real generated content");

        let replaced = replace_managed(&body, "Updated content").unwrap();
        assert!(replaced.contains("markers look like"));
        assert!(replaced.contains("Updated content"));
        assert!(!replaced.contains("Real generated content"));
    }

    #[test]
    fn test_replace_managed_marker_in_generated_content() {
        // If generated content contains the end marker text, replace_managed
        // matches the first end marker after the start marker. This is a
        // degenerate case — generated content should never contain markers.
        let existing = wrap("Normal content");
        let result = replace_managed(&existing, "Content with <!-- gg:managed:end --> inside");
        assert!(result.is_some());
        // The new content is wrapped correctly
        let body = result.unwrap();
        assert!(body.contains("Content with <!-- gg:managed:end --> inside"));
    }
}
