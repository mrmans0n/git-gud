# `gg drop`

Remove one or more commits from the stack.

For a published entry, running `gg drop` also authorizes a later `gg sync` to
delete that entry's remote branch. The Drop itself remains local: it records
deferred deletion intent, and Sync performs the remote deletion.

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

- `-f, --force` (alias `--ignore-immutable`): Skip the confirmation prompt
  **and** override the immutability guard. `gg drop` refuses by default to
  drop or rewrite commits whose PR is merged or which are already reachable
  from `origin/<base>`; this flag bypasses both safeties. See
  [Core concepts · Immutable commits](../core-concepts.md#immutable-commits).
- `--json`: Output result as JSON

## Behavior

1. Validates the working directory is clean
2. Resolves each target to a commit in the stack
3. Shows which commits will be dropped and asks for confirmation (unless `--force`)
4. Performs a `git rebase -i` that omits the dropped commits
5. Cleans up local per-commit branches for dropped commits
6. Records deferred remote-branch deletion authority for published entries
7. Prints a summary of what was dropped

At least one commit must remain in the stack after dropping.

Deferred deletion authority is bound to the exact remote-tracking OID known
when Drop runs. If no trusted remote version is known, Drop records no
authority and Sync keeps the remote branch. Sync also refuses deletion if the
live branch has moved to another OID; it never adopts that newer version as
authority.

`gg undo` of the Drop restores the local entry and cancels any unconsumed
deletion intent. Undoing that Undo (redoing the Drop) restores the exact saved
intent, including its OID and lifecycle state. Intent already consumed by a
completed Sync is never recreated. An intent left in `deleting` after an
uncertain remote outcome stays uncertain across undo/redo and is not retried
automatically.

When Sync reports a changed branch or uncertain deletion, inspect the named
remote ref and `.git/gg/config.json` before reconciling manually. GG cannot
determine whether an ambiguous server response applied the deletion, and it
will not authorize a retry or a newer branch version on your behalf.

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
