# Setup and inspection

## Preconditions

- Know the repository path.
- Read-only inspection requires no mutation authority.

## Procedure

1. Use `gg --help` or `gg <command> --help` only when command or flag
   availability matters.
2. Run `git status --short`.
3. Match the scope with `gg ls --json`, `gg log --json`, or `gg inbox --json`.
4. Run `gg setup` only when the user requested initialization.
5. Create or switch stacks with `gg co -w <stack>` by default.
6. After worktree checkout, verify the active directory because shell
   integration may be absent.

Do not reproduce setup JSON or authentication tutorials. Use CLI prompts and
the [mdBook setup guide](../../../docs/src/commands/setup.md) for explanatory
detail.

## Stop conditions

Stop on a missing repository, unrelated dirty state before a requested
mutation, an ambiguous stack, or missing provider authentication for a remote
operation.

## Verification

Re-run the relevant structured inspection. Confirm the stack, base, provider,
worktree, and `HEAD`.

## Report

Report stack identity, position, dirty state, behind-base state, review summary,
and anything requiring attention.
