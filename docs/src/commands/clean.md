# `gg clean`

Delete merged stacks (and associated managed worktrees).

```bash
gg clean [OPTIONS]
```

## Options

- `-a, --all`: Clean all merged stacks without prompting
- `--json`: Emit machine-readable JSON output

## Examples

```bash
gg clean
gg clean --all
gg clean --json
```

`--json` prints:
- `version`: output schema version
- `clean.cleaned`: stacks that were cleaned
- `clean.skipped`: stacks skipped (unmerged or declined in interactive mode)

When merge verification allows Clean to delete an entry branch from `origin`,
the Clean operation records a `branch_deleted` remote effect with the branch's
exact server-side OID. Deletion uses a matching force-with-lease so a concurrent
server update is never deleted or misreported. `gg undo` refuses local replay
for that operation and prints the exact
`git push origin <prior_oid>:refs/heads/<branch>` recovery command.
