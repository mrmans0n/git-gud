# `gg lint`

Run configured lint commands on stack commits.

```bash
gg lint [OPTIONS]
```

## Options

- `-u, --until <UNTIL>`: Stop at target entry (position, GG-ID, SHA)

## Examples

```bash
# Lint from bottom to current
gg lint

# Lint only a subset
gg lint --until 2
```
