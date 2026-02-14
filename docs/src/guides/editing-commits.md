# Editing Commits in a Stack

The most common stacked-diff operation is "change commit N without losing N+1, N+2...".

## Navigate to target commit

```bash
gg mv 2
# or use gg first / gg next / gg prev / gg last
```

## Make changes and fold them in

```bash
# after editing files
git add .
gg sc
```

Use `gg sc --all` to include unstaged changes too.

## Reorder commits

Interactive:

```bash
gg reorder
```

Explicit order:

```bash
gg reorder --order "3,1,2"
```

## Absorb scattered staged edits automatically

```bash
gg absorb
```

Useful flags:

- `--dry-run`: preview only
- `--and-rebase`: absorb and rebase in one step
- `--whole-file`: match whole-file changes instead of hunks
- `--squash`: squash fixups directly

After major edits, run:

```bash
gg sync
```
