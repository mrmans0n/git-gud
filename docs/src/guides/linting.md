# Linting Your Stack

`gg lint` runs your configured lint commands commit-by-commit across the stack.

## Configure lint commands

In `.git/gg/config.json`:

```json
{
  "defaults": {
    "lint": [
      "cargo fmt --check",
      "cargo clippy -- -D warnings"
    ]
  }
}
```

## Run lint manually

```bash
gg lint
```

Run only up to a specific entry:

```bash
gg lint --until 2
```

## Run lint during sync

```bash
gg sync --lint
```

Skip lint for one sync (even if enabled by default):

```bash
gg sync --no-lint
```
