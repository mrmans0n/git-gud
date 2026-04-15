# Stack Navigation Comment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the stack-navigation comment feature from the spec at [docs/superpowers/specs/2026-04-15-stack-nav-comment-design.md](../specs/2026-04-15-stack-nav-comment-design.md). When the new `stack_nav_comments` setting is enabled, `gg sync` upserts a managed comment on each open PR/MR listing all stack entries with a 👉 marker on the current one; when disabled, it cleans up any such comments it previously created.

**Architecture:** A pure `stack_nav` module renders the comment body. Four new provider methods (`find_managed_comment`, `create_pr_comment`, `update_pr_comment`, `delete_pr_comment`) add the transport layer on top of `gh api` / `glab api`. A second pass at the end of `gg sync` reconciles nav comments using the state accumulated during the first pass (PR numbers + states).

**Tech Stack:** Rust workspace, `git2`, `serde`, `gh` / `glab` CLIs via `std::process::Command`, existing `dialoguer` / `console` / `indicatif` for UX.

---

## File Structure

**Created files:**
- `crates/gg-core/src/stack_nav.rs` — pure rendering + marker helpers (MARKER const, StackNavEntry, render, is_managed_comment)

**Modified files:**
- `crates/gg-core/src/lib.rs` — register new `stack_nav` module
- `crates/gg-core/src/config.rs` — add `stack_nav_comments: bool` to `Defaults`
- `crates/gg-core/src/gh.rs` — add `list_issue_comments`, `create_issue_comment`, `update_issue_comment`, `delete_issue_comment`
- `crates/gg-core/src/glab.rs` — add `list_mr_notes`, `create_mr_note`, `update_mr_note`, `delete_mr_note`
- `crates/gg-core/src/provider.rs` — add `ManagedComment` struct and four new `Provider` methods that dispatch to gh/glab
- `crates/gg-core/src/output.rs` — add optional `nav_comment_action` field to `SyncEntryResultJson`
- `crates/gg-core/src/commands/sync.rs` — reconcile nav comments after the main PR loop, populate JSON field
- `crates/gg-core/src/commands/setup.rs` — add prompt for the new setting under the "Sync" group
- `docs/src/configuration.md` — document the new setting
- `docs/src/commands/sync.md` — mention nav-comment behavior
- `docs/src/commands/setup.md` — document the new prompt
- `skills/gg/SKILL.md` — add an operating note that `gg sync` may leave a managed comment
- `skills/gg/reference.md` — document the new JSON field and config setting
- `README.md` — add to feature list

---

## Task 1: Create `stack_nav` module with render function

**Files:**
- Create: `crates/gg-core/src/stack_nav.rs`
- Modify: `crates/gg-core/src/lib.rs`

Pure, I/O-free module. Heavily unit tested. Produces the comment body we will later post to providers.

- [ ] **Step 1.1: Register the new module**

Modify `crates/gg-core/src/lib.rs` — add a `pub mod stack_nav;` line after the other module declarations (alphabetical order: between `provider` and `stack`). Resulting module list:

```rust
pub mod commands;
pub mod config;
pub mod context;
pub mod error;
pub mod gh;
pub mod git;
pub mod glab;
pub mod managed_body;
pub mod output;
pub mod provider;
pub mod stack;
pub mod stack_nav;
pub mod template;
```

- [ ] **Step 1.2: Write the first failing test**

Create `crates/gg-core/src/stack_nav.rs` with just the test skeleton:

```rust
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
    todo!("implemented in next step")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_two_entries_current_first_github() {
        let entries = vec![
            StackNavEntry { pr_number: 42, is_current: true },
            StackNavEntry { pr_number: 43, is_current: false },
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
```

- [ ] **Step 1.3: Run the test, confirm it fails**

Run: `cargo test -p gg-core stack_nav::`
Expected: test panics with `not yet implemented` from the `todo!()`.

- [ ] **Step 1.4: Implement `render`**

Replace the `todo!()` body of `render`:

```rust
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
    out.push_str(
        "\n<sub>Managed by [git-gud](https://github.com/mrmans0n/git-gud).</sub>\n",
    );
    out.push_str(MARKER);
    out
}
```

- [ ] **Step 1.5: Run the test, confirm it passes**

Run: `cargo test -p gg-core stack_nav::test_render_two_entries_current_first_github`
Expected: PASS.

- [ ] **Step 1.6: Commit**

```bash
git add crates/gg-core/src/stack_nav.rs crates/gg-core/src/lib.rs
git commit -m "feat(core): add stack_nav render function for nav comments"
```

---

## Task 2: Add full test coverage for `render`

**Files:**
- Modify: `crates/gg-core/src/stack_nav.rs`

- [ ] **Step 2.1: Add remaining render tests**

Add the following tests inside the existing `mod tests` block in `crates/gg-core/src/stack_nav.rs`:

```rust
#[test]
fn test_render_three_entries_current_middle_github() {
    let entries = vec![
        StackNavEntry { pr_number: 42, is_current: false },
        StackNavEntry { pr_number: 43, is_current: true },
        StackNavEntry { pr_number: 44, is_current: false },
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
        StackNavEntry { pr_number: 10, is_current: false },
        StackNavEntry { pr_number: 11, is_current: false },
        StackNavEntry { pr_number: 12, is_current: true },
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
        StackNavEntry { pr_number: 1, is_current: true },
        StackNavEntry { pr_number: 2, is_current: false },
    ];
    let body = render("s", &entries, "!");
    assert!(body.contains("- 👉 !1\n"));
    assert!(body.contains("- !2\n"));
    assert!(!body.contains('#'));
}

#[test]
fn test_render_is_idempotent() {
    let entries = vec![
        StackNavEntry { pr_number: 1, is_current: true },
        StackNavEntry { pr_number: 2, is_current: false },
    ];
    let a = render("s", &entries, "#");
    let b = render("s", &entries, "#");
    assert_eq!(a, b);
}

#[test]
fn test_render_includes_stack_name_backticked() {
    let entries = vec![
        StackNavEntry { pr_number: 1, is_current: true },
        StackNavEntry { pr_number: 2, is_current: false },
    ];
    let body = render("my-stack", &entries, "#");
    assert!(body.contains("`my-stack`"));
}

#[test]
fn test_render_ends_with_marker() {
    let entries = vec![
        StackNavEntry { pr_number: 1, is_current: true },
        StackNavEntry { pr_number: 2, is_current: false },
    ];
    let body = render("s", &entries, "#");
    assert!(body.ends_with(MARKER));
}

#[test]
fn test_render_includes_attribution_footer() {
    let entries = vec![
        StackNavEntry { pr_number: 1, is_current: true },
        StackNavEntry { pr_number: 2, is_current: false },
    ];
    let body = render("s", &entries, "#");
    assert!(body.contains("<sub>Managed by [git-gud]"));
}
```

