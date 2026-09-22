# `gg rebase`

Rebase the current stack onto an updated branch.

```bash
gg rebase [TARGET]
```

- If `TARGET` is omitted, git-gud uses the stack base branch.

The local base branch is fast-forwarded to `origin/<base>`. If it is checked out
in a worktree, that checkout's index and files are updated together. If the
checkout has uncommitted changes, git-gud warns with its path and skips the
local base update. The stack still rebases onto `origin/<base>`. This also
applies when `gg sync` auto-rebases.

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
