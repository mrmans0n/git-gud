# `gg sc`

Squash local changes into the current stack commit.

```bash
gg sc [OPTIONS]
```

## Options

- `-a, --all`: Include staged and unstaged changes

## Examples

```bash
# Standard amend-like flow
git add .
gg sc

# Include unstaged changes too
gg sc --all
```
