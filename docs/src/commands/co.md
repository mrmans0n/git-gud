# `gg co`

Create a new stack or switch to an existing one.

```bash
gg co [OPTIONS] [STACK_NAME]
```

Options:

- `-b, --base <BASE>`: base branch for the stack
- `-w, --worktree`: create/reuse a managed worktree for this stack

Examples:

```bash
gg co user-auth
gg co user-auth --base main
gg co user-auth --worktree
```
