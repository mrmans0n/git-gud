# `gg reconcile`

Repair stack metadata when branches/PRs were manipulated outside `gg sync`.

```bash
gg reconcile [OPTIONS]
```

## Options

- `-n, --dry-run`: Preview only; make no changes

## What it does

- Adds missing GG-ID trailers to stack commits
- Maps existing remote PRs/MRs back to local stack entries

## Examples

```bash
# Safe preview
gg reconcile --dry-run

# Apply reconciliation
gg reconcile
```
