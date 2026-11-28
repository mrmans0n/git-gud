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
2. Reconcile publication state before any retry. Read the saved mapping and any
   `review_creation_intents` from `gg/config.json` under
   `git rev-parse --git-common-dir`. A creation intent binds one entry to the
   exact source branch, target branch, and origin operation ID that preceded an
   ambiguous create request. Do not delete it or issue another create manually.
   A later `gg sync` loads that origin first, then accepts only one open or draft
   provider review with the exact saved source and target; zero, multiple,
   terminal, mismatched, or failed lookups stop without mutation. Inspect the
   mapped review number's state, source, and target directly on the provider,
   and inspect the remote tips of every branch that may have been pushed or
   deleted. A successfully created replacement review can already be persisted
   in local config even when a later step failed.
   A pending remote deletion is freshly authorized after its durable checkpoint;
   a dependent-review result or lookup failure restores the intent to `pending`
   without attempting the deletion push. Treat a `deleting` intent as uncertain:
   do not infer from the provider lookup alone that no push was attempted.
3. Treat `touched_remote: true`, or an ambiguous push or deletion response, as a
   boundary against automatic recovery. Review creation is the narrow exception:
   when its durable intent and exact origin record are intact, `gg sync`
   authoritatively discovers the exact saved source/target review, reconciles
   one `pr_created` effect on that origin, and installs the mapping without a
   second create. If any prerequisite cannot be read or saved, it retains the
   intent and stops. Do not silently undo or rerun any other remote mutation.
   Retry only after provider state, remote refs, local config, and the operation
   records have been reconciled. For failed `gg clean` or land auto-clean, inspect
   every deletion segment separately. Any durably saved `branch_deleted` effect
   is exact evidence of the deleted branch and prior OID regardless of whether
   its operation record is pending, committed, or interrupted. A touched segment
   without that effect means the request outcome is uncertain. The 100-record
   operation history cap counts every record except exact branch-deletion
   evidence and source operation IDs referenced by live remote-deletion intents.
   Pending records and source operation IDs referenced by live review-creation
   intents remain protected from deletion, but share the cap and can reduce the
   retained ordinary history. An interrupted Drop or Clean source is protected
   outside the cap only while its remote-deletion intent remains. Direct Clean
   deletions also
   retain a `pending_remote_branch_deletions` entry in `deleting` state with the
   exact branch and expected OID. Clean refuses another attempt while that entry
   remains, and a later Drop or resumed Drop cannot replace it with fresh
   `pending` authority. Do not repeat the deletion until `git ls-remote --heads
   origin <branch>`, the config entry, and the saved operation record have been
   reconciled.
   For GitLab pipeline creation, inspect the stack's
   `gitlab_landing_attempt` in `<git-common-dir>/gg/config.json` and its exact
   referenced Land operation. The saved identity covers target project, MR IID,
   source project and branch, source OID, and target branch. `Attempted` means
   the POST may have succeeded: gg will not retry it automatically, and neither
   should the agent. Check the matching MR's pipeline list and immutable identity
   directly in GitLab. If those observations do not prove the outcome, preserve
   the record and stop; there is no automated gg repair command for an uncertain
   pipeline POST. A `Completed` handoff is retained until the matching MR is
   observed merged or closed. In JSONL mode, `pipeline_checkpoint` means the
   known pipeline result and operation finalization are durable; failure to
   write or flush that event blocks queueing and every later action in that
   invocation.
   For a GitLab `squash_disable` attempt, inspect the same saved full MR identity and referenced Land record. `Attempted` with `touched_remote: true` is never retried automatically. gg reconciles it only when a fresh policy and exact-identity read proves `squash=false`; otherwise preserve the evidence and stop.
   `Completed` is recovered from the exact record without a provider lookup. In JSONL mode, `squash_checkpoint` means the completed attempt, exact `pr_squash_updated` effect, and finalization are durable. A broken pipe keeps `Completed` and blocks every later mutation until restart emits the checkpoint.
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
