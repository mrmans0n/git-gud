# `gg sync`

Push entry branches and create/update PRs/MRs for the current stack.

```bash
gg sync [OPTIONS]
```

## Options

- `-d, --draft`: Create new PRs/MRs as draft
- `-f, --force`: Force push even if remote is ahead
- `--update-descriptions`: Update PR/MR title/body from commit messages
- `-l, --lint`: Run lint before sync
- `--no-lint`: Disable lint before sync (overrides config default)
- `--no-rebase-check`: Skip checking whether your stack base is behind `origin/<base>`
- `-u, --until <UNTIL>`: Sync up to target commit (position, GG-ID, or SHA)
- `--json`: Output structured JSON for automation (suppresses human/progress output)

Before pushing, `gg sync` checks whether your stack base is behind `origin/<base>`. If it is behind by at least the configured threshold, git-gud warns and suggests rebasing first (`gg rebase`).

You can control this behavior with config:

- `defaults.sync_auto_rebase` (`sync.auto_rebase`): automatically run `gg rebase` before sync when behind threshold is reached
- `defaults.sync_behind_threshold` (`sync.behind_threshold`): minimum number of commits behind before warning/rebase logic applies (`0` disables the check)

## Examples

```bash
# First publish as drafts
gg sync --draft

# Sync only first two entries
gg sync --until 2

# Refresh PR/MR descriptions after commit message edits
gg sync --update-descriptions

# Run lint as part of sync
gg sync --lint

# Skip behind-base check once
gg sync --no-rebase-check

# Machine-readable output
# (useful in scripts/agents)
gg sync --json
```

Example JSON (shape):

```json
{
  "version": 1,
  "sync": {
    "stack": "my-stack",
    "base": "main",
    "rebased_before_sync": false,
    "entries": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "Add feature",
        "gg_id": "c-abc1234",
        "branch": "user/my-stack--c-abc1234",
        "action": "created",
        "pr_number": 42,
        "pr_url": "https://github.com/org/repo/pull/42",
        "draft": false,
        "pushed": true,
        "error": null
      }
    ]
  }
}
```