- [ ] **Step 2.2: Run the tests, confirm they all pass**

Run: `cargo test -p gg-core stack_nav::`
Expected: all 7 render tests PASS.

- [ ] **Step 2.3: Commit**

```bash
git add crates/gg-core/src/stack_nav.rs
git commit -m "test(core): add render coverage for stack_nav"
```

---

## Task 3: Add `is_managed_comment` marker detection

**Files:**
- Modify: `crates/gg-core/src/stack_nav.rs`

- [ ] **Step 3.1: Add failing test for `is_managed_comment`**

Append to the `mod tests` block in `crates/gg-core/src/stack_nav.rs`:

```rust
#[test]
fn test_is_managed_comment_with_marker() {
    let body = "some text\n<!-- gg:stack-nav -->";
    assert!(is_managed_comment(body));
}

#[test]
fn test_is_managed_comment_without_marker() {
    let body = "a user comment with no markers";
    assert!(!is_managed_comment(body));
}

#[test]
fn test_is_managed_comment_with_trailing_whitespace_after_marker() {
    let body = "body\n<!-- gg:stack-nav -->   \n";
    assert!(is_managed_comment(body));
}

#[test]
fn test_is_managed_comment_empty_body() {
    assert!(!is_managed_comment(""));
}
```

- [ ] **Step 3.2: Run tests, confirm compile error (function not yet defined)**

Run: `cargo test -p gg-core stack_nav::is_managed_comment`
Expected: compile error — `cannot find function is_managed_comment`.

- [ ] **Step 3.3: Implement `is_managed_comment`**

Add above the `#[cfg(test)] mod tests {` block in `crates/gg-core/src/stack_nav.rs`:

```rust
/// Returns true if `body` contains the managed-comment marker.
///
/// Used to identify git-gud-managed nav comments among arbitrary PR comments
/// when we need to find our own comment to update or delete it.
pub fn is_managed_comment(body: &str) -> bool {
    body.contains(MARKER)
}
```

- [ ] **Step 3.4: Run tests, confirm all pass**

Run: `cargo test -p gg-core stack_nav::`
Expected: all stack_nav tests PASS (11 total).

- [ ] **Step 3.5: Commit**

```bash
git add crates/gg-core/src/stack_nav.rs
git commit -m "feat(core): add is_managed_comment marker detection"
```

---

## Task 4: Add `stack_nav_comments` config field

**Files:**
- Modify: `crates/gg-core/src/config.rs`

- [ ] **Step 4.1: Write failing test for the new config default**

Look at the end of `crates/gg-core/src/config.rs` for the `#[cfg(test)] mod tests` block. Find `fn test_defaults_default()` or the equivalent test function; if none exists, append a new one. Add this test (place it near the end of the `mod tests { ... }` block in `config.rs`):

```rust
#[test]
fn test_stack_nav_comments_defaults_to_false() {
    let defaults = Defaults::default();
    assert!(!defaults.stack_nav_comments, "should default to false (opt-in)");
}

#[test]
fn test_stack_nav_comments_round_trips_through_json() {
    let mut config = Config::default();
    config.defaults.stack_nav_comments = true;
    let json = serde_json::to_string(&config).unwrap();
    let parsed: Config = serde_json::from_str(&json).unwrap();
    assert!(parsed.defaults.stack_nav_comments);
}

#[test]
fn test_stack_nav_comments_missing_field_loads_as_false() {
    // Existing configs without the field must continue to deserialize.
    let json = r#"{"defaults":{}}"#;
    let parsed: Config = serde_json::from_str(json).unwrap();
    assert!(!parsed.defaults.stack_nav_comments);
}
```

- [ ] **Step 4.2: Run tests, confirm compile error**

Run: `cargo test -p gg-core config::tests::test_stack_nav_comments_defaults_to_false`
Expected: compile error — no field `stack_nav_comments` on `Defaults`.

- [ ] **Step 4.3: Add the field and default**

In `crates/gg-core/src/config.rs`:

Add the field to the `Defaults` struct — insert after `pub sync_update_descriptions: bool,` (around line 76):

```rust
    /// Post and maintain a managed navigation comment on each PR/MR in a
    /// multi-entry stack. Default: false (opt-in).
    #[serde(default)]
    pub stack_nav_comments: bool,
```

Add the initializer to the `impl Default for Defaults` block — insert after `sync_update_descriptions: true,` (around line 121):

```rust
            stack_nav_comments: false,
```

- [ ] **Step 4.4: Add a getter helper**

Still in `crates/gg-core/src/config.rs`, search for existing getters like `get_sync_update_descriptions`. They are defined on `impl Config` using the pattern of falling back through global config. Find that block and add:

```rust
    /// Whether to post and maintain stack-navigation comments on PRs/MRs.
    pub fn get_stack_nav_comments(&self) -> bool {
        self.defaults.stack_nav_comments
    }
```

Place this method next to `get_sync_update_descriptions` or similar sync-related getters. If `get_sync_update_descriptions` does anything more complex (e.g., consults a global fallback), match its shape.

- [ ] **Step 4.5: Run tests, confirm they pass**

Run: `cargo test -p gg-core config::tests::test_stack_nav_comments`
Expected: all 3 new tests PASS.

- [ ] **Step 4.6: Commit**

```bash
git add crates/gg-core/src/config.rs
git commit -m "feat(core): add stack_nav_comments config field (default false)"
```

---

## Task 5: Add GitHub comment helpers in `gh.rs`

**Files:**
- Modify: `crates/gg-core/src/gh.rs`

GitHub PR comments are "issue comments" in the API (they appear on the Conversation tab). We shell out to `gh api` with explicit endpoints.

- [ ] **Step 5.1: Add failing compile-check test**

