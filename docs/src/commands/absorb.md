# `gg absorb`

Automatically distribute staged changes to the most appropriate commits in your stack.

```bash
gg absorb [OPTIONS]
```

## Options

- `--dry-run`: Preview actions without changing commits
- `-a, --and-rebase`: Rebase automatically after creating fixups
- `-w, --whole-file`: Match and absorb by whole file rather than hunks
- `--one-fixup-per-commit`: At most one fixup per commit
- `-n, --no-limit`: Search all commits in the stack (not just last 10)
- `-s, --squash`: Squash directly instead of creating `fixup!` commits

## Examples

```bash
# Preview before applying
gg absorb --dry-run

# Absorb and finish with rebase
gg absorb --and-rebase

# Heavy refactor across many files
gg absorb --whole-file --no-limit
```
