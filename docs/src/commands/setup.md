# `gg setup`

Interactive setup for `.git/gg/config.json`.

```bash
gg setup
```

Use this when:

- Starting git-gud in a new repository
- Working with self-hosted GitHub/GitLab
- Updating defaults (base branch, username, lint config)

## Configuration Fields

The setup wizard prompts for all available options:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `provider` | select | auto-detect | GitHub or GitLab |
| `base` | string | auto-detect | Default base branch (main/master/trunk) |
| `branch_username` | string | from CLI auth | Username for branch naming |
| `lint` | list | empty | Lint commands to run per commit |
| `sync_auto_lint` | bool | false | Run lint automatically before sync |
| `auto_add_gg_ids` | bool | true | Auto-add GG-IDs to commits |
| `sync_auto_rebase` | bool | false | Auto-rebase when base is behind origin |
| `sync_behind_threshold` | number | 1 | Commits behind origin before warning/rebase |
| `land_auto_clean` | bool | false | Auto-clean stack after landing all |
| `land_wait_timeout_minutes` | number | 30 | Timeout for `gg land --wait` |
| `unstaged_action` | select | ask | Action for `gg amend` with unstaged changes |
| `worktree_base_path` | string | empty | Template for stack worktrees ({repo}, {stack}) |

### GitLab-only

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `gitlab.auto_merge_on_land` | bool | false | Use "merge when pipeline succeeds" |

All fields are written to `config.json` after setup, making it easy to review and edit configuration manually.
