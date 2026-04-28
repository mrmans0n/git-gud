# Troubleshooting / FAQ

## `gh` or `glab` is missing

Install the provider CLI:

- GitHub: <https://cli.github.com/>
- GitLab: <https://gitlab.com/gitlab-org/cli>

## Not authenticated with provider

```bash
gh auth login
glab auth login
```

## "Not on a stack branch"

You're on a branch that doesn't match the stack naming scheme.

```bash
gg co <stack-name>
```

## I pushed with `git push` and now mappings are wrong

Run reconcile:

```bash
gg reconcile --dry-run
gg reconcile
```

## Merge commits are not supported

Stacks require linear history. Rebase your branch:

```bash
git rebase main
```

## `gg land --wait` times out

Increase timeout in config:

```json
{
  "defaults": {
    "land_wait_timeout_minutes": 60
  }
}
```

## When should I use `gg absorb` vs `gg sc`?

- Use `gg sc` when you're on the exact commit you want to modify.
- Use `gg absorb` when staged edits belong to multiple commits and you want git-gud to distribute them.

## How do I undo a `gg` command?

Run [`gg undo`](./commands/undo.md). It reverses the local ref/HEAD
effects of the most recent mutating `gg` command (drop, squash, split, unstack,
rebase, reorder, absorb, reconcile, checkout, nav, clean, sync, land,
or `run --amend`). Working-tree changes are not touched.

```bash
gg undo              # reverse the last local operation
gg undo --list       # see the recent operation log
gg undo <op_id>      # target a specific record
gg undo; gg undo     # undo then redo — a second undo reverses the first
```

Remote-touching operations (`sync`, `land`) are recorded but refused
for local replay. `gg undo` prints a provider-specific revert hint
(e.g. `gh pr close <n>`, `git push --delete …`) instead of silently
rewriting published history. See [`gg undo`](./commands/undo.md) for
the full refusal matrix and JSON schema.
