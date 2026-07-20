# `gg unstack`

Split the current stack into two independent stacks.

```bash
gg unstack [--target <TARGET>] [--name <STACK_NAME>] [--no-tui] [-f] [--json] [-w | --keep-current]
```

The selected entry becomes the root of a new stack, and all entries above it
move with it. Entries below the selected point remain in the original stack.

The command is called `unstack` because `gg split` already means splitting one
commit into two commits.

## Examples

```bash
# Pick the split point interactively
gg unstack

# Move entries 3 and above into a new stack named auth-followup
gg unstack --target 3 --name auth-followup --no-tui

# Use a GG-ID or SHA prefix instead of a position
gg unstack --target c-abc1234 --name auth-followup --no-tui

# Machine-readable output
gg unstack --target 3 --json --no-tui

# Put the new upper stack in a managed worktree
gg unstack --target 3 --name upper-auth --wt

# Keep this worktree on the lower stack without creating another worktree
gg unstack --target 3 --name upper-auth --keep-current --json
```

## Behavior

Given a stack:

```text
1  Add schema
2  Add API
3  Add UI
4  Add tests
```

Running `gg unstack --target 3 --name ui-work` leaves:

```text
original stack: 1 Add schema, 2 Add API
new stack:      1 Add UI, 2 Add tests
```

`GG-ID` trailers are preserved. `GG-Parent` trailers are rewritten so the first
entry in each resulting stack has no parent and later entries point at the
previous entry in that same stack.

Local PR/MR mappings in `.git/gg/config.json` move with their entries. Stale
local entry branches under the old stack name are removed for moved entries.
If moved entries had review mappings, run `gg sync` afterwards to recreate or
update review branches for the new stack.

Placement depends on the selected mode:

| Mode | Invoking worktree after success | Additional upper worktree | JSON `current_stack` |
|------|---------------------------------|---------------------------|----------------------|
| default | upper stack | not created | new upper stack name |
| `--worktree` / `--wt` | lower stack | created or reused | original lower stack name |
| `--keep-current` | lower stack | not created | original lower stack name |

`--keep-current` is intended for native clients that must keep using the
invoking worktree but do not want another worktree. It conflicts with
`--worktree`/`--wt`.

With `--json`, `unstack.current_stack` always reports the stack left in the
invoking worktree. `unstack.worktree_path` is present only when `--worktree`
created or reused the upper managed worktree.

## Options

- `--target <TARGET>`: first entry for the new stack. Accepts a 1-indexed
  position, GG-ID, or SHA prefix.
- `--name <STACK_NAME>`: name for the new stack. If omitted, gg generates a
  unique name such as `<old-stack>-2`.
- `--no-tui`: disable the interactive picker. Use this with `--target` in
  scripts and tests.
- `-f, --force`: bypass the immutability guard for merged/base commits.
- `--json`: emit structured JSON.
- `-w, --worktree`: create or reuse a managed worktree for the new stack
  (`--wt` also works).
- `--keep-current`: leave the invoking worktree on the lower stack without
  creating an upper worktree. Conflicts with `--worktree`/`--wt`.

Position `1` is rejected because it would leave the original stack empty. The
last position is allowed and creates a one-entry new stack.
