# `gg setup`

Interactive setup for `.git/gg/config.json`.

```bash
gg setup        # Quick mode: essential settings only
gg setup --all  # Full mode: all settings organized by category
```

Use this when:

- Starting git-gud in a new repository
- Working with self-hosted GitHub/GitLab
- Updating defaults (base branch, username, lint config)

## Quick Mode (default)

Quick mode prompts for only the essential settings:

- **Provider**: GitHub or GitLab
- **Base branch**: Default base branch (main/master/trunk)
- **Username**: Username for branch naming

After completing quick setup, you'll see:

```
Tip: Run 'gg setup --all' to configure advanced options (sync, land, lint, etc.)
```

## Full Mode (`--all`)

Full mode organizes all settings into logical groups:

### General

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `provider` | select | auto-detect | GitHub or GitLab |
| `base` | string | auto-detect | Default base branch (main/master/trunk) |
| `branch_username` | string | from CLI auth | Username for branch naming |
| `unstaged_action` | select | ask | Action for `gg amend` with unstaged changes |

### Sync

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `sync_auto_rebase` | bool | false | Auto-rebase when base is behind origin |
| `sync_behind_threshold` | number | 1 | Commits behind origin before warning/rebase |
| `sync_draft` | bool | false | Create new PRs/MRs as drafts by default |
| `sync_update_descriptions` | bool | true | Update PR/MR descriptions on re-sync |
| `stack_nav_comments` | bool | false | Post a managed navigation comment linking all PRs/MRs in the stack |

### Land

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `land_auto_clean` | bool | false | Auto-clean stack after landing all |
| `land_wait_timeout_minutes` | number | 30 | Timeout for `gg land --wait` |

### Lint

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `lint` | list | empty | Lint commands to run per commit |
| `sync_auto_lint` | bool | false | Run lint automatically before sync |

### Worktrees

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `worktree_base_path` | string | empty | Template for stack worktrees ({repo}, {stack}) |

### GitLab (only shown if provider is GitLab)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `gitlab.auto_merge_on_land` | bool | false | Use "merge when pipeline succeeds" |

## Global Config

git-gud supports global configuration at `~/.config/gg/config.json`. When running `gg setup`:

- If no local config exists, global defaults are shown in prompts
- Local config always takes precedence over global config

This allows you to set organization-wide defaults while allowing per-repo overrides.

All fields are written to `config.json` after setup, making it easy to review and edit configuration manually.

> Note: `auto_add_gg_ids` is deprecated. Existing configs that include it are still read, but setup no longer prompts for it and runtime behavior always treats it as enabled.
