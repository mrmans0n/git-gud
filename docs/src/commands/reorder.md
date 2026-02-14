# `gg reorder`

Reorder commits in your stack.

```bash
gg reorder [OPTIONS]
```

## Options

- `-o, --order <ORDER>`: New order as positions/SHAs (`"3,1,2"` or `"3 1 2"`)

## Examples

```bash
# Interactive reorder
gg reorder

# Explicit reorder
gg reorder --order "3,1,2"
```
