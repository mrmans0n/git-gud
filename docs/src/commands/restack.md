# `gg restack`

Detect and repair ancestry drift in a stack.

After manual git operations (`git commit --amend`, `git rebase -i`, `git cherry-pick`), the Git parent chain can diverge from the GG-Parent metadata chain. `gg restack` compares the two, builds a repair plan, and executes a single `git rebase -i` to realign them.

```bash
gg restack [OPTIONS]
```

## Options

| Option | Short | Description |
|--------|-------|-------------|
| `--dry-run` | `-n` | Show what would be done without executing |
| `--from <TARGET>` | | Only repair from this entry upward (position, SHA, or GG-ID) |
| `--json` | | Output structured JSON |

## How it works

1. Loads the current stack and compares each entry's `GG-Parent` trailer against the expected parent (the `GG-ID` of the entry below it).
2. Entries where the trailer doesn't match the actual parent are marked for **reattach**.
3. If any mismatches are found, runs a single `git rebase -i` to fix the parent chain.
4. After the rebase, normalizes stack metadata (GG-ID and GG-Parent trailers).

## Examples

### Check if a stack needs restacking

```bash
gg restack --dry-run
```

### Repair a stack after amending a commit

```bash
# Amend a commit in the middle of the stack
git commit --amend --no-edit

# Fix the ancestry chain
gg restack
```

### Repair only from position 3 upward

```bash
gg restack --from 3
```

### JSON output

```bash
gg restack --json
```

```json
{
  "version": 1,
  "restack": {
    "stack_name": "my-feature",
    "total_entries": 3,
    "entries_restacked": 1,
    "entries_ok": 2,
    "dry_run": false,
    "steps": [
      {
        "position": 1,
        "gg_id": "c-abc1234",
        "title": "feat: add login",
        "action": "ok",
        "current_parent": null,
        "expected_parent": null
      },
      {
        "position": 2,
        "gg_id": "c-def5678",
        "title": "feat: add dashboard",
        "action": "ok",
        "current_parent": "c-abc1234",
        "expected_parent": "c-abc1234"
      },
      {
        "position": 3,
        "gg_id": "c-fed9876",
        "title": "test: integration tests",
        "action": "reattach",
        "current_parent": "c-abc1234",
        "expected_parent": "c-def5678"
      }
    ]
  }
}
```

## Edge cases

- **Stack already consistent**: Exits with a success message, no rebase performed.
- **Empty stack**: Exits with a message, no action taken.
- **Rebase in progress**: Errors with a message to run `gg continue` or `gg abort` first.
- **Rebase conflict**: Returns the standard conflict error -- resolve with `gg continue` or cancel with `gg abort`.
- **`--from 1`**: Equivalent to a full restack (base = stack base branch).
