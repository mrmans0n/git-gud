# `gg split` — Design Document

**Date:** 2026-03-13
**Author:** Ambrosio (AI) + Nacho López
**Status:** Draft

## Summary

Add a `split` command to git-gud that splits a commit in the stack into two commits. The user selects which files (and later, hunks) go into the first commit; the rest stays in the second. All descendants in the stack are automatically rebased.

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Selection granularity | Files first, hunks later (`-i` flag) | Progressive complexity; files cover 80% of cases |
| Target commit | Any commit in the stack | Full flexibility; gg already has rebase infra |
| Commit ordering | Selected → below, remainder → above | Convention from jj/hg; "extract and push down" |
| Commit messages | Prompt for both | Full control over both new commits |
| File selection UI | CLI args + dialoguer checkbox | Args for scripts/MCP, checkbox for interactive |

## UX

### Command Syntax

```
gg split [OPTIONS] [FILES...]
```

### Arguments

| Argument | Description |
|----------|-------------|
| `FILES...` | Files to include in the first (lower) commit. If omitted, opens interactive selector. |

### Options

| Flag | Short | Description |
|------|-------|-------------|
| `--commit <TARGET>` | `-c` | Target commit: position (1-indexed), short SHA, or GG-ID. Default: current (HEAD). |
| `--message <MSG>` | `-m` | Message for the first (selected) commit. Skips editor prompt for that commit. |
| `--no-edit` | | Keep original message for remainder, don't prompt. Only prompt for the new commit. |

### Interactive Flow (no FILES args)

The selected files become a **new commit inserted BEFORE the original** in the stack.
The original commit keeps the remaining (unselected) files and stays in its position.

```
$ gg split

Splitting commit 3: "Add auth and logging" (abc1234)

Select files for the new commit (inserted BEFORE the original in the stack):

  [x] src/auth.rs          (+120 -0)
  [ ] src/logging.rs       (+45 -10)
  [x] src/auth_test.rs     (+80 -0)
  [ ] src/main.rs          (+3 -1)

Enter message for the new commit (inserted before original):
> Add authentication module

Enter message for the original commit (remaining files):
> Add logging improvements

✔ Split complete!
  New commit 3 (before): a1b2c3d "Add authentication module" (2 files)
  Original commit 4 (after): d4e5f6a "Add logging improvements" (2 files)
  Rebased 2 descendant commits.
```

Stack visualization:
```
BEFORE                    AFTER
  5: "Fix tests"            6: "Fix tests"       (rebased)
  4: "Add UI"               5: "Add UI"           (rebased)
  3: "Add auth+logging"     4: "Add logging"      ← original (remainder, keeps GG-ID)
  2: "Setup DB"             3: "Add auth"         ← NEW commit (selected, inserted before)
  1: "Init project"         2: "Setup DB"
                             1: "Init project"
```

### Non-interactive Flow (FILES provided)

```
$ gg split -c 3 -m "Add authentication module" src/auth.rs src/auth_test.rs

✔ Split complete!
  New commit 3 (before): a1b2c3d "Add authentication module" (2 files)
  Original commit 4 (after): d4e5f6a "Add logging improvements" (2 files)
  Rebased 2 descendant commits.
```

### Edge Cases

| Case | Behavior |
|------|----------|
| All files selected | Warn: "All changes selected — original commit will be empty" (proceed anyway) |
| No files selected | Error: "No files selected, nothing to split" |
| Single-file commit | Error: "Commit only has 1 file, nothing to split" (unless future hunk mode) |
| Dirty working directory | Error: "Working directory not clean" (same as reorder/rebase) |
| Commit not in stack | Error: "Commit not found in current stack" |
| Merge conflicts during rebase | Abort split, restore original state, report error |

## Architecture

### Algorithm

```
1. Load stack, resolve target commit
2. Require clean working directory
3. Get diff between target commit and its parent → list of changed files with stats
4. User selects files (CLI args or interactive checkbox)
5. Validate selection (not empty, not all — or warn)
6. Create FIRST commit (selected files):
   a. Start from parent tree
   b. For each selected file: copy blob from target commit's tree
   c. Create new tree with these changes
   d. Create commit with this tree, parent = target's parent
   e. Assign new GG-ID
7. Create SECOND commit (remainder):
   a. Tree = target commit's original tree (has ALL changes)
   b. Parent = first commit
   c. Keeps original GG-ID (for PR tracking continuity)
   d. Update commit message
8. Rebase all descendants: re-parent commits above the original to point to second commit
9. Update branch pointers
10. Prompt for commit messages (unless provided via flags)
```

### Why the remainder keeps the original GG-ID

The GG-ID is used by `gg sync` to map commits to PRs. The "remainder" commit is the logical continuation of the original (same position, same PR), so it keeps the GG-ID. The new "selected" commit gets a fresh GG-ID and will become a new PR on next `gg sync`.

### Key Implementation Details

**File diff extraction (step 3):**
Use `git2::Diff` between parent and target commit to enumerate changed files with their stats (+/- lines). This is already similar to what `absorb` does.

**Tree manipulation (steps 6-7):**
Use `git2::TreeBuilder` to construct the new trees:
- For the first commit: start with parent's tree, replace selected file blobs with target's versions
- For the second commit: use target's original tree directly (parent is now first commit, so the diff is implicit)

**Rebase (step 8):**
Reuse the rebase infrastructure from `reorder.rs` — it already handles rebasing a sequence of commits onto a new base. The pattern: `git rebase --onto <second_commit> <original_commit> <stack_tip>`.

**Commit message editing (step 10):**
Use `dialoguer::Editor` (same as `reorder.rs`) or `$EDITOR` to let the user write messages. With `-m` flag, skip the editor for the first commit. With `--no-edit`, skip the editor for the remainder.

### Module Structure

```
crates/gg-core/src/commands/split.rs    # Command implementation
crates/gg-cli/src/main.rs              # CLI argument parsing (add Split variant)
crates/gg-cli/tests/integration_tests.rs # Integration tests
```

### New Dependencies

None — `git2`, `dialoguer`, and `console` already in the workspace.

## Future: Hunk-level Splitting (`-i` / `--interactive`)

Not in this PR, but the design accommodates it:

- Add a `--interactive` / `-i` flag that enables hunk-level selection
- Instead of file list → show hunks per file (like `git add -p`)
- Implementation options:
  1. **Shell out to `git add -p`** — Easiest, delegates UI to Git
  2. **TUI with ratatui** — Best UX, most work
  3. **Delegate to external diff editor** — Like jj does
- The tree construction logic stays the same; only step 3-4 changes (hunks instead of whole files)

## Test Plan

1. **Unit tests** (in `split.rs`):
   - File selection logic (filtering, validation)
   - Tree construction (selected files → new tree)
   - GG-ID assignment (new for selected, preserved for remainder)

2. **Integration tests** (in `integration_tests.rs`):
   - Split HEAD commit with file args → two commits, correct trees
   - Split non-HEAD commit → descendants rebased correctly
   - Split with `--message` flag → correct messages, no editor prompt
   - Edge cases: single-file commit, all files selected, dirty working dir
   - GG-ID continuity: remainder keeps original GG-ID
   - Branch pointers updated correctly after split

## Open Questions

None — all design decisions resolved.
