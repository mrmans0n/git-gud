# `gg sc`

Squash local changes into the current stack commit.

```bash
gg sc [OPTIONS]
```

## Options

- `-a, --all`: Include staged and unstaged changes
- `--staged-only`: Include staged changes only and ignore
  `defaults.unstaged_action`. Unstaged and untracked files are never staged or
  stashed. Conflicts with `--all`. A mid-stack amend refuses before mutation
  when tracked changes are unstaged or any untracked files are present, since
  either can prevent rebasing descendants safely.
- `-f, --force` (alias `--ignore-immutable`): Override the immutability guard.
  By default `gg sc` refuses to amend a commit whose PR is merged or which is
  already reachable from `origin/<base>`. See
  [Core concepts · Immutable commits](../core-concepts.md#immutable-commits).

When unstaged changes are present, behavior is controlled by `defaults.unstaged_action` in `.git/gg/config.json`:

- `ask` (default): prompt to stage all, stash, continue, or abort
- `add`: auto-stage all changes (`git add -A`) and continue
- `stash`: auto-stash and continue
- `continue`: continue without including unstaged changes
- `abort`: fail immediately

## Examples

```bash
# Standard amend-like flow
git add .
gg sc

# Include unstaged changes too
gg sc --all

# Native-client flow: amend only the prepared index
gg sc --staged-only
```