Add this to the `#[cfg(test)] mod tests` block at the bottom of `crates/gg-core/src/gh.rs`. If the file has no tests module, create one:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comment_helpers_exist() {
        // Compile-only test; ensures the new functions are wired up.
        // Real invocations require a live gh CLI and are tested manually / in CI.
        let _: fn(u64) -> Result<Vec<IssueComment>> = list_issue_comments;
        let _: fn(u64, &str) -> Result<()> = create_issue_comment;
        let _: fn(u64, &str) -> Result<()> = update_issue_comment;
        let _: fn(u64) -> Result<()> = delete_issue_comment;
    }
}
```

- [ ] **Step 5.2: Run tests, confirm compile error**

Run: `cargo test -p gg-core gh::tests::test_comment_helpers_exist`
Expected: compile error — none of the four functions or `IssueComment` exist yet.

- [ ] **Step 5.3: Add the `IssueComment` struct and helpers**

Append to the non-test portion of `crates/gg-core/src/gh.rs` (above the tests module):

```rust
/// A GitHub issue comment (which includes PR comments on the Conversation tab).
#[derive(Debug, Clone, Deserialize)]
pub struct IssueComment {
    pub id: u64,
    pub body: String,
}

/// List all comments on a PR (issue comments, i.e. Conversation-tab comments).
///
/// Paginates across 100-per-page responses until exhausted.
pub fn list_issue_comments(pr_number: u64) -> Result<Vec<IssueComment>> {
    let endpoint = format!(
        "repos/{{owner}}/{{repo}}/issues/{}/comments?per_page=100",
        pr_number
    );
    let output = Command::new("gh")
        .args(["api", "--paginate", &endpoint])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to list comments for PR #{}: {}",
            pr_number, stderr
        )));
    }

    // With --paginate, gh concatenates JSON arrays. Parse as one Vec.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let comments: Vec<IssueComment> = serde_json::from_str(&stdout).map_err(|e| {
        GgError::Other(format!(
            "Failed to parse comments JSON for PR #{}: {}",
            pr_number, e
        ))
    })?;
    Ok(comments)
}

/// Post a new comment on a PR.
pub fn create_issue_comment(pr_number: u64, body: &str) -> Result<()> {
    let endpoint = format!("repos/{{owner}}/{{repo}}/issues/{}/comments", pr_number);
    let output = Command::new("gh")
        .args([
            "api",
            "-X",
            "POST",
            &endpoint,
            "-f",
            &format!("body={}", body),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to create comment on PR #{}: {}",
            pr_number, stderr
        )));
    }
    Ok(())
}

/// Edit an existing PR comment by its comment id.
pub fn update_issue_comment(comment_id: u64, body: &str) -> Result<()> {
    let endpoint = format!("repos/{{owner}}/{{repo}}/issues/comments/{}", comment_id);
    let output = Command::new("gh")
        .args([
            "api",
            "-X",
            "PATCH",
            &endpoint,
            "-f",
            &format!("body={}", body),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to update comment {}: {}",
            comment_id, stderr
        )));
    }
    Ok(())
}

/// Delete a PR comment by its comment id.
pub fn delete_issue_comment(comment_id: u64) -> Result<()> {
    let endpoint = format!("repos/{{owner}}/{{repo}}/issues/comments/{}", comment_id);
    let output = Command::new("gh")
        .args(["api", "-X", "DELETE", &endpoint])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::Other(format!(
            "Failed to delete comment {}: {}",
            comment_id, stderr
        )));
    }
    Ok(())
}
```

> Note: the `{owner}` / `{repo}` placeholders are expanded by `gh api` from the current repo context, identical to the pattern used elsewhere in this file.

Fix the test signatures if the sketch in Step 5.1 used `fn(u64, &str)` for `update_issue_comment` — the real signature is `fn(u64, &str) -> Result<()>` where the first `u64` is the comment id, not the PR number. Update the test:

```rust
    let _: fn(u64) -> Result<Vec<IssueComment>> = list_issue_comments;
    let _: fn(u64, &str) -> Result<()> = create_issue_comment;
    let _: fn(u64, &str) -> Result<()> = update_issue_comment;
    let _: fn(u64) -> Result<()> = delete_issue_comment;
```

- [ ] **Step 5.4: Run tests**

Run: `cargo test -p gg-core gh::tests::test_comment_helpers_exist`
Expected: PASS (compile-only smoke test).

- [ ] **Step 5.5: Commit**

```bash
git add crates/gg-core/src/gh.rs
git commit -m "feat(core): add GitHub issue-comment CRUD helpers for nav comments"
```

---

## Task 6: Add GitLab note helpers in `glab.rs`

**Files:**
- Modify: `crates/gg-core/src/glab.rs`

GitLab's equivalent of PR comments is "notes." Shell out to `glab api`.

- [ ] **Step 6.1: Add failing compile-check test**

Append to the existing `#[cfg(test)] mod tests` block in `crates/gg-core/src/glab.rs`:

```rust
#[test]
fn test_note_helpers_exist() {
    // Compile-only; real API calls tested manually.
    let _: fn(u64) -> Result<Vec<MrNote>> = list_mr_notes;
    let _: fn(u64, &str) -> Result<()> = create_mr_note;
    let _: fn(u64, u64, &str) -> Result<()> = update_mr_note;
    let _: fn(u64, u64) -> Result<()> = delete_mr_note;
}
```

- [ ] **Step 6.2: Run tests, confirm compile error**

Run: `cargo test -p gg-core glab::tests::test_note_helpers_exist`
Expected: compile error — none of those items exist.

- [ ] **Step 6.3: Add the `MrNote` struct and helpers**

Append to the non-test portion of `crates/gg-core/src/glab.rs`:

