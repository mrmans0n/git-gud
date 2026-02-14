# `gg rebase`

Rebase the current stack onto an updated branch.

```bash
gg rebase [TARGET]
```

- If `TARGET` is omitted, git-gud uses the stack base branch.

## Examples

```bash
# Rebase onto configured base
gg rebase

# Rebase onto specific branch
gg rebase main
```
