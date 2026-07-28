use std::{
    fs,
    path::{Path, PathBuf},
};

fn skill_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../skills/gg")
        .canonicalize()
        .expect("skills/gg must exist")
}

fn read(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn gg_skill_is_a_compact_goal_router() {
    let root = skill_root();
    let skill = read(root.join("SKILL.md"));
    let references = [
        ("setup and inspection", "setup-and-inspection.md"),
        ("editing stacks", "editing-stacks.md"),
        ("syncing and reviews", "syncing-and-reviews.md"),
        ("landing and cleanup", "landing-and-cleanup.md"),
        ("recovery", "recovery.md"),
        ("native clients", "native-clients.md"),
    ];

    assert!(
        skill.lines().count() <= 180,
        "SKILL.md must stay at or below 180 lines"
    );
    assert!(
        skill.starts_with(
            "---\nname: gg\ndescription: Use when a user explicitly asks to use git-gud (gg), \
the exact terms stacked diffs, stacked PRs, or stacked MRs, or when operating \
in a repository already managed as a gg stack.\n---\n"
        ),
        "frontmatter must preserve the approved activation boundary"
    );

    let reference_root = root.join("references");
    let mut actual_references = fs::read_dir(&reference_root)
        .expect("skills/gg/references must exist")
        .map(|entry| {
            let entry = entry.expect("reference directory entry must be readable");
            assert!(
                entry
                    .file_type()
                    .expect("reference entry type must be readable")
                    .is_file(),
                "skills/gg/references must contain files only; found {}",
                entry.path().display()
            );
            entry
                .file_name()
                .into_string()
                .expect("reference filenames must be UTF-8")
        })
        .collect::<Vec<_>>();
    actual_references.sort();

    let mut expected_references = references
        .iter()
        .map(|(_, reference)| (*reference).to_string())
        .collect::<Vec<_>>();
    expected_references.sort();
    assert_eq!(
        actual_references, expected_references,
        "skills/gg/references must contain exactly the six routed workflow files"
    );

    for (label, reference) in references {
        let relative = format!("references/{reference}");
        assert!(
            skill.contains(&format!("[{label}]({relative})")),
            "SKILL.md must contain the exact Markdown router link [{label}]({relative})"
        );

        let body = read(root.join(&relative));
        for heading in [
            "## Preconditions",
            "## Procedure",
            "## Stop conditions",
            "## Verification",
            "## Report",
        ] {
            assert!(body.contains(heading), "{relative} must contain {heading}");
        }
    }

    assert!(
        !root.join("reference.md").exists(),
        "the monolithic reference.md must be removed"
    );
    assert!(
        !root.join("examples").exists(),
        "human tutorials must not ship inside the operational skill"
    );
    assert!(
        !skill.contains("## Common operations"),
        "SKILL.md must not contain a command catalog"
    );
    assert!(
        !skill.contains("## MCP Server Usage for Agents"),
        "native-client details must be routed"
    );
}