```rust
/// A GitLab MR note (a discussion entry on the merge request).
#[derive(Debug, Clone, Deserialize)]
pub struct MrNote {
    pub id: u64,
    pub body: String,
}

/// Get the project identifier for `glab api` calls (URL-encoded `owner/repo`).
///
/// `glab api` accepts `:fullpath` as a shorthand for the current repo's
/// `projects/<encoded>` prefix, matching how `glab` resolves the remote.
fn glab_project_prefix() -> &'static str {
    // `:fullpath` is a glab-expanded template that resolves to the current repo.
    // It works anywhere `glab api` accepts a path.
    ":fullpath"
}

/// List all notes on an MR.
///
/// Note IDs are needed to update or delete individual notes.
pub fn list_mr_notes(mr_iid: u64) -> Result<Vec<MrNote>> {
    let endpoint = format!(
        "projects/{}/merge_requests/{}/notes?per_page=100",
        glab_project_prefix(),
        mr_iid
    );
    let output = Command::new("glab")
        .args(["api", "--paginate", &endpoint])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::GlabError(format!(
            "Failed to list notes for MR !{}: {}",
            mr_iid, stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let notes: Vec<MrNote> = serde_json::from_str(&stdout).map_err(|e| {
        GgError::GlabError(format!(
            "Failed to parse notes JSON for MR !{}: {}",
            mr_iid, e
        ))
    })?;
    Ok(notes)
}

/// Create a new note on an MR.
pub fn create_mr_note(mr_iid: u64, body: &str) -> Result<()> {
    let endpoint = format!(
        "projects/{}/merge_requests/{}/notes",
        glab_project_prefix(),
        mr_iid
    );
    let output = Command::new("glab")
        .args([
            "api",
            "-X",
            "POST",
            &endpoint,
            "-f",
            &format!("body={}", body),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::GlabError(format!(
            "Failed to create note on MR !{}: {}",
            mr_iid, stderr
        )));
    }
    Ok(())
}

/// Update an existing note on an MR.
pub fn update_mr_note(mr_iid: u64, note_id: u64, body: &str) -> Result<()> {
    let endpoint = format!(
        "projects/{}/merge_requests/{}/notes/{}",
        glab_project_prefix(),
        mr_iid,
        note_id
    );
    let output = Command::new("glab")
        .args([
            "api",
            "-X",
            "PUT",
            &endpoint,
            "-f",
            &format!("body={}", body),
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::GlabError(format!(
            "Failed to update note {} on MR !{}: {}",
            note_id, mr_iid, stderr
        )));
    }
    Ok(())
}

/// Delete a note from an MR.
pub fn delete_mr_note(mr_iid: u64, note_id: u64) -> Result<()> {
    let endpoint = format!(
        "projects/{}/merge_requests/{}/notes/{}",
        glab_project_prefix(),
        mr_iid,
        note_id
    );
    let output = Command::new("glab")
        .args(["api", "-X", "DELETE", &endpoint])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GgError::GlabError(format!(
            "Failed to delete note {} on MR !{}: {}",
            note_id, mr_iid, stderr
        )));
    }
    Ok(())
}
```

- [ ] **Step 6.4: Run tests, confirm they pass**

Run: `cargo test -p gg-core glab::tests::test_note_helpers_exist`
Expected: PASS.

- [ ] **Step 6.5: Commit**

```bash
git add crates/gg-core/src/glab.rs
git commit -m "feat(core): add GitLab MR-note CRUD helpers for nav comments"
```

---

## Task 7: Expose comment operations through the `Provider` abstraction

**Files:**
- Modify: `crates/gg-core/src/provider.rs`

Add a `ManagedComment` struct and four methods on `Provider` that dispatch to the gh/glab helpers. This is the unified surface callers (sync.rs) will use.

- [ ] **Step 7.1: Write failing unit tests**

Add the following tests to the `#[cfg(test)] mod tests` block at the bottom of `crates/gg-core/src/provider.rs` (after the existing tests):

```rust
#[test]
fn test_managed_comment_construction() {
    let comment = ManagedComment {
        id: 99,
        body: "hello\n<!-- gg:stack-nav -->".to_string(),
    };
    assert_eq!(comment.id, 99);
    assert!(comment.body.contains("<!-- gg:stack-nav -->"));
}
```

- [ ] **Step 7.2: Run the test, confirm compile error**

Run: `cargo test -p gg-core provider::tests::test_managed_comment_construction`
Expected: compile error — `ManagedComment` does not exist.

- [ ] **Step 7.3: Add `ManagedComment` and the four methods**

In `crates/gg-core/src/provider.rs`, find the `PrCreationResult` struct (around line 72) and add immediately after it:

```rust
/// A PR/MR comment that git-gud manages (identified by marker).
#[derive(Debug, Clone)]
pub struct ManagedComment {
    pub id: u64,
    pub body: String,
}
```

Then, inside the `impl Provider` block — place these near `get_pr_body` / `update_pr_description` for locality — add four new methods:

```rust
    /// Find the first comment on a PR/MR whose body contains `marker`.
    ///
    /// Returns `Ok(None)` if no such comment exists.
    pub fn find_managed_comment(
        &self,
        pr_number: u64,
        marker: &str,
    ) -> Result<Option<ManagedComment>> {
        match self {
            Provider::GitHub => {
                let comments = gh::list_issue_comments(pr_number)?;
                Ok(comments
                    .into_iter()
                    .find(|c| c.body.contains(marker))
                    .map(|c| ManagedComment {
                        id: c.id,
                        body: c.body,
                    }))
            }
            Provider::GitLab => {
                let notes = glab::list_mr_notes(pr_number)?;
                Ok(notes
                    .into_iter()
                    .find(|n| n.body.contains(marker))
                    .map(|n| ManagedComment {
                        id: n.id,
                        body: n.body,
                    }))
            }
        }
    }

    /// Create a comment on a PR/MR.
    pub fn create_pr_comment(&self, pr_number: u64, body: &str) -> Result<()> {
        match self {
            Provider::GitHub => gh::create_issue_comment(pr_number, body),
            Provider::GitLab => glab::create_mr_note(pr_number, body),
        }
    }

    /// Update an existing comment by its id.
    ///
    /// `pr_number` is required for GitLab (notes are per-MR); GitHub ignores it
    /// because issue-comment endpoints address comments by id alone.
    pub fn update_pr_comment(
        &self,
        pr_number: u64,
        comment_id: u64,
        body: &str,
    ) -> Result<()> {
        match self {
            Provider::GitHub => gh::update_issue_comment(comment_id, body),
            Provider::GitLab => glab::update_mr_note(pr_number, comment_id, body),
        }
    }

    /// Delete a comment by its id.
    pub fn delete_pr_comment(&self, pr_number: u64, comment_id: u64) -> Result<()> {
        match self {
            Provider::GitHub => gh::delete_issue_comment(comment_id),
            Provider::GitLab => glab::delete_mr_note(pr_number, comment_id),
        }
    }
```

- [ ] **Step 7.4: Run tests, confirm they pass**

Run: `cargo test -p gg-core provider::tests::`
Expected: all provider tests PASS, including the new `test_managed_comment_construction`.

- [ ] **Step 7.5: Commit**

```bash
git add crates/gg-core/src/provider.rs
git commit -m "feat(core): add Provider::{find,create,update,delete}_pr_comment"
```

---

## Task 8: Add `nav_comment_action` optional field to sync JSON output

**Files:**
- Modify: `crates/gg-core/src/output.rs`

The field is `Option<String>` — when omitted from serialized output, existing JSON consumers are unaffected for users who don't opt in.

- [ ] **Step 8.1: Write failing test**

Append to the `#[cfg(test)] mod tests` block in `crates/gg-core/src/output.rs`:

