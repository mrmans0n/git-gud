# `gg inbox`

Show an actionable triage view across all stacks.

`gg inbox` scans every stack, refreshes PR/MR status from the remote, and
groups entries into action buckets so you can see at a glance what needs
attention.

```bash
gg inbox [OPTIONS]
```

## Options

- `-a, --all`: Include merged items (default: only actionable items).
- `--json`: Print structured JSON output.

## Action Buckets

Entries are classified in priority order (first match wins):

| Priority | Bucket | Condition |
|----------|--------|-----------|
| 1 | **Merged** | PR/MR state is merged (hidden unless `--all`) |
| 2 | *(skip)* | PR/MR state is closed |
| 3 | **Draft** | PR/MR is a draft |
| 4 | **Changes requested** | Review decision is "changes requested" |
| 5 | **Ready to land** | Approved + CI green + mergeable |
| 6 | **Blocked on CI** | CI failed, running, or pending |
| 7 | **Behind base** | Stack needs rebasing onto newer base commits |
| 8 | **Awaiting review** | Fallthrough — open, not blocked |

## Example output

```text
Refreshing PR status... done

Inbox (4 items across 2 stacks)

Ready to land (1):
  auth #1  abc1234  Add config types  PR #278

Changes requested (1):
  parser #2  def5678  Refactor parser  PR #285

Blocked on CI (1):
  perf #1  9ab0123  Add caching layer  PR #283  ⏳

Awaiting review (1):
  parser #1  456cdef  Add parser types  PR #284
```

## JSON shape

```json
{
  "version": 1,
  "total_items": 4,
  "buckets": {
    "ready_to_land": [
      {
        "stack_name": "auth",
        "position": 1,
        "sha": "abc1234",
        "title": "Add config types",
        "pr_number": 278,
        "pr_url": "https://github.com/user/repo/pull/278",
        "ci_status": "success"
      }
    ],
    "changes_requested": [],
    "blocked_on_ci": [],
    "awaiting_review": [],
    "behind_base": [],
    "draft": []
  }
}
```

The `merged` bucket is omitted from JSON when empty. With `--all`, it
appears as an array of entries.

## See also

- [`gg ls`](./ls.md) — detailed view of a single stack or all stacks.
- [`gg log`](./log.md) — smartlog view of the current stack.
