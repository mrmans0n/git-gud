# `gg undo`

Roll back the most recent locally-undoable mutating operation, or list
recent operations from the operation log.

```bash
gg undo [OPERATION_ID] [OPTIONS]
gg undo --list [OPTIONS]
```

`gg` records every mutating command (sc, drop, reorder, split, absorb,
reconcile, run in amend mode, rebase, sync, land) in a per-repo
**operation log** kept under `.git/gg/operations/` (in the git common dir,
so the same log is visible from every linked worktree). `gg undo` replays
the saved snapshot of refs to put the repository back in the state it was
in before the target operation ran.

## Arguments

- `[OPERATION_ID]` *(optional)*: Target a specific operation by id. When
  omitted, the most recent **locally-undoable** operation is rolled back.
  Operation ids are shown by `gg undo --list`.

## Options

- `--list`: Instead of undoing, list recent operations newest-first.
- `--json`: Emit machine-readable JSON.
- `--limit <N>`: With `--list`, cap the number of operations returned
  (default `100`).

## What gets undone

`gg undo` restores the exact refs the operation captured before it ran:
the active branch, any per-commit branches the operation touched, and
`HEAD`. Working-tree changes are **not** touched — undo operates on commits
and branches, not on files.

Commands that mutate refs and are therefore undoable:

- `gg sc` (squash/amend)
- `gg drop` / `gg abandon`
- `gg reorder` / `gg arrange`
- `gg split`
- `gg absorb`
- `gg reconcile`
- `gg run` (when run in amend mode)
- `gg rebase`
- `gg sync` (only the local-ref portion; see "Refusals" below)
- `gg land` (only the local-ref portion)

### Refusals

`gg undo` refuses to roll back an operation in the following cases:

- **Touched a remote.** Anything that pushed to, merged on, or created a PR
  on the remote (`gg sync`, `gg land`) is not undoable locally — use the
  provider's own tools (`git push --force-with-lease`, closing/reopening
  the PR, `gh pr merge --disable-auto`, etc.). The refusal message includes
  targeted hints.
- **Interrupted.** The operation crashed or was killed before finishing.
  Its record is swept to `Interrupted` and skipped by `gg undo` (the
  partial state it left behind is whatever the crash produced).
- **Stale.** A ref the operation captured has moved since it ran
  (e.g., a manual `git reset`). Undoing would clobber that external
  change, so `gg undo` refuses and tells you which ref diverged.
- **Unsupported schema.** A newer version of `gg` recorded the operation
  with a schema this build does not know how to replay.

## Redo

Redo is modeled as a second `gg undo`. The undo itself is recorded in the
operation log, so `gg undo; gg undo` rolls the undo back and thereby
replays the original change.

## Examples

```bash
# Drop commit 2, then change your mind:
gg drop 2 --force
gg undo                 # commit 2 is back, HEAD is restored

# Undo, then redo (double-undo):
gg undo
gg undo                 # the drop is back in effect

# List recent operations with ids:
gg undo --list

# Target a specific op id from --list:
gg undo 1765432100000_7a3f

# Machine-readable list:
gg undo --list --json --limit 20
```

## `--list` output

Human-readable:

```
ID          KIND          STATUS      UNDOABLE  ARGS
7a3f0001    drop          committed   yes       drop 2 --force
89cc0002    sync          committed   remote    sync
9010003     sc            committed   yes       sc
```

JSON (fields are stable under `version: 1`):

```json
{
  "version": 1,
  "operations": [
    {
      "id": "1765432100000_7a3f",
      "kind": "drop",
      "status": "committed",
      "created_at_ms": 1765432100000,
      "args": ["drop", "2", "--force"],
      "touched_remote": false,
      "is_undoable": true
    }
  ]
}
```

## Concurrency

Mutating commands acquire an exclusive advisory file-lock on
`.git/gg/operation.lock` (in the git common dir) for their entire
duration. Attempts to run a second mutating command — from any worktree of
the same repo — fail fast with a clear "operation already in progress"
error. Read-only commands (`ls`, `log`, `status`, navigation, and
`gg undo --list`) are unaffected.

## JSON output on undo

```json
{
  "version": 1,
  "status": "succeeded",
  "undone": {
    "id": "1765432100000_7a3f",
    "kind": "drop",
    "status": "committed",
    "args": ["drop", "2", "--force"]
  }
}
```

Refusals produce `"status": "refused"` with a `refusal` object carrying
`reason` (`"remote" | "interrupted" | "stale" | "unsupported_schema"`),
a human-readable `message`, the `target` operation, and any `hints`.

## See also

- [Core concepts](../core-concepts.md)
- [MCP Server](../mcp-server.md): `stack_undo` / `stack_undo_list`
