# Editing stacks

## Preconditions

- Run `git status --short` and inspect structured stack order before selecting
  a target.
- Confirm the requested edit authorizes local history mutation.
- Before every rewrite, surface `ImmutableTargets`. Require explicit approval
  before `--force` or `--ignore-immutable`.
- Before any `--discard` workflow, require zero pre-existing untracked files or
  separate explicit confirmation to delete those exact files.
- Before any amend-mode workflow that may stage broadly, including
  `gg run --amend` and `gg lint`, require zero pre-existing untracked files or
  separate explicit confirmation to include those exact files.
- Before `gg run --amend` or amend-mode `gg lint`, perform a fresh immutable
  target preflight. Fetch the current base with `git fetch origin <base>`,
  identify every targeted commit, reject any target already contained in
  `origin/<base>` using `git merge-base --is-ancestor <target> origin/<base>`,
  and refresh mapped PR/MR provider state. Reject merged or closed mapped
  targets, and stop on missing or stale mappings when publication state cannot
  otherwise be proven safe. Do not rely on cached stack state alone.
- Before every `gg restack` invocation, perform the same fresh immutable-target
  preflight for every entry restack will replay: fetch the base, reject
  base-ancestor, merged, or closed mapped targets, and require separate
  immutable-target approval for any unsafe target.

## Procedure

- When a client has already prepared the index, run `gg sc --staged-only`.
- For staged fixes spanning multiple commits, prefer `gg absorb -s`.
- Keep ordinary terminal `gg split` interactive. For native Describe/Apply
  split, use [native clients](native-clients.md).
- To insert mid-stack, run `gg mv <target>`, create or amend the commit, then
  run the required `gg restack` preflight. Only then run `gg restack`.
  Confirm `unintegrated_commits` is empty afterward.
- For approved non-interactive drop, run
  `gg drop <targets> --yes --json`; add `--force` only after separate explicit
  immutability-bypass approval. For non-interactive reorder and unstack, supply
  explicit targets and order.
- For an ordinary local rebase request, fetch the requested base when it is
  remote-backed, run `gg rebase [target]`, and stop on `ImmutableTargets` unless
  the user separately approved the override. If conflicts occur, switch to
  [recovery](recovery.md) for `gg continue` / `gg abort`. After success,
  re-inspect stack order, `HEAD`, working tree, and whether a sync remains
  requested.
- For reconcile, run `gg reconcile --dry-run` first and surface planned GG-ID
  additions, PR/MR mappings, and metadata normalization. Before the metadata
  rewrite, fetch the base, refresh mapped provider state, reject base-ancestor,
  merged, or closed targets, and require separate immutable-target approval for
  any unsafe target. After the user confirms the plan, run `gg reconcile`
  interactively or `gg reconcile --yes` for an approved non-interactive
  mutation. Reconcile has no JSON output; verify with structured inspection
  afterward.
- Treat `gg lint` as mutating: it runs configured lint commands in amend mode
  and can rewrite commits. Use it only with lint-amend authority after checking
  targeted commits for immutability and the untracked-file precondition; run
  `gg lint --json`, then verify the stack.
- Use `gg unstack --keep-current --json` only for native clients that must
  retain the lower stack in the current worktree. Otherwise prefer `--wt`.
  When the user intends to continue on the newly created upper stack, use the
  worktree path printed by `gg unstack --wt`, change into that worktree, and
  re-inspect repository, stack, and `HEAD` there before making further edits.
- Run `gg run -- <command>` read-only by default. Use `--amend` only when each
  command's changes, plus any explicitly approved pre-existing untracked files,
  should be folded into its commit. Use `--discard` only when each command's
  changes should be discarded and the untracked-file precondition is satisfied.

## Stop conditions

Stop on unrelated dirty state, pre-existing untracked files before discard,
ambiguous targets, immutable targets without approval, or unresolved conflicts.

## Verification

After rewrites, re-inspect stack order, `HEAD`, and the working tree. Confirm
whether publishing remains requested.

## Report

Report the targeted commits, local rewrites performed, resulting order and
position, dirty or conflict state, and whether remote state is unchanged or
still needs an authorized sync.
