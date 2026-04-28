# Worktrees

`gg co` and `gg unstack` support managed worktrees.

## Create a stack worktree

```bash
gg co my-feature --wt
# or
gg co my-feature --worktree
```

## Unstack into a worktree

```bash
gg unstack --target 3 --name upper-feature --wt
```

This keeps your current directory on the lower stack and creates or reuses a managed worktree for the new upper stack.

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
