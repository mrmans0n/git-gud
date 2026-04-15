# Stack Navigation Comment Design

**Issue:** [#275 – Feature Request: Add links to other PRs in the stack](https://github.com/mrmans0n/git-gud/issues/275)
**Date:** 2026-04-15

## Overview

When enabled via a global opt-in setting, `gg sync` posts (and keeps updated) a single comment on each open PR/MR in a stack. The comment lists every entry in the stack in bottom-up order, with an emoji marker (👉) on the entry that PR corresponds to, so reviewers can see their position within the stack at a glance and navigate to sibling PRs.

The comment uses `#N` references on GitHub and `!N` on GitLab; the provider's reference expansion handles titles, status badges (open / merged / closed / draft), and click-through navigation automatically.

### Example (GitHub, stack `feat-auth` with three entries viewed from the middle PR)

```
This change is part of the `feat-auth` stack:

- #42
- 👉 #43
- #44

<sub>Managed by [git-gud](https://github.com/mrmans0n/git-gud).</sub>
<!-- gg:stack-nav -->
```

The same stack on GitLab uses `!42`, `!43`, `!44`.

## Motivation

Reviewers viewing a single PR from a stack often have no way to know what else is in flight, where their PR sits in the dependency chain, or where to go next. Graphite, git-spice, and Phabricator all solve this by rendering navigation inline on each review unit. git-gud currently has no equivalent.

## Design Decisions

### Where: a pinned comment per PR, not the description

The navigation lives in a dedicated comment on each PR/MR, not inside the PR body. This keeps the `pr_template.md` and `{{description}}` placeholders entirely separate from the managed navigation concern.

- The description stays author-authored; `gg sync` doesn't need to reconcile nav content against user edits within the managed-body block.
- A hidden HTML marker (`<!-- gg:stack-nav -->`) in the comment body lets `gg sync` find "its" comment across subsequent runs without storing comment IDs in `.git/gg/config.json`.

### Opt-in: single global bool, default off

A new field `stack_nav_comments: bool` (default `false`) on the repository `Config` (`.git/gg/config.json`). `gg setup` gains a prompt to enable it.

No per-stack override in v1 — a single global default matches most expected usage patterns. Per-stack granularity can be added later without breaking changes if demand appears.

### Ordering: bottom-up (base first, tip last)

Entries are listed in the natural order of `stack.entries` (index 0 first). This matches `gg ls` and `git rebase -i`, both of which the project's users are already trained on.

### Content: minimal

Each line is `- #NUMBER` (or `- 👉 #NUMBER` for the current entry). No inline titles, no custom format strings. Relying on the provider's reference rendering:

- Eliminates drift between the rendered nav and the commit title.
- Automatically gets open/merged/closed/draft status badges.
- Keeps the comment body small.

### Current-entry marker: 👉

The pointing-finger emoji reads unambiguously as "this is you" and doesn't collide with checkbox or bullet formatting authors commonly paste into PR descriptions. Fixed, not configurable.

### Included entries: all of them

Every entry in `stack.entries` appears in the list, regardless of its PR state (open / merged / closed / draft). The provider's reference expansion shows the state badge next to each number. `gg clean` eventually removes entries from the stack, so the list shrinks naturally over time.

### Single-entry stacks: skip

If `stack.entries.len() == 1`, no comment is posted (or, if one already exists, it is deleted). A one-item nav is noise. The transition from "no comment" → "comment appears when a second entry is added" is a useful signal.

### Closed/merged PRs: don't touch them

When iterating entries to reconcile comments, skip any entry whose PR is closed or merged — don't create, update, or delete comments on them. Closed PRs are historical artifacts and shouldn't be modified by subsequent syncs. Draft PRs are **not** in this category: they are treated as open and receive nav comments like any other open PR.

### Cleanup when nav shouldn't exist

When the setting is `false` or the stack has shrunk to a single entry, `gg sync` actively removes any existing stack-nav comments (found by marker) from still-open PRs. Toggling the setting off, or a stack naturally collapsing to one commit, leaves no orphaned comments — the feature fully owns its output.

### Independence from `--no-update-descriptions`

The nav comment is reconciled on every sync regardless of the `update_descriptions` flag. Adding a commit to a stack requires updating nav on every OTHER open PR too, even ones whose own body didn't change — so gating nav reconciliation on description updates would produce inconsistent navigation.

## Architecture

### New module: `crates/gg-core/src/stack_nav.rs`

Pure, I/O-free rendering and marker detection. Heavily unit-tested.

```rust
pub const MARKER: &str = "<!-- gg:stack-nav -->";

pub struct StackNavEntry {
    pub pr_number: u64,
    pub is_current: bool,
}

/// Render the navigation comment body.
/// `number_prefix` is "#" for GitHub, "!" for GitLab.
pub fn render(stack_name: &str, entries: &[StackNavEntry], number_prefix: &str) -> String;

/// Returns true if a comment body is a git-gud-managed nav comment.
pub fn is_managed_comment(body: &str) -> bool;
```

### Config changes: `crates/gg-core/src/config.rs`

- Add `stack_nav_comments: bool` field to the `Config` struct, with a serde default of `false` so existing configs load unchanged.

### Provider additions: `crates/gg-core/src/provider.rs`

Four new methods on the provider abstraction, plus a small struct:

```rust
pub struct ManagedComment {
    pub id: u64,
    pub body: String,
}

pub trait ProviderOps { // or equivalent existing trait
    fn find_managed_comment(&self, pr_number: u64, marker: &str) -> Result<Option<ManagedComment>>;
    fn create_pr_comment(&self, pr_number: u64, body: &str) -> Result<()>;
    fn update_pr_comment(&self, pr_number: u64, comment_id: u64, body: &str) -> Result<()>;
    fn delete_pr_comment(&self, pr_number: u64, comment_id: u64) -> Result<()>;
}
```

Implementation details per backend:

- **GitHub (`gh.rs`):** `gh api /repos/{owner}/{repo}/issues/{number}/comments` for listing and creating (PR comments are issue comments in GitHub's data model); `gh api -X PATCH /repos/{owner}/{repo}/issues/comments/{id}` for updates; `gh api -X DELETE` for deletion.
- **GitLab (`glab.rs`):** `glab api /projects/:id/merge_requests/:iid/notes` for list/create; `glab api -X PUT .../notes/:note_id` for update; `glab api -X DELETE` for delete.

### `gg sync` flow change — two-pass execution

The existing sync is single-pass: iterate entries, create-or-update each PR. The new flow:

1. **Pass 1 (unchanged):** iterate entries, create/update PRs, collect the final `pr_number` for each entry.
2. **Pass 2 — nav reconcile (conditional):** runs only when `stack_nav_comments == true` AND `stack.entries.len() >= 2`. For each entry whose PR is still open:
   - Render the nav body with this entry's index flagged `is_current = true`.
   - Call `find_managed_comment` on the PR.
   - If no managed comment exists: `create_pr_comment`.
   - If one exists and its body matches the rendered body: no-op (skip network call).
   - If one exists with different body: `update_pr_comment`.
3. **Pass 3 — cleanup (conditional):** runs when `stack_nav_comments == false` OR `stack.entries.len() < 2`. For each entry whose PR is still open, call `find_managed_comment`; if present, `delete_pr_comment`.

Passes 2 and 3 are mutually exclusive — only one runs per sync, selected by state at sync time.

### JSON output extension

`SyncEntryResultJson` gains an optional field:

```rust
pub nav_comment_action: Option<String>, // "created" | "updated" | "unchanged" | "deleted" | "skipped" | "error"
```

Omitted when the setting is disabled AND no cleanup happens, so existing JSON consumers are unaffected for users who don't opt in.

## Testing

### Unit tests

**`stack_nav::render`:**
- Single-entry stack returns an empty string (or is-short-circuit safe).
- Two-entry stack with `is_current` on first → `- 👉 #1\n- #2`.
- Three-entry stack with `is_current` on middle.
- GitLab prefix `!` produces `!1`, `!2`, etc.
- Bottom-up ordering preserved when `is_current` is on the last entry.
- Rendering the same input twice is byte-identical (idempotency).
- Stack name appears in header exactly once, backtick-quoted.

**`stack_nav::is_managed_comment`:**
- Plain user comment → `false`.
- Comment containing exactly the marker → `true`.
- Marker with surrounding whitespace → `true`.
- Marker appearing inside a fenced code block in a user comment → still matches (acceptable false positive — extremely unlikely in practice, and harmless since we never create comments that mix our marker with user content).

### Integration tests

Using a mock provider implementation that records all `create/update/delete/find` calls:

- Setting `false` → no comment API calls.
- Setting `true`, 2 entries, first sync → 2 `create_pr_comment` calls with correct bodies and 👉 placement.
- Setting `true`, 2 entries, second sync with no stack changes → 2 `find_managed_comment` calls, 0 `update_pr_comment` calls (byte-identical bodies skip update).
- Setting `true`, 2 entries, adding a 3rd entry and re-syncing → 2 `update_pr_comment` calls (PRs #1 and #2 get updated nav) + 1 `create_pr_comment` (new PR #3 gets its initial nav).
- Toggling setting `true` → `false` → next sync issues `delete_pr_comment` for every open PR with a managed comment.
- Stack of 2 entries where one PR is merged → only the open PR gets a comment touch.
- Stack shrinks to 1 entry via `gg clean` → next sync deletes the stack-nav comment on the remaining open PR.

### No E2E against real GitHub/GitLab

New provider methods are covered by the existing unit-test patterns for `gh.rs` / `glab.rs` command-shape tests. Live E2E against GitHub/GitLab is expensive and the provider-shell pattern is the same as the existing `create_pr` / `update_pr_description` methods.

## Documentation & skill updates

- **`docs/src/`** — document the `stack_nav_comments` config field under the sync command reference and in the configuration guide page. Include a small screenshot or rendered example of the comment.
- **`skills/gg/SKILL.md`** — add a short section explaining that `gg sync` may leave a managed comment on each PR when the setting is enabled, so agents don't treat it as unexpected content.
- **`skills/gg/reference.md`** — document the new JSON field `nav_comment_action` and the config field.
- **`README.md`** — add to the feature list.

## Out of scope for v1

These are explicitly deferred and can be revisited if demand emerges:

- Per-stack override of the setting.
- Configurable header text or emoji.
- Inline commit titles on each line (relies on provider expansion instead).
- A `gg sync --no-stack-nav` one-shot opt-out flag — the config setting is sufficient.
- An on-demand `gg stack-nav` command that rewrites nav without full sync.
- Surfacing the managed comment in `gg ls` output.
