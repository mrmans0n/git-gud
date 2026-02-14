# `gg absorb`

Absorb staged changes into the appropriate commits.

```bash
gg absorb [OPTIONS]
```

Options:

- `--dry-run`: preview without making changes
- `-a, --and-rebase`: rebase automatically after creating fixup commits
- `-w, --whole-file`: absorb whole files (not per-hunk)
- `--one-fixup-per-commit`: at most one fixup per commit
- `-n, --no-limit`: search all commits in stack
- `-s, --squash`: squash directly instead of creating `fixup!` commits