```rust
#[test]
fn test_sync_entry_nav_comment_action_omitted_when_none() {
    let entry = SyncEntryResultJson {
        position: 1,
        sha: "abc".to_string(),
        title: "t".to_string(),
        gg_id: "c-1234567".to_string(),
        branch: "b".to_string(),
        action: "created".to_string(),
        pr_number: Some(1),
        pr_url: None,
        draft: false,
        pushed: true,
        error: None,
        nav_comment_action: None,
    };
    let json = serde_json::to_value(&entry).unwrap();
    assert!(
        json.get("nav_comment_action").is_none(),
        "field should be omitted when None"
    );
}

#[test]
fn test_sync_entry_nav_comment_action_serializes_when_some() {
    let entry = SyncEntryResultJson {
        position: 1,
        sha: "abc".to_string(),
        title: "t".to_string(),
        gg_id: "c-1234567".to_string(),
        branch: "b".to_string(),
        action: "created".to_string(),
        pr_number: Some(1),
        pr_url: None,
        draft: false,
        pushed: true,
        error: None,
        nav_comment_action: Some("created".to_string()),
    };
    let json = serde_json::to_value(&entry).unwrap();
    assert_eq!(json["nav_comment_action"], "created");
}
```

- [ ] **Step 8.2: Run, confirm compile error**

Run: `cargo test -p gg-core output::tests::test_sync_entry_nav_comment_action_omitted_when_none`
Expected: compile error — missing field `nav_comment_action`.

- [ ] **Step 8.3: Add the field**

In `crates/gg-core/src/output.rs`, modify the `SyncEntryResultJson` struct (around line 122):

```rust
#[derive(Serialize)]
pub struct SyncEntryResultJson {
    pub position: usize,
    pub sha: String,
    pub title: String,
    pub gg_id: String,
    pub branch: String,
    pub action: String,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
    pub draft: bool,
    pub pushed: bool,
    pub error: Option<String>,
    /// Optional: action taken on the managed nav comment for this entry's PR.
    /// One of "created", "updated", "unchanged", "deleted", "skipped", "error".
    /// Omitted when the feature is disabled and no cleanup was required.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nav_comment_action: Option<String>,
}
```

- [ ] **Step 8.4: Fix call sites that construct this struct**

The sync command constructs `SyncEntryResultJson` in two places. Update them to pass `nav_comment_action: None` explicitly (sync.rs will populate this in Task 9). In `crates/gg-core/src/commands/sync.rs`, find each `json_entries.push(SyncEntryResultJson { ... })` call and add the new field with the value `None`:

```rust
        json_entries.push(SyncEntryResultJson {
            position: entry.position,
            sha: entry.short_sha.clone(),
            title: entry.title.clone(),
            gg_id: gg_id.clone(),
            branch: entry_branch,
            action,
            pr_number,
            pr_url,
            draft: entry_draft,
            pushed,
            error: entry_error,
            nav_comment_action: None,
        });
```

There are two such construction sites (one on the `push` error path, one at the end of the loop). Both need the new field.

- [ ] **Step 8.5: Run tests, confirm they pass**

Run: `cargo test -p gg-core output::tests::`
Expected: PASS, including the 2 new tests.

Also run: `cargo build -p gg-core` — should compile without errors.

- [ ] **Step 8.6: Commit**

```bash
git add crates/gg-core/src/output.rs crates/gg-core/src/commands/sync.rs
git commit -m "feat(core): add optional nav_comment_action to sync JSON output"
```

---

## Task 9: Add pure helper for deciding per-entry nav action

**Files:**
- Create: (none — add to `crates/gg-core/src/stack_nav.rs`)

The decision logic ("given state, what should we do for each entry?") is pure and easy to test. Keep it out of sync.rs.

- [ ] **Step 9.1: Write failing tests for the decision helper**

Append to the `mod tests` block in `crates/gg-core/src/stack_nav.rs`:

```rust
#[test]
fn test_decide_action_reconcile_when_setting_on_and_multi_entry_open() {
    let decision = decide_action(NavDecisionInput {
        setting_enabled: true,
        stack_entry_count: 3,
        pr_state: PrEntryState::Open,
        has_existing_comment: false,
    });
    assert_eq!(decision, NavAction::Upsert);
}

#[test]
fn test_decide_action_cleanup_when_setting_off_and_comment_exists() {
    let decision = decide_action(NavDecisionInput {
        setting_enabled: false,
        stack_entry_count: 3,
        pr_state: PrEntryState::Open,
        has_existing_comment: true,
    });
    assert_eq!(decision, NavAction::Delete);
}

#[test]
fn test_decide_action_skip_when_setting_off_and_no_comment() {
    let decision = decide_action(NavDecisionInput {
        setting_enabled: false,
        stack_entry_count: 3,
        pr_state: PrEntryState::Open,
        has_existing_comment: false,
    });
    assert_eq!(decision, NavAction::Skip);
}

#[test]
fn test_decide_action_cleanup_when_single_entry_and_comment_exists() {
    let decision = decide_action(NavDecisionInput {
        setting_enabled: true,
        stack_entry_count: 1,
        pr_state: PrEntryState::Open,
        has_existing_comment: true,
    });
    assert_eq!(decision, NavAction::Delete);
}

#[test]
fn test_decide_action_skip_when_single_entry_and_no_comment() {
    let decision = decide_action(NavDecisionInput {
        setting_enabled: true,
        stack_entry_count: 1,
        pr_state: PrEntryState::Open,
        has_existing_comment: false,
    });
    assert_eq!(decision, NavAction::Skip);
}

#[test]
fn test_decide_action_skip_when_pr_closed() {
    // Closed / merged PRs are historical — never touch their comments.
    for state in [PrEntryState::Merged, PrEntryState::Closed] {
        let decision = decide_action(NavDecisionInput {
            setting_enabled: true,
            stack_entry_count: 3,
            pr_state: state,
            has_existing_comment: true,
        });
        assert_eq!(decision, NavAction::Skip, "closed/merged must be skipped");
    }
}

#[test]
fn test_decide_action_draft_treated_as_open() {
    let decision = decide_action(NavDecisionInput {
        setting_enabled: true,
        stack_entry_count: 2,
        pr_state: PrEntryState::Draft,
        has_existing_comment: false,
    });
    assert_eq!(decision, NavAction::Upsert);
}
```

- [ ] **Step 9.2: Run tests, confirm compile errors**

Run: `cargo test -p gg-core stack_nav::tests::test_decide_action_reconcile_when_setting_on_and_multi_entry_open`
Expected: compile errors — types and function don't exist.

