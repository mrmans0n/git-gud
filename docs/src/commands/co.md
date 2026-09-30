# `gg co`

Create a new stack, switch to an existing local stack, or check out a remote stack by name.

```bash
gg co [OPTIONS] [STACK_NAME]
```

## Options

- `-b, --base <BASE>`: Base branch to use (default auto-detected: main/master/trunk)
- `-w, --worktree`: Create or reuse a managed worktree for this stack

## Remote stacks

If the stack is not available locally, `gg co` fetches from `origin`. It uses
`origin/<username>/<stack-name>` when that branch exists. Otherwise, it rebuilds
the local stack from `<username>/<stack-name>--<gg-id>` entry branches, choosing
the commit that contains every entry in a linear history above the explicit
`--base`, stack-configured base, or auto-detected base, in that order. GG-ID
sorting does not determine stack order.

If the entry branches have diverged, including when a merge commit joins them,
checkout fails without creating the local stack branch. Resolve the divergent
remote branches into a linear stack, then retry. The same selection applies with
`--worktree`.

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
