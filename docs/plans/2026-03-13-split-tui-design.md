# `gg split` TUI — Design Document

**Date:** 2026-03-13
**Author:** Ambrosio (AI) + Nacho López
**Status:** Approved
**Depends on:** `gg split` file-level (PR #207) + hunk-level (PR #209)

## Summary

Replace the sequential `git add -p` style prompt with a full TUI (terminal UI) for hunk selection during `gg split`. Two-panel layout: files on the left, diff hunks on the right. Toggle files or individual hunks with keyboard shortcuts.

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Layout | Two panels (lazygit style) | Good info density, familiar |
| Activation | TUI by default when TTY, `--no-tui` for sequential fallback | TUI is the better experience |
| Granularity | Files + hunks | Toggle whole file or individual hunks |
| Framework | ratatui + crossterm | Already transitive deps via skim |

## Layout

```
┌── Files (1/3 width) ──┬── Diff (2/3 width) ──────────────┐
│ [✓] src/auth.rs (3)   │ @@ -10,6 +10,12 @@               │
│ [ ] src/logging.rs (1)│ +  // Validate token              │
│ [~] src/tests.rs (2)  │ +  if token.is_empty() {          │
│                        │ +      return false;               │
│                        │ +  }                               │
│                        │                                    │
│                        │ @@ -25,3 +31,8 @@                 │
│                        │ +  let valid = check_token(tok);   │
│                        │                                    │
├────────────────────────┴────────────────────────────────────┤
│ Splitting: "Add auth and logging" (abc1234)                 │
│ [Space] toggle · [a] all · [Tab] switch panel · [Enter] ok │
│ [s] split hunk · [q] quit                                   │
└─────────────────────────────────────────────────────────────┘
```

### File Panel (left, ~1/3 width)

- List of changed files with hunk counts
- Status indicators:
  - `[✓]` — all hunks selected
  - `[~]` — some hunks selected (partial)
  - `[ ]` — no hunks selected
- Arrow keys to navigate
- Space to toggle entire file (all or none)
- Enter or Tab to switch to diff panel to select individual hunks

### Diff Panel (right, ~2/3 width)

- Shows hunks for the currently focused file
- Each hunk has a checkbox `[✓]`/`[ ]`
- Colored diff: green for additions, red for deletions, dim for context
- Arrow keys to navigate between hunks
- Space to toggle individual hunk
- `s` to split current hunk into sub-hunks
- Tab to switch back to file panel

### Status Bar (bottom)

- Shows commit being split (title + short SHA)
- Keyboard shortcuts legend
- Selected hunk count: "5/12 hunks selected"

## Keyboard Shortcuts

| Key | In File Panel | In Diff Panel |
|-----|---------------|---------------|
| ↑/↓ or j/k | Navigate files | Navigate hunks |
| Space | Toggle all hunks for file | Toggle current hunk |
| a | Select all hunks (all files) | Select all hunks (this file) |
| n | Deselect all hunks (all files) | Deselect all hunks (this file) |
| s | — | Split current hunk into sub-hunks |
| Tab / → / ← | Switch to diff panel | Switch to file panel |
| Enter | Confirm selection, proceed | Confirm selection, proceed |
| q / Esc | Quit (abort split) | Quit (abort split) |
| ? | Show help overlay | Show help overlay |

## Activation

- `gg split -i` with TTY → **TUI** (default)
- `gg split -i --no-tui` with TTY → sequential prompt (fallback)
- `gg split -i` without TTY (pipe/CI) → sequential prompt (automatic fallback)
- `gg split` (no `-i`) on multi-file → file checkbox (unchanged)
- `gg split` on single-file → TUI (auto-hunk, as before but now TUI)

## Architecture

### New Module

Create `crates/gg-core/src/commands/split_tui.rs` — keeps the TUI code separate from split logic.

### Dependencies

Add to `crates/gg-core/Cargo.toml`:
```toml
ratatui = "0.30"
crossterm = "0.29"
```

### Key Types

```rust
/// State for the split TUI
struct SplitTuiState {
    files: Vec<TuiFile>,
    active_panel: Panel,        // Files or Diff
    file_cursor: usize,         // Selected file index
    hunk_cursor: usize,         // Selected hunk index within file
    scroll_offset: usize,       // Vertical scroll in diff panel
}

struct TuiFile {
    path: String,
    hunks: Vec<TuiHunk>,
}

struct TuiHunk {
    diff_hunk: DiffHunk,        // Reuse from split.rs
    selected: bool,
}

enum Panel {
    Files,
    Diff,
}
```

### Integration

`split_tui.rs` exposes one public function:

```rust
pub fn select_hunks_tui(hunks: Vec<DiffHunk>) -> Result<Vec<usize>>
```

This returns the same `Vec<usize>` (selected hunk indices) that `select_hunks_interactive()` returns. The `run()` function in `split.rs` calls one or the other based on TTY detection and flags.

### TTY Detection

```rust
let use_tui = options.interactive
    && !options.no_tui
    && atty::is(atty::Stream::Stdin)
    && atty::is(atty::Stream::Stdout);
```

`atty` is already a dependency of gg-core.

## Test Plan

TUI is inherently hard to test in integration tests. Strategy:

1. **Unit tests for TUI state logic** — test toggle, navigation, selection counting without rendering
2. **Existing sequential prompt tests** still pass (fallback path)
3. **Manual testing** — the TUI is visual, needs human verification

## Edge Cases

| Case | Behavior |
|------|----------|
| Terminal too small | Show warning, fall back to sequential prompt |
| No hunks to display | Show empty diff panel with message |
| Single hunk file | Still show in TUI, can toggle |
| User quits without confirming | Abort split, no changes |
| Resize during TUI | Redraw (ratatui handles this) |