- [ ] **Step 9.3: Implement the decision helper**

Add to `crates/gg-core/src/stack_nav.rs` (above the tests module):

```rust
/// The per-entry PR state that matters for nav-comment reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrEntryState {
    Open,
    Draft,
    Merged,
    Closed,
}

/// Inputs for the per-entry nav-action decision.
#[derive(Debug, Clone, Copy)]
pub struct NavDecisionInput {
    pub setting_enabled: bool,
    pub stack_entry_count: usize,
    pub pr_state: PrEntryState,
    pub has_existing_comment: bool,
}

/// What to do with the nav comment on a single PR in the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavAction {
    /// Do nothing: no comment should exist and none does.
    Skip,
    /// Create a new comment, or update the existing one (idempotent upsert).
    Upsert,
    /// Delete an existing comment.
    Delete,
}

/// Decide what to do with the nav comment on a single PR, based on state.
///
/// See the design spec for the full decision table. In short:
/// - Closed/merged PRs: always skip.
/// - Setting off OR single-entry stack: delete if a comment exists, else skip.
/// - Otherwise: upsert.
pub fn decide_action(input: NavDecisionInput) -> NavAction {
    // Historical PRs are never touched.
    if matches!(input.pr_state, PrEntryState::Merged | PrEntryState::Closed) {
        return NavAction::Skip;
    }

    let should_have_comment = input.setting_enabled && input.stack_entry_count >= 2;

    if should_have_comment {
        NavAction::Upsert
    } else if input.has_existing_comment {
        NavAction::Delete
    } else {
        NavAction::Skip
    }
}
```

- [ ] **Step 9.4: Run tests, confirm all pass**

Run: `cargo test -p gg-core stack_nav::`
Expected: all stack_nav tests PASS (11 render + 4 is_managed + 7 decide_action = 22).

- [ ] **Step 9.5: Commit**

```bash
git add crates/gg-core/src/stack_nav.rs
git commit -m "feat(core): add NavAction decision helper for sync reconcile"
```

---

## Task 10: Integrate nav reconcile into `gg sync`

**Files:**
- Modify: `crates/gg-core/src/commands/sync.rs`

Hook the nav reconcile into the existing sync flow. Track per-entry PR numbers/states during the main loop, then run a single reconcile pass over the open PRs after the main loop finishes.

- [ ] **Step 10.1: Add a small record type at the top of the module**

At the top of `crates/gg-core/src/commands/sync.rs`, after the `use` statements, add:

```rust
/// Per-entry state captured during the main sync loop that the nav-comment
/// reconcile pass needs. Populated only for entries whose PR exists.
struct NavEntrySnapshot {
    pr_number: u64,
    pr_state: crate::stack_nav::PrEntryState,
    /// Index into `json_entries` so we can attach the nav action result.
    json_index: usize,
}
```

Add `use crate::stack_nav;` to the import block at the top of the file.

- [ ] **Step 10.2: Collect snapshots during the main loop**

Find the `for (i, entry) in entries_to_sync.iter().enumerate()` loop (around line 332). Inside this loop, after the `match existing_pr { ... }` block that sets `pr_number`, and **before** the `if json { json_entries.push(...) }` block at the end of the iteration, insert state collection:

```rust
        // Capture state for nav reconcile pass (below).
        let nav_snapshot: Option<NavEntrySnapshot> = if let Some(num) = pr_number {
            let state = match provider.get_pr_info(num).map(|info| info.state).ok() {
                Some(crate::provider::PrState::Open) => Some(stack_nav::PrEntryState::Open),
                Some(crate::provider::PrState::Draft) => Some(stack_nav::PrEntryState::Draft),
                Some(crate::provider::PrState::Merged) => Some(stack_nav::PrEntryState::Merged),
                Some(crate::provider::PrState::Closed) => Some(stack_nav::PrEntryState::Closed),
                None => None,
            };
            state.map(|pr_state| NavEntrySnapshot {
                pr_number: num,
                pr_state,
                json_index: json_entries.len(), // the push below will be at this index
            })
        } else {
            None
        };
```

Then hoist the snapshot up to a `Vec<Option<NavEntrySnapshot>>` defined above the loop. Specifically, add this declaration next to `let mut json_entries: Vec<SyncEntryResultJson> = Vec::new();`:

```rust
    let mut nav_snapshots: Vec<Option<NavEntrySnapshot>> = Vec::new();
```

And at the end of each iteration, push the snapshot. Right before `pb.inc(1);` at the end of the loop, add:

```rust
        nav_snapshots.push(nav_snapshot);
```

> **Note:** `get_pr_info` makes an extra network call per entry. This is acceptable for v1 — the reconcile pass depends on per-entry state. If this proves slow in practice, v2 can thread state through the main loop (which already calls `get_pr_info` in the `existing_pr: Some(...)` branch).

- [ ] **Step 10.3: Add the reconcile pass after the main loop**

After the main loop closes (after `pb.finish_with_message("Done!");`) and **before** `config.save(git_dir)?;`, insert the reconcile block:

