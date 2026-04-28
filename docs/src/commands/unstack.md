# unstack

`gg unstack` splits the current stack into two independent stacks.

Use this when the upper part of a stack has become a separate line of work. The selected commit becomes the first commit in the new stack, and every descendant moves with it. Commits below the selected commit remain in the original stack.

This is named `unstack` because `gg split` already means splitting one commit into two commits.

```text
main
  A
  B
  C  <- selected target
  D
```

After `gg unstack --target 3 --name upper`:

```text
main
  A
  B          original stack

main
  C
  D          new stack: upper
```

## Usage

```bash
gg unstack
gg unstack --target 3
gg unstack --target c-abc1234 --name auth-cleanup
gg unstack --target abc123 --force
gg unstack --target 2 --json
```

Without `--target`, `gg unstack` opens a TUI picker when the terminal is interactive. The first commit is not a valid target because the original stack would be empty.

## Options

- `-t, --target <TARGET>`: First commit to move into the new stack. Accepts a position, short SHA, or GG-ID.
- `-n, --name <NAME>`: Name for the new stack. Defaults to `<current-stack>-2`, or the next free suffix.
- `--no-tui`: Disable the picker. Requires `--target`.
- `-f, --force` / `--ignore-immutable`: Bypass the immutability guard for moved commits.
- `--json`: Emit structured output.

## Remote PRs/MRs

`gg unstack` is local-only. It updates local stack metadata and moves any PR/MR mappings for the moved GG-IDs to the new stack, but it does not push branches or retarget PRs/MRs.

Run `gg sync` after unstacking to push the new stack and update PR/MR targets.

## JSON Output

```json
{
  "version": 1,
  "unstack": {
    "old_stack": "my-stack",
    "new_stack": "my-stack-2",
    "target_position": 3,
    "moved": [
      {
        "old_position": 3,
        "sha": "abc1234",
        "gg_id": "c-abc1234",
        "title": "Add auth"
      }
    ],
    "old_stack_count": 2,
    "new_stack_count": 2
  }
}
```
