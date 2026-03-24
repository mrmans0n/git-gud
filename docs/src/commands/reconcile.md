# `gg reconcile`

Repair stack metadata when branches/PRs were manipulated outside `gg sync`.

```bash
gg reconcile [OPTIONS]
```

## Options

- `-n, --dry-run`: Preview only; make no changes

## What it does

- Normalizes GG metadata trailers on stack commits (`GG-ID` + `GG-Parent`)
- Maps existing remote PRs/MRs back to local stack entries

## Examples

```bash
# Safe preview
gg reconcile --dry-run

# Apply reconciliation
gg reconcile
```
