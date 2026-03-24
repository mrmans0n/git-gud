# `gg sync`

Push entry branches and create/update PRs/MRs for the current stack.

```bash
gg sync [OPTIONS]
```

## Options

- `-d, --draft`: Create new PRs/MRs as draft
- `-f, --force`: Force push even if remote is ahead
- `--update-descriptions`: Update PR/MR title/body from commit messages
- `--update-breadcrumbs`: Add or update stack breadcrumbs in PR/MR descriptions
- `-l, --lint`: Run lint before sync (aborts sync on lint failure and restores repository state to the pre-sync snapshot)
- `--no-lint`: Disable lint before sync (overrides config default)
- `--no-rebase-check`: Skip checking whether your stack base is behind `origin/<base>`
- `-u, --until <UNTIL>`: Sync up to target commit (position, GG-ID, or SHA)
- `--json`: Output structured JSON for automation (suppresses human/progress output)

Before pushing, `gg sync` checks whether your stack base is behind `origin/<base>`. If it is behind by at least the configured threshold, git-gud warns and suggests rebasing first (`gg rebase`).

When you run `gg sync --lint`, lint runs before any push/PR updates. If lint fails, sync aborts immediately and git-gud restores your repository to the pre-sync snapshot.

You can control this behavior with config:

- `defaults.sync_auto_rebase` (`sync.auto_rebase`): automatically run `gg rebase` before sync when behind threshold is reached
- `defaults.sync_behind_threshold` (`sync.behind_threshold`): minimum number of commits behind before warning/rebase logic applies (`0` disables the check)

## Stack breadcrumbs

When you pass `--update-breadcrumbs`, each PR/MR description gets a navigation block showing where it sits in the stack:

```markdown
<!-- gg:breadcrumbs:start -->
**Stack:** `auth-feature` — 2/3

PR #10 ← THIS → PR #12

1. Add auth module — #10
2. Add login page — #11 **⮜**
3. Add logout button — #12
<!-- gg:breadcrumbs:end -->
```

Breadcrumbs are **idempotent**: re-running `gg sync --update-breadcrumbs` replaces only the managed block between the HTML comment markers. Any text you write outside the markers is preserved.

## Examples

```bash
# First publish as drafts
gg sync --draft

# Sync only first two entries
gg sync --until 2

# Refresh PR/MR descriptions after commit message edits
gg sync --update-descriptions

# Add stack navigation breadcrumbs to all PRs/MRs
gg sync --update-breadcrumbs

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
    ],
    "breadcrumbs": {
      "enabled": true,
      "updated": 1,
      "unchanged": 0
    }
  }
}
```

The `breadcrumbs` field is present only when `--update-breadcrumbs` is used.
