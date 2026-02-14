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
