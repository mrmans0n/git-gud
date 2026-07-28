# Syncing and reviews

## Preconditions

- Require explicit publishing or syncing intent before remote mutation.
- Run `git status --short` and inspect the current structured stack state.

## Procedure

1. If the stack is behind base, run ordinary `gg rebase`. On
   `ImmutableTargets`, stop and ask before `gg rebase --force`.
2. Respect repository lint and draft configuration unless the user specifies
   an override.
3. Prefer `gg sync --jsonl` for monitored agent execution. Use `gg sync --json`
   when only the final aggregate is needed.
4. Consume the final summary event. Verify PR or MR number, URL, action, review,
   CI, and behind-base state.
5. Surface branch-prefix warnings and `"recreated"` source-branch remaps.
6. Treat managed PR body blocks and stack-navigation comments as gg-owned; do
   not edit them manually.
7. Keep GitHub and GitLab terminology provider-correct while accepting `pr_*`
   JSON fields for both.

## Stop conditions

Stop on lost publishing authority, unrelated dirty state, failed lint,
conflicts, authentication failure, immutable auto-rebase, or terminal sync
failure.

## Verification

Re-inspect the local stack and the provider-backed review state. Confirm the
final summary belongs to the current operation and that every intended PR or MR
has the expected source branch and target.

## Report

Report created, updated, unchanged, or recreated PRs or MRs; URLs; review and CI
state; behind-base state; warnings; and every remaining non-terminal gate.
