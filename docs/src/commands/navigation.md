# Navigation (`mv`, `first`, `last`, `prev`, `next`)

These commands move HEAD within your stack without manual rebase gymnastics.

## `gg mv <TARGET>`

Move to a specific entry by:

- Position (1-indexed)
- GG-ID (`c-...`)
- Commit SHA

```bash
gg mv 1
gg mv c-abc1234
gg mv a1b2c3d
```

## Relative navigation

```bash
gg first   # first entry
gg last    # stack head
gg prev    # previous entry
gg next    # next entry
```
