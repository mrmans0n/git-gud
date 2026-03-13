# `gg split`

Split a commit in the stack into two commits. The selected files become a new commit inserted **before** the original in the stack, while the remaining files stay in the original commit.

```bash
gg split [OPTIONS] [FILES...]
```

## Options

- `-c, --commit <TARGET>`: Target commit — position (1-indexed), short SHA, or GG-ID. Defaults to the current commit (HEAD).
- `-m, --message <MESSAGE>`: Commit message for the new (first) commit. Skips the editor prompt.
- `--no-edit`: Keep the original message for the remainder commit without prompting.
- `FILES...`: Files to include in the new commit. If omitted, opens an interactive file selector.

## How It Works

When you split commit **K** into two:

1. **New commit (K')** — Contains only the selected files. Inserted **before** K in the stack. Gets a new GG-ID.
2. **Remainder (K'')** — Contains the remaining files. Stays in K's original position. Keeps the original GG-ID (preserving PR association).

All descendant commits are automatically rebased onto the remainder.

```
BEFORE                    AFTER
  4: "Fix tests"            5: "Fix tests"       (rebased)
  3: "Add auth+logging"     4: "Add logging"      ← remainder (keeps GG-ID)
  2: "Setup DB"             3: "Add auth"         ← NEW commit (selected files)
  1: "Init project"         2: "Setup DB"
                             1: "Init project"
```

## Examples

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

## Edge Cases

- **All files selected** — Warning: the original commit will be empty.
- **No files selected** — Error.
- **Single-file commit** — Error (use hunk-level splitting in a future version).
- **Dirty working directory** — Error. Commit or stash changes first.
- **Merge conflicts during rebase** — Split is aborted, original state restored.
