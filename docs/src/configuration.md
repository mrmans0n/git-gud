# Configuration

Configuration is stored in `.git/gg/config.json` (per-repository).

Generate it interactively:

```bash
gg setup
```

## Example

```json
{
  "defaults": {
    "provider": "gitlab",
    "base": "main",
    "branch_username": "your-username",
    "lint": ["cargo fmt --check", "cargo clippy -- -D warnings"]
  }
}
```

## Common options (`defaults`)

- `provider`: `github` or `gitlab`
- `base`: default base branch
- `branch_username`: prefix for stack branches
- `lint`: commands for `gg lint`
- `auto_add_gg_ids`: auto-add GG-IDs to commits
- `land_wait_timeout_minutes`: timeout for `gg land --wait`
- `land_auto_clean`: auto cleanup after full landing
- `worktree_base_path`: base path for managed worktrees
- `gitlab.auto_merge_on_land`: default auto-merge behavior on GitLab

## PR/MR description template

If `.git/gg/pr_template.md` exists, `gg sync` uses it for newly created PR/MR descriptions.

Supported placeholders:

- `{{title}}`
- `{{description}}`
- `{{stack_name}}`
- `{{commit_sha}}`
