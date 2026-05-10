# `gg co`

Create a new stack, switch to an existing local stack, or check out a remote stack by name.

```bash
gg co [OPTIONS] [STACK_NAME]
```

## Options

- `-b, --base <BASE>`: Base branch to use (default auto-detected: main/master/trunk)
- `-w, --worktree`: Create or reuse a managed worktree for this stack

## Examples

```bash
# Create/switch stack
gg co user-auth

# Create stack based on a specific branch
gg co user-auth --base develop

# Create stack in worktree
gg co user-auth --worktree
```

With shell integration enabled, `gg co user-auth --worktree` also changes your current shell directory to the stack worktree after the command succeeds:

```bash
eval "$(gg init zsh)"  # or bash
gg init fish | source # fish
```

Without shell integration, git-gud prints the worktree path and leaves your shell in the original checkout.
