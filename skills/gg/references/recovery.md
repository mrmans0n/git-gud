# Recovery

## Preconditions

- Inspect `git status --short`, structured stack state, and the interrupted
  operation before recovery.
- For a failed sync, classify an initial mapped-review preflight failure separately
  from a failure after remote work may have started. Preflight lookup, missing or
  null review, and unexpected source, target, or state errors stop before the
  per-entry sync loop mutates. A later fresh-authorization or provider failure can
  follow completed pushes, review creation, retargeting, or another remote effect.
  Do not infer the class from the nonzero exit status alone.

## Procedure

1. For a failed sync, run `gg undo --list --json` and enumerate every matching
   operation record. One invocation can create multiple segments. Inspect each
   record's `status`, `touched_remote`, `remote_effects`, and arguments; never
   select by timestamp or assume one record per invocation.
2. Reconcile publication state before any retry. Read the saved mapping from
   `gg/config.json` under `git rev-parse --git-common-dir`, inspect that review
   number's state, source, and target directly on the
   provider, and inspect the remote tips of every branch that may have been pushed
   or deleted. A successfully created replacement review can already be persisted
   in local config even when a later step failed.
3. Treat `touched_remote: true`, or an ambiguous push or deletion response, as a
   boundary against automatic recovery. Do not silently undo or rerun the remote
   mutation. Retry only after provider state, remote refs, local config, and the
   operation records have been reconciled.
4. While a rebase is paused, confirm its exact `operation_id` with `gg ls --json`,
   then run the argument-free `gg continue` or `gg abort`; those commands recover
   the operation named by the saved marker. Pass an operation ID explicitly only
   when targeting a completed record with `gg undo <operation_id>`.
5. Resolve conflicts, stage only resolved files, then run `gg continue`.
   After continuing a conflicted `gg rebase`, inspect for the operation's
   `gg-rebase-autostash`; restore it or explicitly surface it before declaring
   the worktree clean.
6. Use `gg abort` only when aborting is requested or necessary to return safely.
   After aborting a conflicted `gg rebase`, inspect for the operation's
   `gg-rebase-autostash`; restore it or explicitly surface it before declaring
   the worktree clean.
7. Treat undo as moving refs and `HEAD` only; it never restores working-tree or
   remote state.
8. If `refusal.reason == "remote"`, surface the provider-specific revert hint
   and stop. Never execute remote rollback silently.
9. On `interrupted`, `stale`, or `unsupported_schema` refusal, report the exact
   reason and stop.
10. Before ancestry repair, run `gg restack --dry-run --json` when the required
   mutation is not already clear.

## Stop conditions

Stop when conflicts remain unresolved, the recovery target is ambiguous, a
remote effect or `touched_remote` record has not been reconciled, a refusal
requires external action, or the requested safe state cannot be established
without a new destructive decision.

## Verification

After recovery, verify refs, `HEAD`, every relevant operation record, stack order,
dirty state, and whether any `gg-rebase-autostash` remains. For sync recovery,
also verify local mappings, provider review source/target/state, and remote branch
tips.

## Report

Report whether the failure was preflight or partial remote progress, every
operation segment and its `touched_remote` state, the recovery action, affected
refs and reviews, remaining working-tree changes, exact refusal reason or
provider hint, and unresolved next steps.
