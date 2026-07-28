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
        "setup-and-inspection.md",
        "editing-stacks.md",
        "syncing-and-reviews.md",
        "landing-and-cleanup.md",
        "recovery.md",
        "native-clients.md",
    ];

    assert!(
        skill.lines().count() <= 180,
        "SKILL.md must stay at or below 180 lines"
    );
    assert!(
        skill.starts_with(
            "---\nname: gg\ndescription: Use when a user asks to use git-gud (gg), \
stacked diffs, stacked PRs or MRs, or when operating in a repository already \
managed as a gg stack.\n---\n"
        ),
        "frontmatter must preserve the approved activation boundary"
    );

    for reference in references {
        let relative = format!("references/{reference}");
        assert!(
            skill.contains(&relative),
            "SKILL.md must route directly to {relative}"
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
