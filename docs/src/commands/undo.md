# `gg undo`

Reverse the local ref/HEAD effects of the most recent mutating `gg`
command, backed by a per-repo operation log.

```bash
gg undo [OPERATION_ID] [--json]
gg undo --list [--limit N] [--json]
```

`gg undo` only moves refs and `HEAD` — it never modifies your working
tree, working-copy files, or the index. The log lives at
`<commondir>/gg/operations/*.json` and keeps the last **100 records**;
`Pending` records (operations interrupted by a crash, Ctrl-C, or a
long-running conflict) are never pruned.

## Options

- `OPERATION_ID`: Target a specific record (`op_…`). When omitted,
  undoes the most recent locally-undoable operation.
- `--list`: Show recent operations with id, kind, status, timestamp,
  and undoability marker.
- `--limit N`: Cap `--list` output (default: 20).
- `--json`: Emit machine-readable JSON.

## How it works

Every mutating `gg` command (`sc`, `drop`, `split`, `unstack`, `rebase`, `reorder`,
`absorb`, `reconcile`, `checkout`, `mv`/`first`/`last`/`prev`/`next`,
`clean`, `sync`, `land`, and `run --amend`) now snapshots the refs it
will touch before mutating and finalises the record on success. `gg
undo` replays the `refs_before` snapshot of the target record, moving
refs back to where they were.

A second `gg undo` redoes the first — because `undo` is itself
recorded as an operation, running it twice reverses the reversal.
Entries created by `gg undo` appear in `--list` with a `↶` marker and
an `undoes` field pointing at the original operation id.

## Refusal modes

`gg undo` refuses (exit 1, no refs touched) when:

| Reason | Condition | What to do |
|---|---|---|
| `remote` | The target op pushed/merged/closed/created a PR or MR. | Use the printed provider hint (`gh pr close <n>`, `glab mr close <n>`, `git push --delete …`). Local state is unchanged. |
| `interrupted` | The op crashed or was Ctrl-C'd mid-flight and has `status: Interrupted`. | Fix the underlying state manually; the stale Pending record is swept into Interrupted on the next lock-acquiring op. |
| `stale` | Refs have moved since the target op finalised. | Run `gg undo --list` and target a more recent record instead. The error names the ref, the expected OID, and the actual OID. |
| `unsupported_schema` | The record was written by a newer `gg` with a schema version this binary does not understand. | Upgrade `gg` or delete the offending record. |

## Examples

```bash
# Reverse the last local operation
gg undo

# See what's on the log (newest first)
gg undo --list
gg undo --list --limit 5

# Target a specific record from --list
gg undo op_0000001750000000_018f…

# Redo: undo twice in a row
gg undo
gg undo

# Scripting
gg undo --list --json | jq '.operations[] | select(.is_undoable)'
```

## JSON output

Schema versioning: all `gg` JSON responses share the top-level `version`
field (`OUTPUT_VERSION`). The undo types are additive — new optional
fields may be added in future releases without bumping the version.
Forward-compatible consumers should ignore unknown fields.

### `gg undo --json`

```json
{
  "version": 1,
  "status": "succeeded",
  "undone": {
    "id": "op_0000001750000000_018f…",
    "kind": "drop",
    "status": "committed",
    "created_at_ms": 1750000000000,
    "args": ["drop", "3"],
    "stack_name": "feat/login",
    "touched_remote": false,
    "is_undoable": true,
    "is_undo": false,
    "remote_effects": []
  }
}
```

On refusal:

```json
{
  "version": 1,
  "status": "refused",
  "refusal": {
    "reason": "remote",
    "message": "Cannot locally undo 'sync': it touched a remote.",
    "target": { "id": "op_…", "kind": "sync", "touched_remote": true, "…": "…" },
    "hints": [
      "Close PR #42: gh pr close 42",
      "Delete remote branch: git push --delete origin nacho/feat/1"
    ]
  }
}
```

`refusal.reason` is one of `remote`, `interrupted`, `stale`,
`unsupported_schema`.

### `gg undo --list --json`

```json
{
  "version": 1,
  "operations": [
    {
      "id": "op_…",
      "kind": "undo",
      "status": "committed",
      "created_at_ms": 1750000100000,
      "args": ["undo"],
      "stack_name": "feat/login",
      "touched_remote": false,
      "is_undoable": true,
      "is_undo": true,
      "undoes": "op_previous…"
    },
    { "…": "…" }
  ]
}
```

Operations are returned newest-first. Use `is_undoable` to gate
UI/agent actions; use `is_undo` + `undoes` to render redo markers.

## What `gg undo` does NOT do

- It does **not** restore working-tree files or the index. If you amended
  a commit and want the old source back, use `git reflog` or
  `git stash`.
- It does **not** touch remotes. Operations that pushed, merged, closed,
  or created PRs/MRs are recorded (so you can see them in `--list`) but
  refused for local replay.
- It does **not** guarantee atomicity of the replay. If the process
  dies mid-replay, a second `gg undo` will finish the job — the
  working tree is clean throughout, so only refs move.
- It does **not** support an `--all` / `--range` mode. Each call
  reverses exactly one operation.

## See also

- [`gg log`](./log.md) — smartlog view of the current stack
- [MCP server](../mcp-server.md) — `stack_undo` and `stack_undo_list`
  tools for agentic workflows
