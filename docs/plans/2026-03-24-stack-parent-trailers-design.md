# Stack Parent Trailers Design

**Status:** proposed  
**Date:** 2026-03-24  
**Scope:** Replace the discarded PR-description breadcrumb approach with stable commit-local stack metadata.

---

## Problem

The original roadmap item proposed storing stack breadcrumbs in PR/MR descriptions during `gg sync`.

That approach has two problems:

1. It stores stack context in the wrong place. The stack structure belongs to commits, not remote PR/MR text.
2. It encodes presentation-oriented data (`position`, `prev`, `next`) that changes frequently and is not stable across stack edits.

We want a solution that behaves more like `GG-ID`: stable metadata attached to commits and preserved across rebases.

---

## Goals

1. Persist stack topology in commit trailers.
2. Keep the metadata stable across rebases, reorder, drop, split, and future reparent/restack operations.
3. Avoid storing presentation/UI breadcrumbs in remote PR/MR descriptions.
4. Make the topology derivable by both CLI and MCP consumers from commit-local metadata.
5. Preserve existing `GG-ID` semantics and PR/MR mapping behavior.

---

## Non-goals

- Do **not** update PR/MR descriptions with breadcrumb blocks.
- Do **not** store `position`, `prev`, or `next` as trailers.
- Do **not** introduce a new remote-only synchronization mechanism.
- Do **not** add a full smartlog UI in this change (that remains a later roadmap task).

---

## Proposed metadata model

Add a new commit trailer:

```text
GG-ID: c-abc1234
GG-Parent: c-1234567
```

### Semantics

- `GG-ID` continues to identify the commit itself.
- `GG-Parent` identifies the immediate previous stack entry by its **GG-ID**.
- The first entry in the stack has **no `GG-Parent` trailer**.
- Because stacks are linear, child relationships are derived by scanning entries whose `GG-Parent` matches another entry's `GG-ID`.

### Why `GG-Parent` only

We intentionally avoid storing:
- `GG-Position`
- `GG-Prev`
- `GG-Next`
- stack breadcrumb text

These are all derived, presentation-oriented values. They change whenever the stack is reordered, split, or pruned. `GG-Parent` is the smallest stable structural fact that lets higher-level views reconstruct stack relationships.

---

## Invariants

For a loaded stack ordered from base to HEAD:

- entry `0`: has `GG-ID`, has **no** `GG-Parent`
- entry `n > 0`: has `GG-ID`, and `GG-Parent == entries[n - 1].gg_id`

Additional rules:

- `GG-Parent` must always reference another valid GG-ID in the same stack.
- There must be no self-parent reference.
- There must be no duplicate GG-IDs.
- A stack should remain linear; multiple children for the same parent are considered invalid for this first version.

---

## Behavioral changes

### `gg sync`

`gg sync` should normalize stack trailers before pushing:

- add missing `GG-ID`s if needed (existing behavior)
- update/remove `GG-Parent` trailers to match current stack order

`gg sync` should report this in JSON output, for example:

```json
{
  "sync": {
    "metadata": {
      "gg_ids_added": 1,
      "gg_parents_updated": 2,
      "gg_parents_removed": 1
    }
  }
}
```

### `gg reconcile`

`gg reconcile` should also detect and fix missing/incorrect `GG-Parent` trailers alongside GG-ID maintenance.

### Structural commands

Commands that change stack shape should leave the stack with normalized trailers:

- `gg reorder`
- `gg drop`
- `gg split`
- future `gg reparent` / `gg restack`

Commands that do not change stack topology (for example squash/amend) should preserve trailers unchanged.

---

## Command-specific expectations

### Reorder

After reorder, rewrite `GG-Parent` trailers to match the new order.

### Drop

After drop, the commit immediately above the removed region should point to the nearest surviving predecessor, or have no `GG-Parent` if it becomes the first entry.

### Split

When splitting an original commit `B` into `B1` + `B2`:

- `B1` gets a new `GG-ID`
- `B1.GG-Parent` becomes the original predecessor's GG-ID (or none if first)
- `B2` keeps the original `GG-ID`
- `B2.GG-Parent` becomes `B1`'s new GG-ID
- descendants of `B2` keep referencing `B2`'s GG-ID, so they do not need logical parent changes beyond normal stack rewrite

### Sync after existing history edits

If a user edited history externally and left `GG-Parent` stale, `gg sync` should repair it automatically.

---

## Implementation approach

### 1. Add GG-Parent trailer helpers in `git.rs`

Add helpers parallel to existing GG-ID utilities:

- `pub const GG_PARENT_PREFIX: &str = "GG-Parent:";`
- `get_gg_parent(commit: &Commit) -> Option<String>`
- `set_gg_parent_in_message(message: &str, parent: Option<&str>) -> String`
- `strip_gg_parent_from_message(message: &str) -> String`

Also add a higher-level helper that normalizes both trailers together so the logic is centralized.

### 2. Extend `StackEntry`

Include optional parent GG-ID on loaded entries:

```rust
pub gg_parent: Option<String>
```

This keeps the topology visible to higher layers and JSON output in the future.

### 3. Introduce a reusable trailer normalization pass

Create a small helper in `gg-core` that rewrites a stack in order while preserving:

- tree contents
- author/committer
- GG-ID continuity

and normalizing:

- `GG-ID`
- `GG-Parent`

This replaces the current `add_gg_ids_to_commits()`-only approach with a generalized metadata rewrite.

### 4. Use the normalization pass in `sync` and `reconcile`

`sync` and `reconcile` are the natural places to auto-heal metadata drift.

### 5. Update structural commands only where necessary

Commands that already rewrite history should either:

- run the normalization helper afterwards, or
- set correct trailers directly during rewrite if that is simpler/safer

The simplest first implementation is to run one post-operation normalization pass after `drop`, `reorder`, and `split`.

---

## Alternatives considered

### A. PR/MR description breadcrumbs

Rejected.

They are remote presentation data, not durable stack metadata, and they do not belong in commit-local state.

### B. Store `GG-Position`, `GG-Prev`, `GG-Next`

Rejected.

Those values are derived and volatile. They change often and create unnecessary rewrite churn.

### C. Store only current Git parent SHA

Rejected.

The Git parent SHA is not stable across rebases. The whole point of GG metadata is durable identity across rewritten history.

---

## Risks

1. **More history rewriting**  
   Normalizing trailers can rewrite commits even when file trees are unchanged.

2. **Message trailer edge cases**  
   We must preserve user-authored message body and non-GG trailers.

3. **Mixed old/new stacks**  
   Existing stacks without `GG-Parent` should be upgraded automatically without breaking sync.

---

## Mitigations

- Keep normalization logic centralized and test-heavy.
- Only manage `GG-ID` / `GG-Parent`; leave all other message content intact.
- Report metadata rewrites in JSON output and human messages so automation is not surprised.

---

## Success criteria

- A synced stack contains stable `GG-ID` + `GG-Parent` trailers.
- Reordering, dropping, and splitting leave the stack with correct parent metadata.
- No PR/MR description breadcrumbs are written.
- `gg sync --json` exposes metadata rewrite counts.
- The feature is fully covered by unit + integration tests.
