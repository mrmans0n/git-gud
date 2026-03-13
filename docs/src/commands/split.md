# `gg split`

Split a commit in the stack into two commits. The selected files/hunks become a new commit inserted **before** the original in the stack, while the remaining changes stay in the original commit.

```bash
gg split [OPTIONS] [FILES...]
```

## Options

- `-c, --commit <TARGET>`: Target commit — position (1-indexed), short SHA, or GG-ID. Defaults to the current commit (HEAD).
- `-m, --message <MESSAGE>`: Commit message for the new (first) commit. Skips the editor prompt.
- `--no-edit`: Keep the original message for the remainder commit without prompting.
- `-i, --interactive`: Select individual hunks interactively. Auto-enabled for single-file commits.
- `--no-tui`: Disable TUI, use sequential prompt instead (legacy `git add -p` style).
- `FILES...`: Files to include in the new commit. If omitted, opens an interactive file selector (or hunk selector in interactive mode).

## How It Works

When you split commit **K** into two:

1. **New commit (K')** — Contains only the selected files/hunks. Inserted **before** K in the stack. Gets a new GG-ID.
2. **Remainder (K'')** — Contains the remaining files/hunks. Stays in K's original position. Keeps the original GG-ID (preserving PR association).

All descendant commits are automatically rebased onto the remainder.

```
BEFORE                    AFTER
  4: "Fix tests"            5: "Fix tests"       (rebased)
  3: "Add auth+logging"     4: "Add logging"      ← remainder (keeps GG-ID)
  2: "Setup DB"             3: "Add auth"         ← NEW commit (selected changes)
  1: "Init project"         2: "Setup DB"
                             1: "Init project"
```

## File-Level Splitting

### Interactive file selection

```bash
# Split the current commit — opens a checkbox selector
gg split
```

### Split with explicit files

```bash
# Move auth files to a new commit before the current one
gg split -m "Add authentication" src/auth.rs src/auth_test.rs
```

### Split a specific commit in the stack

```bash
# Split commit at position 3
gg split -c 3 src/config.rs

# Split by GG-ID
gg split -c c-abc1234 src/config.rs
```

### Non-interactive with both messages

```bash
gg split -c 2 -m "Extract helpers" --no-edit helpers.rs utils.rs
```

## Hunk-Level Splitting (`-i`)

When you need finer control than whole files, use interactive hunk mode:

```bash
# Force hunk mode for any commit
gg split -i

# Hunk mode on specific files
gg split -i src/auth.rs
```

**Note:** Single-file commits automatically enter hunk mode since file-level splitting wouldn't make sense.

### TUI Mode (Default)

When run interactively with a TTY, `gg split -i` opens a two-panel TUI for hunk selection:

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
| Enter | Confirm selection | Confirm selection |
| q / Esc | Abort (cancel split) | Abort (cancel split) |

#### File Panel Indicators

- `[✓]` — All hunks selected (green)
- `[~]` — Some hunks selected (yellow)
- `[ ]` — No hunks selected

### Sequential Prompt Mode (`--no-tui`)

Use `--no-tui` to fall back to the legacy `git add -p` style sequential prompt:

```bash
gg split -i --no-tui
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

### Hunk Splitting

If a hunk contains multiple logical changes separated by unchanged lines, pressing `s` will break it into smaller hunks. You can then select each sub-hunk individually. If the hunk is already atomic (contiguous changes), you'll see "This hunk cannot be split further."

## Edge Cases

- **All changes selected** — Warning: the original commit will be empty.
- **No changes selected** — Error.
- **Dirty working directory** — Error. Commit or stash changes first.
- **Merge conflicts during rebase** — Split is aborted, original state restored.
