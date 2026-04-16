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

## I just ran the wrong command — can I undo it?

Yes, for any operation that did not touch a remote. `gg undo` rolls back the
most recent local mutation (drop, reorder, sc, split, absorb, reconcile,
run amend, rebase) by restoring the refs and HEAD as they were before the
command ran. See [`gg undo`](./commands/undo.md) for the full list, the
refusal rules, and how to redo (a second `gg undo`). List recent operations
with `gg undo --list` and target one by id with `gg undo <id>`.

Operations that touched the remote (`gg sync`, `gg land`) are refused — use
your provider's tooling (`git push --force-with-lease`,
`gh pr merge --disable-auto`, reopening a PR, etc.) to reverse those.

## "Another gg operation is already running"

Mutating `gg` commands (sc, drop, split, reorder, absorb, reconcile, run,
rebase, sync, land) take an exclusive lock on `.git/gg/operation.lock` for
their whole duration, including across linked worktrees of the same
repository. If a command fails fast with an "operation already in progress"
error, another mutating command is running. Read-only commands
(`ls`, `log`, `status`, navigation, `gg undo --list`) are never blocked by
the lock.

If a previous mutating command crashed, the next mutating invocation sweeps
the stale record to `Interrupted` and proceeds normally — no manual
cleanup is needed.