```rust
    // --- Nav-comment reconcile pass ---
    //
    // For each synced entry whose PR exists and is reachable, decide whether
    // to create/update/delete the managed nav comment based on:
    //   - the stack_nav_comments setting
    //   - the total stack size
    //   - the PR's state (open/draft vs merged/closed)
    //
    // We render the nav body with `is_current = true` on the entry being
    // processed, so each PR's comment highlights the reader's location.
    let setting_enabled = config.get_stack_nav_comments();
    let stack_entry_count = entries_to_sync.len();
    let number_prefix = provider.pr_number_prefix();

    // Collect the (pr_number, is_current) pairs once — used to render each
    // per-PR body with a different `is_current` flag.
    let all_entries: Vec<(u64, usize)> = nav_snapshots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.as_ref().map(|snap| (snap.pr_number, i)))
        .collect();

    for (i, snap) in nav_snapshots.iter().enumerate() {
        let snap = match snap {
            Some(s) => s,
            None => continue,
        };

        // Check for an existing managed comment once per PR.
        let existing = match provider
            .find_managed_comment(snap.pr_number, stack_nav::MARKER)
        {
            Ok(v) => v,
            Err(e) => {
                if !json {
                    println!(
                        "{} Could not list comments on {} {}{}: {}",
                        style("Warning:").yellow(),
                        provider.pr_label(),
                        number_prefix,
                        snap.pr_number,
                        e
                    );
                }
                if json {
                    if let Some(entry_json) = json_entries.get_mut(snap.json_index) {
                        entry_json.nav_comment_action = Some("error".to_string());
                    }
                }
                continue;
            }
        };

        let decision = stack_nav::decide_action(stack_nav::NavDecisionInput {
            setting_enabled,
            stack_entry_count,
            pr_state: snap.pr_state,
            has_existing_comment: existing.is_some(),
        });

        let action_result: Option<&str> = match decision {
            stack_nav::NavAction::Skip => None,
            stack_nav::NavAction::Upsert => {
                // Render body with `is_current = true` for this entry's position.
                let nav_entries: Vec<stack_nav::StackNavEntry> = all_entries
                    .iter()
                    .map(|(n, j)| stack_nav::StackNavEntry {
                        pr_number: *n,
                        is_current: *j == i,
                    })
                    .collect();
                let body = stack_nav::render(&stack.name, &nav_entries, number_prefix);

                match existing {
                    Some(c) if c.body == body => Some("unchanged"),
                    Some(c) => match provider.update_pr_comment(snap.pr_number, c.id, &body) {
                        Ok(()) => Some("updated"),
                        Err(e) => {
                            if !json {
                                println!(
                                    "{} Could not update nav comment on {} {}{}: {}",
                                    style("Warning:").yellow(),
                                    provider.pr_label(),
                                    number_prefix,
                                    snap.pr_number,
                                    e
                                );
                            }
                            Some("error")
                        }
                    },
                    None => match provider.create_pr_comment(snap.pr_number, &body) {
                        Ok(()) => Some("created"),
                        Err(e) => {
                            if !json {
                                println!(
                                    "{} Could not create nav comment on {} {}{}: {}",
                                    style("Warning:").yellow(),
                                    provider.pr_label(),
                                    number_prefix,
                                    snap.pr_number,
                                    e
                                );
                            }
                            Some("error")
                        }
                    },
                }
            }
            stack_nav::NavAction::Delete => {
                match existing {
                    Some(c) => match provider.delete_pr_comment(snap.pr_number, c.id) {
                        Ok(()) => Some("deleted"),
                        Err(e) => {
                            if !json {
                                println!(
                                    "{} Could not delete nav comment on {} {}{}: {}",
                                    style("Warning:").yellow(),
                                    provider.pr_label(),
                                    number_prefix,
                                    snap.pr_number,
                                    e
                                );
                            }
                            Some("error")
                        }
                    },
                    None => None, // decision was Delete but nothing to delete
                }
            }
        };

        if let Some(action) = action_result {
            if let Some(entry_json) = json_entries.get_mut(snap.json_index) {
                entry_json.nav_comment_action = Some(action.to_string());
            }
        }
    }
```

- [ ] **Step 10.4: Build and fix compile errors**

Run: `cargo build -p gg-core`

Fix any compile errors. Common likely issues:
- Missing `use crate::stack_nav;` — add it.
- `existing_pr` already consumed `provider.get_pr_info(...)` earlier but didn't store the state in a local usable after the match. The snapshot code above calls `provider.get_pr_info` again — intentional, since for the `existing_pr: None` branch (newly created PR) we don't have state yet. Confirm the branch where `existing_pr: Some(_)` doesn't move `pr_info` in a way that blocks re-fetching.

Expected final state: `cargo build -p gg-core` succeeds.

- [ ] **Step 10.5: Run the full test suite**

Run: `cargo test -p gg-core`
Expected: all existing tests still PASS.

Run: `cargo fmt --all` and `cargo clippy --all-targets --all-features -- -D warnings` to catch style/lint issues introduced. Fix any clippy warnings with the minimal change.

- [ ] **Step 10.6: Commit**

```bash
git add crates/gg-core/src/commands/sync.rs
git commit -m "feat(sync): reconcile stack-nav comments on PRs/MRs after sync"
```

---

## Task 11: Add `gg setup` prompt for the new setting

**Files:**
- Modify: `crates/gg-core/src/commands/setup.rs`

- [ ] **Step 11.1: Add the prompt in the "Sync" group**

In `crates/gg-core/src/commands/setup.rs`, find the "── Sync ──" block (starting around line 143). Add the new prompt after `sync_update_descriptions` but before the group closes (i.e., immediately after the `sync_update_descriptions = Confirm...` block ending around line 163):

```rust
    defaults.stack_nav_comments = Confirm::with_theme(theme)
        .with_prompt(
            "Post a navigation comment on each PR/MR in a stack (links to other PRs/MRs)?",
        )
        .default(existing.stack_nav_comments)
        .interact()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;
```

- [ ] **Step 11.2: Build**

Run: `cargo build -p gg-core`
Expected: success.

- [ ] **Step 11.3: Commit**

```bash
git add crates/gg-core/src/commands/setup.rs
git commit -m "feat(setup): prompt for stack_nav_comments in full setup mode"
```

---

## Task 12: Update documentation

**Files:**
- Modify: `docs/src/configuration.md`
- Modify: `docs/src/commands/sync.md`
- Modify: `docs/src/commands/setup.md`

- [ ] **Step 12.1: Read the current docs to match existing style**

Read each of these files in full before editing:
- `docs/src/configuration.md`
- `docs/src/commands/sync.md`
- `docs/src/commands/setup.md`

Match the existing voice, header levels, and example formatting.

- [ ] **Step 12.2: Document the config field**

Add to `docs/src/configuration.md` — find the section that documents sync-related settings (look for `sync_update_descriptions`). Add a new entry in the same format:

```markdown
#### `defaults.stack_nav_comments` (boolean, default: `false`)

Opt-in. When `true`, `gg sync` posts a managed comment on each open PR/MR in a
multi-entry stack, listing all entries with a 👉 marker on the current one. The
list uses `#N` references on GitHub and `!N` on GitLab, so the provider renders
titles and status badges automatically.

When the setting is `false` (default), no nav comments are posted. If the
setting was previously `true` and is now `false`, the next `gg sync` removes
any existing managed comments it previously created.

Single-entry stacks are always skipped (a one-item navigation is noise). The
feature is fully managed: git-gud never touches comments it didn't create
(identified by a hidden `<!-- gg:stack-nav -->` marker).
```

- [ ] **Step 12.3: Mention in the sync command reference**

Add a section to `docs/src/commands/sync.md` (place near the end, before any "See also" section). Match the existing heading style:

```markdown
## Stack navigation comments

If `defaults.stack_nav_comments` is enabled in `.git/gg/config.json`, every
`gg sync` reconciles a managed comment on each PR/MR in the stack. The
comment shows all entries in the stack in bottom-up order, with a 👉 marker
on the entry that PR corresponds to — letting reviewers see where they are
in the chain and click through to siblings.

