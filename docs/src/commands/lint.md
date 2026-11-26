# `gg lint`

Run configured lint commands on stack commits.

```bash
gg lint [OPTIONS]
```

## Options

- `-u, --until <UNTIL>`: Stop at target entry (position, GG-ID, SHA)
- `--json`: Emit structured JSON output

## Examples

```bash
# Lint from bottom to current
gg lint

# Lint only a subset
gg lint --until 2
```

When lint starts from a detached stack position (for example, after `gg mv 2`), it keeps HEAD detached at that logical position. If lint rewrites the commit, HEAD follows the rewritten entry when its GG-ID identifies it uniquely. If the identity is missing or ambiguous, HEAD returns to the exact original commit instead of attaching to or guessing a stack branch.
