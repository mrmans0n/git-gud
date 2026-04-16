# `gg split`

Split a commit in the stack into two commits. The selected hunks become a new commit inserted **before** the original in the stack, while the remaining changes stay in the original commit.

```bash
gg split [OPTIONS] [FILES...]
```

## Options

- `-c, --commit <TARGET>`: Target commit — position (1-indexed), short SHA, or GG-ID. Defaults to the current commit (HEAD).
- `-m, --message <MESSAGE>`: Commit message for the new (first) commit. Skips the editor prompt.
- `--no-edit`: Keep the original message for the remainder commit without prompting.
- `--no-tui`: Disable TUI, use sequential prompt instead (legacy `git add -p` style).
- `-f, --force` (alias `--ignore-immutable`): Override the immutability guard.
  Splitting a merged or base-ancestor commit is refused by default. See
  [Core concepts · Immutable commits](../core-concepts.md#immutable-commits).
- `FILES...`: Files to include in the new commit. When provided, all hunks from those files are auto-selected (skips the interactive picker).

## How It Works

When you split commit **K** into two:

1. **New commit (K')** — Contains only the selected hunks. Inserted **before** K in the stack. Gets a new GG-ID.
2. **Remainder (K'')** — Contains the remaining hunks. Stays in K's original position. Keeps the original GG-ID (preserving PR association).

All descendant commits are automatically rebased onto the remainder.

```
BEFORE                    AFTER
  4: "Fix tests"            5: "Fix tests"       (rebased)
  3: "Add auth+logging"     4: "Add logging"      ← remainder (keeps GG-ID)
  2: "Setup DB"             3: "Add auth"         ← NEW commit (selected changes)
  1: "Init project"         2: "Setup DB"
                             1: "Init project"
```

## Interactive Hunk Selection

Running `gg split` without file arguments opens the interactive hunk picker:

```bash
# Split the current commit — opens hunk selector
gg split

# Split a specific commit in the stack
gg split -c 3
```

### TUI Mode (Default)

When run with a TTY, `gg split` opens a two-panel TUI for hunk selection:

```
┌── Files (1/3 width) ──┬── Diff (2/3 width) ──────────────┐
│ [✓] src/auth.rs (3)   │ @@ -10,6 +10,12 @@               │
│ [ ] src/logging.rs (1)│ +  // Validate token              │
│ [~] src/tests.rs (2)  │ +  if token.is_empty() {          │
│                        │ +      return false;               │
│                        │ +  }                               │
├────────────────────────┴────────────────────────────────────┤
│ 5/12 hunks selected │ [Space] toggle · [Tab] switch panel │
└─────────────────────────────────────────────────────────────┘
```

#### TUI Keyboard Shortcuts

| Key | In File Panel | In Diff Panel |
|-----|---------------|---------------|
| ↑/↓ or j/k | Navigate files | Navigate hunks |
| Space | Toggle all hunks for file | Toggle current hunk |
| a | Select all hunks (all files) | Select all hunks (this file) |
| n | Deselect all hunks (all files) | Deselect all hunks (this file) |
| s | — | Split current hunk into sub-hunks |
| Tab / ← / → | Switch to diff panel | Switch to file panel |
| Enter | Enter commit message | Enter commit message |
| q / Esc | Abort (cancel split) | Abort (cancel split) |

#### Inline Commit Message

After pressing **Enter** to confirm your hunk selection, an inline text input appears at the bottom of the TUI for the commit message. It's pre-filled with `Split from: <original commit title>`.

| Key | Action |
|-----|--------|
| Enter | Confirm message and create the split |
| Esc | Go back to hunk selection |
| ← / → | Move cursor |
| Home / End | Jump to beginning/end |
| Backspace / Delete | Delete characters |
| Ctrl+A / Ctrl+E | Jump to beginning/end (emacs-style) |
| Ctrl+U | Clear from cursor to beginning |
| Ctrl+K | Clear from cursor to end |

After confirming the new commit message, a second inline input appears for the **remainder commit message** (pre-filled with the original commit's message). This replaces the external editor for both messages, keeping the entire split workflow inside the TUI.

| Key | Action |
|-----|--------|
| Enter | Confirm remainder message and complete the split |
| Esc | Go back to the new commit message input |

The `-m` flag still works and bypasses the TUI input for the new commit. The `--no-edit` flag skips the remainder message input entirely, keeping the original message as-is.

#### File Panel Indicators

- `[✓]` — All hunks selected (green)
- `[~]` — Some hunks selected (yellow)
- `[ ]` — No hunks selected

### Sequential Prompt Mode (`--no-tui`)

Use `--no-tui` to fall back to the legacy `git add -p` style sequential prompt:

```bash
gg split --no-tui
```

This mode is automatically used when no TTY is available (e.g., in CI pipelines or when piping).

For each hunk, you'll see the diff with colored output and a prompt:

```
--- a/src/auth.rs
+++ b/src/auth.rs
@@ -10,6 +10,12 @@ fn authenticate(user: &str) -> bool {
+    // Validate token
+    if token.is_empty() {
+        return false;
+    }

Include this hunk? [y]es/[n]o/[a]ll file/[d]one file/[s]plit/[q]uit/?help:
```

#### Sequential Mode Actions

| Key | Action | Description |
|-----|--------|-------------|
| `y` | Yes | Include this hunk in the new commit |
| `n` | No | Skip this hunk (stays in remainder) |
| `a` | All file | Include all remaining hunks from this file |
| `d` | Done file | Skip all remaining hunks from this file |
| `s` | Split | Split this hunk into smaller hunks |
| `q` | Quit | Stop; all remaining hunks stay in remainder |
| `?` | Help | Show this help |

## File-Based Splitting

When file arguments are provided, all hunks from those files are auto-selected without opening the interactive picker:

```bash
# Move auth files to a new commit before the current one
gg split -m "Add authentication" src/auth.rs src/auth_test.rs

# Split a specific commit with explicit files
gg split -c 3 src/config.rs

# Non-interactive with both messages
gg split -c 2 -m "Extract helpers" --no-edit helpers.rs utils.rs

# Split by GG-ID
gg split -c c-abc1234 src/config.rs
```

### Hunk Splitting

If a hunk contains multiple logical changes separated by unchanged lines, pressing `s` will break it into smaller hunks. You can then select each sub-hunk individually. If the hunk is already atomic (contiguous changes), you'll see "This hunk cannot be split further."

## Edge Cases

- **All changes selected** — Warning: the original commit will be empty.
- **No changes selected** — Error.
- **Dirty working directory** — Error. Commit or stash changes first.
- **Merge conflicts during rebase** — Split is aborted, original state restored.
