# Worktrees

`gg co` supports managed worktrees.

## Create a stack worktree

```bash
gg co my-feature --wt
# or
gg co my-feature --worktree
```

## Default location

By default, worktrees are created next to your repository using:

`../<repo-name>.<stack-name>`

Example:

- repo: `/code/my-repo`
- stack: `user-auth`
- worktree: `/code/my-repo.user-auth`

## Configure worktree base path

Set `defaults.worktree_base_path` in `.git/gg/config.json`:

```json
{
  "defaults": {
    "worktree_base_path": "/tmp/gg-worktrees"
  }
}
```

## Visibility and cleanup

- `gg ls` / `gg ls --all` marks worktree stacks with `[wt]`
- `gg clean` also removes associated managed worktrees