The comment is identified by a hidden HTML marker (`<!-- gg:stack-nav -->`)
and never touches comments git-gud didn't create. Disabling the setting and
re-syncing cleans up any previously-posted comments automatically.

Merged or closed PRs are left alone — `gg sync` never modifies comments on
historical PRs.

When running with `--json`, each entry includes an optional `nav_comment_action`
field (one of `"created"`, `"updated"`, `"unchanged"`, `"deleted"`, `"skipped"`,
`"error"`) when a reconcile decision was made.
```

- [ ] **Step 12.4: Mention in the setup reference**

In `docs/src/commands/setup.md`, find the section listing prompts in full setup mode (search for the "Sync" group or `sync_update_descriptions`). Add a line describing the new prompt, matching existing formatting:

```markdown
- **Post a navigation comment on each PR/MR in a stack?** — sets `defaults.stack_nav_comments`.
  When enabled, each PR/MR in a multi-entry stack gets a managed comment listing
  all entries with a 👉 marker on the current one. Opt-in; default is `no`.
```

- [ ] **Step 12.5: Commit**

```bash
git add docs/src/configuration.md docs/src/commands/sync.md docs/src/commands/setup.md
git commit -m "docs: document stack navigation comment feature"
```

---

## Task 13: Update agent skill and README

**Files:**
- Modify: `skills/gg/SKILL.md`
- Modify: `skills/gg/reference.md`
- Modify: `README.md`

- [ ] **Step 13.1: Update the skill**

Read `skills/gg/SKILL.md` in full first to match the existing tone and structure.

Add a short subsection to `skills/gg/SKILL.md` where other optional sync behaviors are described (search for "sync" or a section discussing PR bodies / managed content). Use this wording:

```markdown
### Stack-navigation comments

If the repo's `.git/gg/config.json` has `defaults.stack_nav_comments: true`,
`gg sync` posts and maintains a managed comment on each open PR/MR in the
stack. The comment lists every entry (`#N` on GitHub, `!N` on GitLab) with a
👉 marker on the current one. The comment is identified by a hidden
`<!-- gg:stack-nav -->` marker and managed entirely by git-gud — don't edit
these comments manually, and don't be surprised when `gg sync` adds, updates,
or removes them automatically.

Disable the feature by setting `defaults.stack_nav_comments: false` (the
default). The next `gg sync` then cleans up any existing managed comments.
```

- [ ] **Step 13.2: Update the skill reference**

Add to `skills/gg/reference.md` where other config options are listed:

```markdown
#### `defaults.stack_nav_comments`

- **Type:** `boolean`
- **Default:** `false`
- **Effect:** When `true`, `gg sync` posts and maintains a managed "stack
  navigation" comment on each open PR/MR in a multi-entry stack. When `false`
  (default), no such comments are posted; any pre-existing managed comments
  are removed on the next sync.
```

Also, find the JSON schema section for `gg sync --json` output (search for `SyncEntryResultJson` or `nav_comment_action` / `pr_number` in the existing schema docs). Add a field description:

```markdown
- `nav_comment_action` (string, optional): action taken on the managed
  stack-nav comment for this entry's PR during this sync. One of
  `"created"`, `"updated"`, `"unchanged"`, `"deleted"`, `"skipped"`, or
  `"error"`. Omitted when no reconcile action was required.
```

- [ ] **Step 13.3: Update the README**

Read `README.md` to find the feature list (usually near the top, under a "Features" heading). Add a line matching existing bullet style:

```markdown
- **Stack navigation comments** — opt-in. Each PR/MR in a stack gets a managed comment listing sibling PRs with a 👉 marker on the current one (GitHub `#N` or GitLab `!N`).
```

- [ ] **Step 13.4: Commit**

```bash
git add skills/gg/SKILL.md skills/gg/reference.md README.md
git commit -m "docs(skill): document stack navigation comment behavior"
```

---

## Task 14: Final verification

**Files:** none — verification only.

- [ ] **Step 14.1: Full format, lint, test sweep**

Run the project's pre-commit checklist:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Expected: all three pass with zero warnings and zero test failures.

- [ ] **Step 14.2: Verify all new tests are discoverable and passing**

Run explicitly:

```bash
cargo test -p gg-core stack_nav::
cargo test -p gg-core config::tests::test_stack_nav_comments
cargo test -p gg-core provider::tests::test_managed_comment_construction
cargo test -p gg-core output::tests::test_sync_entry_nav_comment_action
```

Expected: all pass.

- [ ] **Step 14.3: Manual smoke test (optional but recommended)**

If a test repo with a real remote is available:

1. Set `defaults.stack_nav_comments: true` in `.git/gg/config.json`.
2. Create a two-commit stack.
3. Run `gg sync`.
4. Confirm both PRs/MRs get a stack-nav comment with the expected format.
5. Add a third commit, run `gg sync` again — first two should be updated, third should be created.
6. Toggle the setting to `false`, run `gg sync` — all three comments should be deleted.

Document any surprises in the PR description when opening the PR.

- [ ] **Step 14.4: Commit any final fixes if needed**

If step 14.1 surfaces warnings or failures, fix them as minimally as possible, run the sweep again, and commit. Otherwise, no commit needed for this task.

---

## Self-Review Notes

Spec coverage verified against design doc sections:

- Overview + example → Task 1 (render format matches exactly)
- Opt-in single global bool, default off → Task 4
- Ordering (bottom-up) → Task 1 (callers pass entries in stack order)
- Content (minimal) → Task 1 (no titles, just `#N` / `!N` + 👉)
- Current-entry marker (👉) → Task 1
- Included entries (all) → Task 10 renders entries from `all_entries` regardless of per-PR state
- Single-entry stacks skipped → Task 9 (`decide_action` returns Skip when `stack_entry_count < 2`)
- Closed/merged PRs untouched → Task 9 (first guard in `decide_action`)
- Cleanup when nav shouldn't exist → Task 9 (Delete decision) + Task 10 (reconcile pass handles Delete)
- Independence from `--no-update-descriptions` → Task 10 (reconcile runs unconditionally, not gated by `update_descriptions`)
- `stack_nav.rs` module → Task 1-3, 9
- Config field → Task 4
- Provider methods → Tasks 5, 6, 7
- Two/three-pass sync → Task 10
- JSON field → Task 8
- Tests → Tasks 1, 2, 3, 4, 7, 8, 9
- Docs + skill + README → Tasks 12, 13
