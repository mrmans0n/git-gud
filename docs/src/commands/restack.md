# `gg restack`

Repair stack ancestry after manual history changes (amend, cherry-pick, upstream rebase).

```bash
gg restack [OPTIONS]
```

## Options

- `-n, --dry-run`: Show what would be done without making changes
- `--from <TARGET>`: Repair only from this commit upward (position, SHA, or GG-ID)
- `--json`: Output result as JSON

## Behavior

1. Validates no rebase is already in progress
2. Validates the working directory is clean
3. Requires all commits to have GG-IDs (directs to `gg reconcile` if missing)
4. Compares each entry's `GG-Parent` trailer against the expected parent
5. If all parents match, reports "Stack is already consistent"
6. If `--dry-run`, displays the plan and exits
7. Performs a single `git rebase -i` to realign the chain
8. Normalizes GG metadata after rebase
9. Prints a summary with a hint to run `gg sync`

## Examples

```bash
# Check if the stack needs restacking (no changes)
gg restack --dry-run

# Repair the full stack
gg restack

# Repair only from position 3 upward
gg restack --from 3

# Repair from a specific GG-ID upward
gg restack --from c-abc1234

# Get JSON output
gg restack --json

# Combine dry-run and JSON
gg restack --dry-run --json
```

## JSON Output

```json
{
  "version": 1,
  "restack": {
    "stack_name": "my-feature",
    "total_entries": 4,
    "entries_restacked": 2,
    "entries_ok": 2,
    "dry_run": false,
    "steps": [
      {
        "position": 1,
        "gg_id": "c-abc1234",
        "title": "Add login form",
        "action": "ok",
        "current_parent": null,
        "expected_parent": null
      },
      {
        "position": 2,
        "gg_id": "c-def5678",
        "title": "Add validation",
        "action": "reattach",
        "current_parent": "c-old1111",
        "expected_parent": "c-abc1234"
      }
    ]
  }
}
```

## Inserting a commit mid-stack

`gg restack` also integrates commits you make at a detached HEAD after navigating to a mid-stack position with `gg mv`:

```bash
gg mv 1             # detach HEAD at position 1
git commit -m "inserted"
gg restack          # folds "inserted" into the stack; HEAD stays on it
```

After `gg restack` completes, HEAD is left on the just-inserted (or just-amended) commit. Run `gg sync` to push the updated stack.

> **Note:** If folding the commit in hits a conflict, resolve it and run `gg continue` (or `gg abort`). The conflict path goes through Git's normal rebase-continue, so afterwards you are returned to the stack head rather than left on the integrated commit, and the freshly folded commit may not yet have its `GG-ID`/`GG-Parent` assigned. The commit is still folded in correctly — run `gg sync` (or `gg reconcile`) to finish normalizing the metadata, and `gg mv` to navigate back to it if you want to keep working there. (This matches how `gg restack` behaves after any rebase conflict.)

## Edge Cases

- **Empty stack** produces an error
- **Missing GG-IDs** directs to `gg reconcile`
- **Rebase in progress** blocks with a clear error message
- **Rebase conflicts** are handled the same as `gg reorder` — resolve with `gg continue` or `gg abort`
- **`--from 1`** is equivalent to a full restack
- **Detached HEAD with un-integrated commits** — `gg restack` detects and folds them in automatically
