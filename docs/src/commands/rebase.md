# `gg rebase`

Rebase the current stack onto an updated branch.

```bash
gg rebase [TARGET]
```

- If `TARGET` is omitted, git-gud uses the stack base branch.

## Options

- `-f, --force` (alias `--ignore-immutable`): Override the immutability guard.
  Rebase rewrites the parent of every commit in the stack; merged commits
  (including squash-merged PRs) and commits already reachable from
  `origin/<base>` are silently skipped — `git rebase` drops them automatically
  via patch-id matching, so `--force` is not required for these. See
  [Core concepts · Immutable commits](../core-concepts.md#immutable-commits).

## Examples

```bash
# Rebase onto configured base
gg rebase

# Rebase onto specific branch
gg rebase main
```
