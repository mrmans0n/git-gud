# Editing stacks

## Preconditions

- Run `git status --short` and inspect structured stack order before selecting
  a target.
- Confirm the requested edit authorizes local history mutation.
- Before every rewrite, surface `ImmutableTargets`. Require explicit approval
  before `--force` or `--ignore-immutable`.

## Procedure

- When a client has already prepared the index, run `gg sc --staged-only`.
- For staged fixes spanning multiple commits, prefer `gg absorb -s`.
- Keep ordinary terminal `gg split` interactive. For native Describe/Apply
  split, use [native clients](native-clients.md).
- To insert mid-stack, run `gg mv <target>`, create or amend the commit, then
  run `gg restack`. Confirm `unintegrated_commits` is empty afterward.
- For non-interactive drop, reorder, and unstack, supply explicit targets and
  order.
- Use `gg unstack --keep-current --json` only for native clients that must
  retain the lower stack in the current worktree. Otherwise prefer `--wt`.
- Run `gg run -- <command>` read-only by default. Use `--amend` only when each
  command's changes should be folded into its commit; use `--discard` only when
  each command's changes should be discarded.

## Stop conditions

Stop on unrelated dirty state, ambiguous targets, immutable targets without
approval, or unresolved conflicts.

## Verification

After rewrites, re-inspect stack order, `HEAD`, and the working tree. Confirm
whether publishing remains requested.

## Report

Report the targeted commits, local rewrites performed, resulting order and
position, dirty or conflict state, and whether remote state is unchanged or
still needs an authorized sync.
