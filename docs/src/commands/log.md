# `gg log`

Show a smartlog-style view of the current stack.

`gg log` is **stack-scoped**: it renders just the current stack as a tree,
with each commit's position, short SHA, title, PR/MR state, CI badge, and a
`<- HEAD` marker on the currently-checked-out commit. For a cross-stack
overview use [`gg ls --all`](./ls.md).

```bash
gg log [OPTIONS]
```

## Options

- `-r, --refresh`: Refresh PR/MR status from the remote before rendering.
- `--json`: Print structured JSON output (for scripts and automation).
  Automatically performs a best-effort refresh of PR/MR state from the
  provider API, so `pr_state` and `ci_status` fields are populated without
  needing `--refresh`.

## Example output

```text
my-feature (3 commits, base: main)

  ├── [1] abc1234 feat: add parser open #101 ✓
  │        #101
  ├── [2] def5678 feat: wire CLI flag open #102 ●
  │        #102
  └── [3] 9abcdef test: coverage for edge cases not pushed  <- HEAD
```

## JSON shape

```json
{
  "version": 1,
  "log": {
    "stack": "my-feature",
    "base": "main",
    "current_position": 3,
    "entries": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "feat: add parser",
        "gg_id": "c-abc1234",
        "gg_parent": null,
        "pr_number": 101,
        "pr_state": "open",
        "approved": false,
        "ci_status": "success",
        "is_current": false,
        "in_merge_train": false,
        "merge_train_position": null
      }
    ]
  }
}
```

Entry fields match [`gg ls --json`](./ls.md) so consumers can share
parsers across both commands.

## See also

- [`gg ls`](./ls.md) — current stack details with more summary metrics, plus
  `--all` and `--remote` modes.
