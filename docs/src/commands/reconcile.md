# `gg reconcile`

Reconcile stacks pushed without `gg sync`.

```bash
gg reconcile [OPTIONS]
```

Options:

- `-n, --dry-run`: show what would change

What it can do:

1. Add missing GG-IDs to commits
2. Map existing PRs/MRs back into gg config
