# Configuration

git-gud uses a layered configuration system:

1. **Global config**: `~/.config/gg/config.json` — shared defaults across all repos
2. **Local config**: `.git/gg/config.json` — per-repository settings

Local config always takes precedence over global config.

## Setup

Initialize or update local config with:

```bash
gg setup        # Quick mode: essential settings only
gg setup --all  # Full mode: all settings organized by category
```

For global config, manually create `~/.config/gg/config.json` with your preferred defaults.

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
    "unstaged_action": "ask",
    "land_wait_timeout_minutes": 30,
    "land_admin": false,
    "land_auto_clean": false,
    "sync_auto_lint": false,
    "sync_auto_rebase": false,
    "sync_behind_threshold": 1,
    "sync_draft": false,
    "sync_update_descriptions": true,
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
| `auto_add_gg_ids` | `boolean` | **Deprecated** compatibility field. gg always enforces GG metadata normalization, regardless of this value. | `true` |
| `unstaged_action` | `string` | Default behavior for `gg sc`/`gg amend` when unstaged changes exist: `ask`, `add`, `stash`, `continue`, or `abort` | `ask` |
| `land_wait_timeout_minutes` | `number` | Timeout for `gg land --wait` polling | `30` |
| `land_admin` | `boolean` | Use admin privileges to bypass approval requirements on land (GitHub only) | `false` |
| `land_auto_clean` | `boolean` | Auto-run cleanup after full landing | `false` |
| `sync_auto_lint` | `boolean` | Automatically run `gg lint` before `gg sync` | `false` |
| `sync_auto_rebase` | `boolean` | Automatically run `gg rebase` before `gg sync` when behind threshold is reached | `false` |
| `sync_behind_threshold` | `number` | Warn/rebase in `gg sync` when base is at least this many commits behind `origin/<base>` (`0` disables check) | `1` |
| `sync_draft` | `boolean` | Create new PRs/MRs as drafts by default | `false` |
| `sync_update_descriptions` | `boolean` | Update PR/MR descriptions on re-sync | `true` |
| `worktree_base_path` | `string` | Base directory for managed worktrees | Parent of repo |
| `gitlab.auto_merge_on_land` | `boolean` | Default GitLab auto-merge behavior for `gg land` | `false` |

## Global Config

Store shared defaults in `~/.config/gg/config.json`. This is useful for:

- Organization-wide settings (e.g., always use drafts)
- Personal preferences that apply to all your repos
- Reducing repetitive setup across multiple repositories

Example global config:

```json
{
  "defaults": {
    "sync_draft": true,
    "land_auto_clean": true,
    "sync_behind_threshold": 5
  }
}
```

When `gg setup` runs in a new repo, these global defaults will be shown in prompts. You can accept them or override per-repo.

## Stack state

git-gud also stores stack-specific state in the local config file (for example PR/MR mappings by GG-ID). This is how it remembers which commit corresponds to which PR/MR over time.

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
