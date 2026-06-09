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

## Insert a commit in the middle of a stack

Sometimes you realize a commit is missing between two existing ones. Use `gg mv` to navigate there, make a normal `git commit`, then run `gg restack` to fold it in.

```bash
gg co testing
git commit -m "one"
git commit -m "two"

# navigate to position 1 (HEAD detaches)
gg mv 1

# make the new commit
git add <files>
git commit -m "inserted"

# gg ls shows the un-integrated commit rather than losing it:
#   ⚠ Un-integrated commit at HEAD (detached):
#       3fb873d inserted  — sits on top of [1]
#     Run `gg restack` to fold it into the stack.
gg ls

# fold it in — stack becomes: one, inserted, two
# HEAD stays on the inserted commit when restack finishes
gg restack
```

`gg ls` is read-only: it never mutates the stack. The "un-integrated commit" callout is purely informational — the commit is not lost, it just isn't part of the stack yet. `gg restack` is what integrates it.

The same flow works when you `git commit --amend` the navigated commit instead of creating a new one. In that case `gg restack` rewrites the commit in place and replays everything above it on top.
