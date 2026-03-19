# `gg drop`

Remove one or more commits from the stack.

**Alias:** `gg abandon`

```bash
gg drop <TARGET>... [OPTIONS]
```

## Arguments

- `<TARGET>...`: One or more commits to drop. Each target can be:
  - **Position** (1-indexed): `1`, `3`
  - **Short SHA**: `abc1234`
  - **GG-ID**: `c-abc1234`

## Options

- `-f, --force`: Skip the confirmation prompt
- `--json`: Output result as JSON

## Behavior

1. Validates the working directory is clean
2. Resolves each target to a commit in the stack
3. Shows which commits will be dropped and asks for confirmation (unless `--force`)
4. Performs a `git rebase -i` that omits the dropped commits
5. Cleans up per-commit branches for dropped commits
6. Prints a summary of what was dropped

At least one commit must remain in the stack after dropping.

## Examples

```bash
# Drop the second commit in the stack
gg drop 2

# Drop multiple commits at once
gg drop 1 3

# Drop by GG-ID, skip confirmation
gg drop c-abc1234 --force

# Drop with JSON output
gg drop 2 --force --json

# Use the 'abandon' alias (inspired by jj)
gg abandon 2
```

## JSON Output

```json
{
  "version": 1,
  "drop": {
    "dropped": [
      {"position": 2, "sha": "abc1234", "title": "Fix typo"}
    ],
    "remaining": 3
  }
}
```

## Edge Cases

- **Dropping all commits** produces an error — at least one commit must remain
- **Invalid position** shows the valid range
- **Rebase conflicts** are handled the same as `gg reorder` — resolve with `gg continue` or `gg abort`
