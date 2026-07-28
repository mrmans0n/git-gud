# Recovery

## Preconditions

- Inspect `git status --short`, structured stack state, and the interrupted
  operation before recovery.

## Procedure

1. Resolve conflicts, stage only resolved files, then run `gg continue`.
2. Use `gg abort` only when aborting is requested or necessary to return safely.
3. Before targeted undo, run `gg undo --list --json`.
4. Treat undo as moving refs and `HEAD` only; it never restores working-tree or
   remote state.
5. If `refusal.reason == "remote"`, surface the provider-specific revert hint
   and stop. Never execute remote rollback silently.
6. On `interrupted`, `stale`, or `unsupported_schema` refusal, report the exact
   reason and stop.
7. Before ancestry repair, run `gg restack --dry-run --json` when the required
   mutation is not already clear.

## Stop conditions

Stop when conflicts remain unresolved, the recovery target is ambiguous, a
refusal requires external action, or the requested safe state cannot be
established without a new destructive decision.

## Verification

After recovery, verify refs, `HEAD`, operation record, stack order, and dirty
state.

## Report

Report the interrupted operation, recovery action, affected refs, remaining
working-tree changes, exact refusal reason or provider hint, and unresolved
next steps.
