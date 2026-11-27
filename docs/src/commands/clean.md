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
server update is never deleted or misreported. Each remote branch deletion has
its own durable operation segment: uncertainty is saved before the request, and
the exact effect is saved and finalized before Clean reports success or starts
the next deletion. Clean also saves the branch and expected OID in
`pending_remote_branch_deletions` before a direct request. If the request outcome
is ambiguous, Clean leaves that entry in `deleting`, stops, and blocks later
automatic retries until you reconcile the remote ref and local records. Managed
worktree removal clears and durably saves its configured path, then finalizes its
local operation segment before printing the unchanged `Removed worktree` success
message. A retry reconciles an already removed, unregistered worktree after a
failed config save. `gg undo` refuses local replay for a completed remote deletion
and prints the exact `git push origin <prior_oid>:refs/heads/<branch>` recovery
command.
