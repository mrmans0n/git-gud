const PREVIEW_WORKFLOW: &str = include_str!("../../../.github/workflows/docs-preview.yml");

fn step(name: &str) -> &str {
    let marker = format!("      - name: {name}\n");
    PREVIEW_WORKFLOW
        .split_once(&marker)
        .unwrap_or_else(|| panic!("missing workflow step: {name}"))
        .1
        .split("\n      - ")
        .next()
        .expect("step body must exist")
}

#[test]
fn preview_publication_requires_a_same_repository_pr() {
    let publication = step("Publish PR preview");
    let condition = publication
        .lines()
        .find_map(|line| line.strip_prefix("        if: "));

    assert_eq!(
        condition,
        Some("${{ github.event.pull_request.head.repo.full_name == github.repository }}"),
        "fork PR tokens cannot publish to gh-pages; gate only the publication step"
    );
}

#[test]
fn documentation_still_builds_for_external_prs() {
    let job_configuration = PREVIEW_WORKFLOW
        .split_once("    steps:\n")
        .expect("preview job must have steps")
        .0;
    assert!(
        !job_configuration
            .lines()
            .any(|line| line.trim_start().starts_with("if:")),
        "do not skip the entire validation job for fork PRs"
    );
    let build = step("Build docs");
    assert!(build.contains("run: mdbook build docs"));
    assert!(
        !build
            .lines()
            .any(|line| line.trim_start().starts_with("if:")),
        "documentation must be built even when publication is not allowed"
    );
}

#[test]
fn documentation_validation_keeps_the_unprivileged_pr_event() {
    assert!(PREVIEW_WORKFLOW.contains("\n  pull_request:\n"));
    assert!(
        !PREVIEW_WORKFLOW.contains("pull_request_target"),
        "never grant elevated credentials to the contributor checkout"
    );
}

#[test]
fn same_repository_preview_configuration_is_preserved() {
    let publication = step("Publish PR preview");
    assert!(publication.contains("uses: rossjrw/pr-preview-action@v1"));
    assert!(publication.contains("source-dir: docs/book/"));
    assert!(publication.contains("preview-branch: gh-pages"));
    assert!(publication.contains("umbrella-dir: pr-preview"));
}
