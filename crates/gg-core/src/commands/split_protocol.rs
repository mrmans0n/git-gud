//! Versioned DTOs and stable identities for structured split operations.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use git2::{ObjectType, Oid};
use serde::{Deserialize, Serialize};

use crate::error::{GgError, Result};

pub const SPLIT_PROTOCOL_VERSION: u32 = 1;

/// A single line in a diff hunk.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffLine {
    /// Origin character: '+' (added), '-' (deleted), ' ' (context).
    pub origin: char,
    /// The line content without the origin character.
    pub content: String,
}

/// A contiguous change in a file.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffHunk {
    /// File path relative to the repository root.
    pub file_path: String,
    /// Hunk header (for example, "@@ -10,6 +10,12 @@").
    pub header: String,
    /// Lines in the hunk.
    pub lines: Vec<DiffLine>,
    /// Starting line number in the old file.
    pub old_start: u32,
    /// Number of lines in the old file.
    pub old_lines: u32,
    /// Starting line number in the new file.
    pub new_start: u32,
    /// Number of lines in the new file.
    pub new_lines: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SplitTargetIdentity {
    pub gg_id: Option<String>,
    pub sha: String,
    pub tree: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SplitHunkDescription {
    pub id: String,
    pub path: String,
    pub header: String,
    pub patch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SplitDescribeResponse {
    pub version: u32,
    pub plan_token: String,
    pub target: SplitTargetIdentity,
    pub hunks: Vec<SplitHunkDescription>,
    pub non_textual_files: Vec<String>,
    pub first_message: String,
    pub remainder_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SplitPlanV1 {
    pub version: u32,
    pub plan_token: String,
    pub target: SplitTargetIdentity,
    pub selected_hunk_ids: Vec<String>,
    pub first_message: String,
    pub remainder_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SplitCommitIdentity {
    pub sha: String,
    pub gg_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SplitApplyResult {
    pub operation_id: String,
    pub original_sha: String,
    pub first: SplitCommitIdentity,
    pub remainder: SplitCommitIdentity,
    pub rewritten_descendants: Vec<SplitCommitIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SplitApplyResponse {
    pub version: u32,
    #[serde(flatten)]
    pub result: SplitApplyResult,
}

/// Convert an internal hunk into its public protocol representation.
pub fn describe_hunk(index: usize, hunk: &DiffHunk) -> SplitHunkDescription {
    let patch = canonical_patch(hunk);
    let canonical = format!("{index}\0{}\0{patch}", hunk.file_path);
    let oid = hash_blob(&canonical);

    SplitHunkDescription {
        id: format!("h-{}", &oid.to_string()[..12]),
        path: hunk.file_path.clone(),
        header: hunk.header.clone(),
        patch,
    }
}

/// Build a token binding a Describe response to its target and ordered hunks.
pub fn plan_token(target: &SplitTargetIdentity, hunks: &[SplitHunkDescription]) -> String {
    let gg_id = match &target.gg_id {
        Some(gg_id) => format!("some:{}:{gg_id}", gg_id.len()),
        None => "none".into(),
    };
    let mut canonical = format!(
        "{}\0{}\0{}\0{}",
        SPLIT_PROTOCOL_VERSION, gg_id, target.sha, target.tree
    );
    for hunk in hunks {
        canonical.push('\0');
        canonical.push_str(&hunk.id);
    }
    let oid = hash_blob(&canonical);
    format!("split-v1-{}", &oid.to_string()[..12])
}

/// Read and validate a version 1 structured split plan.
pub fn read_plan(path: &Path) -> Result<SplitPlanV1> {
    let contents = fs::read_to_string(path).map_err(|error| {
        GgError::Other(format!(
            "Failed to read split plan '{}': {error}",
            path.display()
        ))
    })?;
    let plan: SplitPlanV1 = serde_json::from_str(&contents).map_err(|error| {
        GgError::Other(format!(
            "Failed to parse split plan JSON '{}': {error}",
            path.display()
        ))
    })?;

    if plan.version != SPLIT_PROTOCOL_VERSION {
        return Err(GgError::Other(format!(
            "Unsupported split plan version {}; expected {}",
            plan.version, SPLIT_PROTOCOL_VERSION
        )));
    }
    if plan.first_message.trim().is_empty() {
        return Err(GgError::Other(
            "Split plan first_message must not be empty".into(),
        ));
    }
    if plan.remainder_message.trim().is_empty() {
        return Err(GgError::Other(
            "Split plan remainder_message must not be empty".into(),
        ));
    }

    let mut unique_ids = HashSet::new();
    if let Some(duplicate) = plan
        .selected_hunk_ids
        .iter()
        .find(|id| !unique_ids.insert(id.as_str()))
    {
        return Err(GgError::Other(format!(
            "Split plan contains duplicate hunk ID '{duplicate}'"
        )));
    }

    Ok(plan)
}

fn canonical_patch(hunk: &DiffHunk) -> String {
    let mut patch = String::with_capacity(
        hunk.header.len()
            + 1
            + hunk
                .lines
                .iter()
                .map(|line| line.content.len() + 1)
                .sum::<usize>(),
    );
    patch.push_str(&hunk.header);
    patch.push('\n');
    for line in &hunk.lines {
        patch.push(line.origin);
        patch.push_str(&line.content);
    }
    patch
}

fn hash_blob(value: &str) -> Oid {
    Oid::hash_object(ObjectType::Blob, value.as_bytes())
        .expect("hashing in-memory split protocol data cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use tempfile::tempdir;

    fn test_target(sha: &str) -> SplitTargetIdentity {
        SplitTargetIdentity {
            gg_id: Some("c-abc1234".into()),
            sha: sha.into(),
            tree: "cccccccccccccccccccccccccccccccccccccccc".into(),
        }
    }

    fn test_hunk(file_path: &str, header: &str, patch: &str) -> DiffHunk {
        DiffHunk {
            file_path: file_path.into(),
            header: header.into(),
            lines: patch
                .split_inclusive('\n')
                .map(|line| DiffLine {
                    origin: line.chars().next().unwrap(),
                    content: line[1..].into(),
                })
                .collect(),
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 1,
        }
    }

    #[test]
    fn hunk_ids_are_stable_and_content_sensitive() {
        let a = test_hunk("src/lib.rs", "@@ -1,1 +1,1 @@", "-old\n+new\n");
        let b = test_hunk("src/lib.rs", "@@ -1,1 +1,1 @@", "-old\n+new\n");
        let changed = test_hunk("src/lib.rs", "@@ -1,1 +1,1 @@", "-old\n+other\n");
        assert_eq!(describe_hunk(0, &a).id, describe_hunk(0, &b).id);
        assert_ne!(describe_hunk(0, &a).id, describe_hunk(0, &changed).id);
    }

    #[test]
    fn duplicate_hunks_have_distinct_ids() {
        let hunk = test_hunk("src/lib.rs", "@@ -1,1 +1,1 @@", "-old\n+new\n");
        assert_ne!(describe_hunk(0, &hunk).id, describe_hunk(1, &hunk).id);
    }

    #[test]
    fn plan_token_changes_with_target_or_hunks() {
        let target = test_target("aaaaaaaa");
        let hunk = describe_hunk(0, &test_hunk("a", "@@ -1 +1 @@", "-a\n+b\n"));
        let cloned_hunks = vec![hunk.clone()];
        assert_eq!(
            plan_token(&target, std::slice::from_ref(&hunk)),
            plan_token(&target, &cloned_hunks)
        );
        assert_ne!(
            plan_token(&target, &cloned_hunks),
            plan_token(&test_target("bbbbbbbb"), std::slice::from_ref(&hunk))
        );

        let other_hunk = describe_hunk(1, &test_hunk("b", "@@ -1 +1 @@", "-c\n+d\n"));
        assert_ne!(
            plan_token(&target, std::slice::from_ref(&hunk)),
            plan_token(&target, std::slice::from_ref(&other_hunk))
        );
        assert_ne!(
            plan_token(&target, &[hunk.clone(), other_hunk.clone()]),
            plan_token(&target, &[other_hunk, hunk])
        );
    }

    #[test]
    fn plan_token_distinguishes_missing_and_empty_gg_ids() {
        let mut without_gg_id = test_target("aaaaaaaa");
        without_gg_id.gg_id = None;
        let mut empty_gg_id = without_gg_id.clone();
        empty_gg_id.gg_id = Some(String::new());

        assert_ne!(
            plan_token(&without_gg_id, &[]),
            plan_token(&empty_gg_id, &[])
        );
    }

    #[test]
    fn plan_v1_round_trips() {
        let plan = SplitPlanV1 {
            version: 1,
            plan_token: "token".into(),
            target: test_target("aaaaaaaa"),
            selected_hunk_ids: vec!["h-abc".into()],
            first_message: "first".into(),
            remainder_message: "remainder".into(),
        };
        let json = serde_json::to_string(&plan).unwrap();
        assert_eq!(serde_json::from_str::<SplitPlanV1>(&json).unwrap(), plan);
    }

    #[test]
    fn read_plan_rejects_unsupported_versions() {
        let mut plan = test_plan();
        plan.version = 2;
        let error = write_and_read_plan(&plan).unwrap_err();
        assert!(error
            .to_string()
            .contains("Unsupported split plan version 2"));
    }

    #[test]
    fn read_plan_rejects_empty_messages() {
        let mut plan = test_plan();
        plan.first_message = "  \n".into();
        let error = write_and_read_plan(&plan).unwrap_err();
        assert!(error
            .to_string()
            .contains("first_message must not be empty"));

        let mut plan = test_plan();
        plan.remainder_message = "\t".into();
        let error = write_and_read_plan(&plan).unwrap_err();
        assert!(error
            .to_string()
            .contains("remainder_message must not be empty"));
    }

    #[test]
    fn read_plan_rejects_duplicate_hunk_ids() {
        let mut plan = test_plan();
        plan.selected_hunk_ids.push("h-abc".into());
        let error = write_and_read_plan(&plan).unwrap_err();
        assert!(error.to_string().contains("duplicate hunk ID 'h-abc'"));
    }

    #[test]
    fn read_plan_reports_unreadable_json() {
        let directory = tempdir().unwrap();
        let missing = directory.path().join("missing.json");
        let error = read_plan(&missing).unwrap_err();
        assert!(error.to_string().contains("Failed to read split plan"));

        let invalid = directory.path().join("invalid.json");
        fs::write(&invalid, "not json").unwrap();
        let error = read_plan(&invalid).unwrap_err();
        assert!(error
            .to_string()
            .contains("Failed to parse split plan JSON"));
    }

    fn test_plan() -> SplitPlanV1 {
        SplitPlanV1 {
            version: SPLIT_PROTOCOL_VERSION,
            plan_token: "token".into(),
            target: test_target("aaaaaaaa"),
            selected_hunk_ids: vec!["h-abc".into()],
            first_message: "first".into(),
            remainder_message: "remainder".into(),
        }
    }

    fn write_and_read_plan(plan: &SplitPlanV1) -> Result<SplitPlanV1> {
        let directory = tempdir().unwrap();
        let path = directory.path().join("plan.json");
        fs::write(&path, serde_json::to_vec(plan).unwrap()).unwrap();
        read_plan(&path)
    }
}
