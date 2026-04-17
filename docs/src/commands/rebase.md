# `gg rebase`

Rebase the current stack onto an updated branch.

```bash
gg rebase [TARGET]
```

- If `TARGET` is omitted, git-gud uses the stack base branch.

## Options

- `-f, --force` (alias `--ignore-immutable`): Override the immutability guard.
  Rebase rewrites the parent of every commit in the stack; if any commit is
  merged (via squash-merge), gg refuses by default to avoid producing local
  duplicates of upstream history. Commits already reachable from
  `origin/<base>` are silently skipped — `git rebase` drops them automatically,
  so `--force` is not required. See
  [Core concepts · Immutable commits](../core-concepts.md#immutable-commits).

## Examples

```bash
# Rebase onto configured base
gg rebase

# Rebase onto specific branch
gg rebase main
```
