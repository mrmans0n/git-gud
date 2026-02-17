# Configuration

git-gud stores config per repository in `.git/gg/config.json`.

Initialize or update it with:

```bash
gg setup
```

## Example config

```json
{
  "defaults": {
    "provider": "gitlab",
    "base": "main",
    "branch_username": "your-username",
    "lint": [
      "cargo fmt --check",
      "cargo clippy -- -D warnings"
    ],
    "auto_add_gg_ids": true,
    "land_wait_timeout_minutes": 30,
    "land_auto_clean": false,
    "sync_auto_lint": false,
    "sync_auto_rebase": false,
    "sync_behind_threshold": 1,
    "worktree_base_path": "/tmp/gg-worktrees",
    "gitlab": {
      "auto_merge_on_land": false
    }
  }
}
```

## `defaults` options

| Option | Type | What it controls | Default |
|---|---|---|---|
| `provider` | `string` | Provider (`github`/`gitlab`) for self-hosted or explicit override | Auto-detected |
| `base` | `string` | Default base branch for new stacks | Auto-detected |
| `branch_username` | `string` | Username prefix in stack/entry branch names | Auto-detected |
| `lint` | `string[]` | Commands used by `gg lint` / `gg sync --lint` | `[]` |
| `auto_add_gg_ids` | `boolean` | Auto-add GG-ID trailers when missing | `true` |
| `land_wait_timeout_minutes` | `number` | Timeout for `gg land --wait` polling | `30` |
| `land_auto_clean` | `boolean` | Auto-run cleanup after full landing | `false` |
| `sync_auto_lint` | `boolean` | Automatically run `gg lint` before `gg sync` | `false` |
| `sync_auto_rebase` (`sync.auto_rebase`) | `boolean` | Automatically run `gg rebase` before `gg sync` when behind threshold is reached | `false` |
| `sync_behind_threshold` (`sync.behind_threshold`) | `number` | Warn/rebase in `gg sync` when base is at least this many commits behind `origin/<base>` (`0` disables check) | `1` |
| `worktree_base_path` | `string` | Base directory for managed worktrees | Parent of repo |
| `gitlab.auto_merge_on_land` | `boolean` | Default GitLab auto-merge behavior for `gg land` | `false` |

## Stack state

git-gud also stores stack-specific state in this file (for example PR/MR mappings by GG-ID). This is how it remembers which commit corresponds to which PR/MR over time.

## PR/MR templates

You can customize descriptions by creating `.git/gg/pr_template.md`.

Supported placeholders:

- `{{title}}`
- `{{description}}`
- `{{stack_name}}`
- `{{commit_sha}}`

Example:

```markdown
## Summary

{{description}}

---

**Stack:** `{{stack_name}}`
**Commit:** `{{commit_sha}}`
```
