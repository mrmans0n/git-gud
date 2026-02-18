# `gg sc`

Squash local changes into the current stack commit.

```bash
gg sc [OPTIONS]
```

## Options

- `-a, --all`: Include staged and unstaged changes

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
```
