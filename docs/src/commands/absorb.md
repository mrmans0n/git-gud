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
- `-f, --force` (alias `--ignore-immutable`): Override the immutability guard.
  By default, `gg absorb` refuses to run if any commit in the stack is
  merged or reachable from `origin/<base>` — because it cannot tell ahead of
  time whether git-absorb will target those commits. `--dry-run` skips the
  guard so you can preview safely. See
  [Core concepts · Immutable commits](../core-concepts.md#immutable-commits).

## Examples

```bash
# Preview before applying
gg absorb --dry-run

# Absorb and finish with rebase
gg absorb --and-rebase

# Heavy refactor across many files
gg absorb --whole-file --no-limit
```
