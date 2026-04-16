# `gg log`

Render the current stack as a smartlog: a graph view with a glyph column,
short SHA, GG-ID, commit title, and PR/MR state. The current commit is marked
with a filled glyph (`●`) and a trailing `<- HEAD`.

```bash
gg log [OPTIONS]
```

## Options

- `-r, --refresh`: Refresh PR/MR status from remote before rendering
- `--json`: Print structured JSON output (for scripts and automation). Automatically performs a best-effort refresh of PR/MR state, so `pr_state` and `ci_status` fields are populated without needing `--refresh`.

## Examples

```bash
# Render the current stack as a graph
gg log

# Force-refresh PR/MR state, then render
gg log --refresh

# Structured JSON for automation
gg log --json
```

## JSON output

`gg log --json` emits a `LogResponse` wrapper:

```json
{
  "version": 1,
  "log": {
    "stack": "my-feature",
    "base": "main",
    "current_position": 2,
    "entries": [
      {
        "position": 1,
        "sha": "9012345",
        "title": "Extract storage interface",
        "gg_id": "c-9012345",
        "gg_parent": null,
        "pr_number": 40,
        "pr_state": "open",
        "approved": false,
        "ci_status": "running",
        "is_current": false,
        "in_merge_train": true,
        "merge_train_position": 2
      }
    ]
  }
}
```

`entries` is ordered base → head (`position 1` is the oldest commit, the last
entry is the stack head). `current_position` is 1-indexed and `null` when
`HEAD` is at the stack head or detached. Each entry is the same shape `gg ls
--json` emits, so tooling that parses one parses the other.

## Related

- [`gg ls`](ls.md) — status table view of the same stack (see it as a list)
