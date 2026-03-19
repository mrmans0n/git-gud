# gg drop + gg arrange — Design Doc

## Goal

Add two new capabilities inspired by Jujutsu (jj):
1. **`gg drop`** — Remove commits from the stack (like `jj abandon`)
2. **`gg arrange`** — Alias for `gg reorder` with enhanced TUI that supports dropping commits inline

## Feature 1: `gg drop`

### Command

```
gg drop <TARGET>...
```

**Aliases:** `gg abandon`

### Arguments

- `<TARGET>` — One or more commit identifiers: position (1-indexed), short SHA, or GG-ID
- `--force` / `-f` — Skip confirmation prompt
- `--json` — JSON output

### Behavior

1. Validate clean working directory
2. Load stack
3. Resolve target commits from arguments
4. Show what will be dropped (commit title, SHA, position)
5. Ask for confirmation (unless `--force`)
6. For each commit to drop (in reverse order, from top of stack):
   - Rebase descendants onto the parent of the dropped commit
   - Remove per-commit branch (if exists)
7. Print summary

### JSON Output

```json
{
  "version": 1,
  "drop": {
    "dropped": [
      {"position": 3, "sha": "abc1234", "title": "Fix typo"}
    ],
    "remaining": 4
  }
}
```

### Edge Cases

- Dropping all commits → error "Cannot drop all commits"
- Dropping non-existent position → error with valid range
- Current position is dropped → move to the commit below (or top if dropping bottom)
- Commit has open PR → warn but allow (with --force, no warning)

### Implementation

Uses `git rebase -i` with `GIT_SEQUENCE_EDITOR` to produce a todo list that omits the dropped commits (same pattern as `reorder.rs`).

## Feature 2: `gg arrange`

### Command

```
gg arrange [--no-tui] [--order ORDER]
```

This is an alias for `gg reorder`. Both commands share the same implementation.

### TUI Enhancement

Add drop support to the existing reorder TUI:
- **`d` key** — Toggle drop mark on current commit (strikethrough + red)
- **`Delete` key** — Same as `d`
- Visual: dropped commits show as ~~strikethrough~~ in red with `[DROP]` prefix
- Dropped commits can still be moved (in case user wants to undrop)
- Dropped commits cannot be the only remaining commits (at least 1 must survive)
- Help text updated to show the new keybinding

### Editor Fallback

The editor fallback already has the comment "Delete a line to drop that commit." — just need to make the validation accept fewer commits than the stack size.

### Implementation

1. Add `dropped: HashSet<usize>` to `ReorderTuiState`
2. Add `toggle_drop()` method
3. Modify `get_order()` → `get_result()` returning both order and drops
4. Modify `draw()` to show dropped commits visually
5. Modify `reorder_tui()` return type to include drop info
6. Modify `reorder.rs` to handle drops after reorder

## Command Registration (main.rs)

```rust
/// Drop commits from the stack
#[command(name = "drop", aliases = ["abandon"])]
Drop {
    /// Commits to drop: position (1-indexed), short SHA, or GG-ID
    targets: Vec<String>,
    /// Skip confirmation
    #[arg(short, long)]
    force: bool,
    /// JSON output
    #[arg(long)]
    json: bool,
},

/// Arrange commits: reorder and/or drop (alias for reorder)
#[command(name = "arrange")]
Arrange {
    #[arg(short, long, value_name = "ORDER")]
    order: Option<String>,
    #[arg(long)]
    no_tui: bool,
},
```

## Docs & Skills

- New doc page: `docs/src/commands/drop.md`
- Update: `docs/src/commands/reorder.md` (mention arrange alias + drop in TUI)
- Update: `skills/gg/SKILL.md` and `skills/gg/reference.md`
