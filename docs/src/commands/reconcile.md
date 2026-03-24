# `gg reconcile`

Repair stack metadata when branches/PRs were manipulated outside `gg sync`.

```bash
gg reconcile [OPTIONS]
```

## Options

- `-n, --dry-run`: Preview only; make no changes

## What it does

- Normalizes GG-ID and GG-Parent trailers on all stack commits
- Adds missing GG-IDs, fixes stale GG-Parent chains
- Maps existing remote PRs/MRs back to local stack entries

## Examples

```bash
# Safe preview
gg reconcile --dry-run

# Apply reconciliation
gg reconcile
```
