# Using Worktrees

Worktrees let you keep your main checkout clean while developing a stack in a dedicated directory.

## Create stack in a managed worktree

```bash
gg co user-auth --worktree
```

To move your shell into the worktree automatically after checkout, enable shell integration:

```bash
# zsh
eval "$(gg init zsh)"

# bash
eval "$(gg init bash)"

# fish
gg init fish | source
```

Without shell integration, `gg co --worktree` prints the worktree path and leaves your shell in the original checkout.

Short flag:

```bash
gg co user-auth -w
```

## Why use worktrees

- Keep your main checkout untouched
- Work on multiple stacks side by side
- Avoid stashing/switching overhead

## Default path behavior

By default git-gud creates:

`../<repo-name>.<stack-name>`

You can change this with `defaults.worktree_base_path` in `.git/gg/config.json`.

## Cleanup behavior

`gg clean` removes merged stacks and associated managed worktrees.
