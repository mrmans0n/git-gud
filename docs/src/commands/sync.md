# `gg sync`

Push entry branches and create/update PRs/MRs for the current stack.

```bash
gg sync [OPTIONS]
```

## Options

- `-d, --draft`: Create new PRs/MRs as draft
- `-f, --force`: Force push even if remote is ahead
- `--update-descriptions`: Update PR/MR title/body from commit messages
- `-l, --lint`: Run lint before sync
- `--no-lint`: Disable lint before sync (overrides config default)
- `-u, --until <UNTIL>`: Sync up to target commit (position, GG-ID, or SHA)

## Examples

```bash
# First publish as drafts
gg sync --draft

# Sync only first two entries
gg sync --until 2

# Refresh PR/MR descriptions after commit message edits
gg sync --update-descriptions

# Run lint as part of sync
gg sync --lint
```
