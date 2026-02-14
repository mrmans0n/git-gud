# Reconciling Out-of-Sync Stacks

Use reconcile when stack metadata and remote state diverge.

Common causes:

- Someone pushed with `git push` instead of `gg sync`
- A stack was edited across machines and mappings got stale
- Commits exist without GG-ID trailers

## Preview changes safely

```bash
gg reconcile --dry-run
```

## Apply reconciliation

```bash
gg reconcile
```

Reconcile can:

1. Add missing GG-IDs to stack commits (via rebase)
2. Map existing PRs/MRs to the right GG-IDs in config

After reconciling, run:

```bash
gg ls --refresh
gg sync
```
