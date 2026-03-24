# Stack Parent Trailers Implementation Plan

**Goal:** Replace the discarded PR-description breadcrumb approach with stable commit-local stack metadata using `GG-Parent` trailers.
**Architecture:** Extend existing GG-ID trailer handling in `gg-core/src/git.rs`, expose parent metadata on stack entries, and add one reusable trailer-normalization pass that `sync`, `reconcile`, and structural stack-editing commands can reuse.
**Tech Stack:** Rust, `git2`, existing stack model (`Stack`, `StackEntry`), existing sync/reconcile rewrite flows, CLI/integration tests.

---

### Task 1: Add GG-Parent trailer parsing and message helpers

**Files:**
- Modify: `crates/gg-core/src/git.rs`
- Test: `crates/gg-core/src/git.rs`

**Steps:**
1. Add `GG_PARENT_PREFIX` constant.
2. Add `get_gg_parent(commit: &Commit) -> Option<String>`.
3. Add helpers to set/remove GG-Parent in a commit message without disturbing non-GG content.
4. Add a higher-level helper that normalizes both GG trailers in one pass.
5. Add unit tests for:
   - missing parent trailer
   - setting parent trailer
   - replacing parent trailer
   - removing parent trailer for first stack entry
   - preserving body + non-GG trailers

### Task 2: Expose parent metadata in the stack model

**Files:**
- Modify: `crates/gg-core/src/stack.rs`
- Test: `crates/gg-core/src/stack.rs` (or integration if existing unit coverage is insufficient)

**Steps:**
1. Add `gg_parent: Option<String>` to `StackEntry`.
2. Populate it in `StackEntry::from_commit()`.
3. Add a helper to compute the expected parent GG-ID for each loaded entry from stack order.
4. Add tests covering:
   - first entry has no expected parent
   - middle/head entries resolve expected parent correctly

### Task 3: Replace GG-ID-only rewrite with generalized metadata normalization

**Files:**
- Modify: `crates/gg-core/src/commands/sync.rs`
- Modify: `crates/gg-core/src/commands/reconcile.rs`
- Modify: `crates/gg-core/src/output.rs`
- Test: `crates/gg-core/src/commands/sync.rs`

**Steps:**
1. Replace `add_gg_ids_to_commits()` with a generalized stack metadata normalization helper.
2. The helper should, for each entry in order:
   - preserve existing GG-ID when present
   - generate one if missing
   - set/remove `GG-Parent` to match stack order
3. Return structured counts:
   - `gg_ids_added`
   - `gg_parents_updated`
   - `gg_parents_removed`
4. Extend sync JSON output with a `metadata` block.
5. Update reconcile to detect and repair GG-Parent drift.
6. Add tests for:
   - sync adds missing GG-ID and GG-Parent together
   - sync repairs stale GG-Parent after history edits
   - JSON output includes metadata counts

### Task 4: Normalize metadata after structural stack changes

**Files:**
- Modify: `crates/gg-core/src/commands/reorder.rs`
- Modify: `crates/gg-core/src/commands/drop_cmd.rs`
- Modify: `crates/gg-core/src/commands/split.rs`
- Possibly modify: `crates/gg-core/src/commands/mod.rs` (if shared helper placement requires it)
- Test: `crates/gg-cli/tests/integration_tests.rs`

**Steps:**
1. After `reorder`, run trailer normalization and verify parent chain matches new order.
2. After `drop`, run trailer normalization and verify the first surviving child points to the correct predecessor.
3. After `split`, ensure:
   - new lower commit gets a fresh GG-ID
   - original upper commit keeps its GG-ID
   - both end up with correct GG-Parent trailers after normalization
4. Add integration tests for reorder/drop/split covering GG-Parent behavior.

### Task 5: Update docs and skills

**Files:**
- Modify: `README.md`
- Modify: `docs/src/core-concepts.md`
- Modify: `docs/src/commands/sync.md`
- Modify: `docs/src/commands/reconcile.md`
- Modify: `skills/gg/SKILL.md`
- Modify: `skills/gg/reference.md`

**Steps:**
1. Document `GG-Parent` as the stable structural trailer.
2. Remove/replace any description of PR/MR breadcrumb syncing.
3. Document that `sync` and `reconcile` may rewrite commit metadata to normalize stack topology.

### Task 6: Full verification and PR

**Files:**
- No code changes expected unless fixes are needed.

**Steps:**
1. Run `cargo fmt --all`.
2. Run `cargo clippy --all-targets --all-features -- -D warnings`.
3. Run `cargo test --all-features`.
4. Open a PR with design/plan references.
