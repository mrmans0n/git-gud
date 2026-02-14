# `gg sync`

Push stack branches and create/update PRs/MRs.

```bash
gg sync [OPTIONS]
```

Options:

- `-d, --draft`: create new PRs/MRs as draft
- `-f, --force`: force push when remote is ahead
- `--update-descriptions`: update titles/descriptions from commit messages
- `-l, --lint`: run lint before sync
- `--no-lint`: disable lint before sync
- `-u, --until <UNTIL>`: sync only up to target commit

Example:

```bash
gg sync --draft
gg sync --until 2
gg sync --update-descriptions
```
