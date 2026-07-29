# Syncing and reviews

## Preconditions

- Require explicit publishing or syncing intent before remote mutation.
- Run `git status --short` and inspect the current stack/base. Before deciding
  whether to rebase, run `git fetch origin <base>` and compare the stack tip
  with the fetched remote base using
  `git merge-base --is-ancestor origin/<base> <stack-tip>`. Do not use
  `gg ls` behind counts for this predicate; they show local base freshness, not
  whether the stack contains the latest base commits.
- Before every sync, apply a fresh immutable-target preflight for metadata
  normalization. Identify stack entries that may be normalized before `--until`
  filtering, reject entries already contained in `origin/<base>`, refresh mapped
  PR/MR state, and stop on merged or closed targets unless the user separately
  approved rewriting them.
- Before honoring `--lint` or effective `defaults.sync_auto_lint`, apply the
  same amend-mode safety preflight as `gg lint`: require zero pre-existing
  untracked files or explicit approval to include those exact files, fetch the
  current base, reject targets already in `origin/<base>`, and refresh mapped
  PR/MR state so merged or closed targets are not rewritten.

## Procedure

1. If the fetched remote base is not an ancestor of the stack tip after the
   explicit fetch, run ordinary `gg rebase`. On
   `ImmutableTargets`, stop and ask before `gg rebase --force`.
2. Respect repository lint and draft configuration unless the user specifies
   an override. Complete the metadata-normalization preflight before every
   `gg sync`. If lint will run, also complete the amend-mode safety preflight.
3. Prefer `gg sync --jsonl` for monitored agent execution. Use `gg sync --json`
   when only the final aggregate is needed.
4. Consume the final summary event for publication results only: stack, base,
   pre-sync rebase status, warnings, metadata normalization, and each entry's
   source branch, push result, draft flag, PR or MR number, URL, error, and
   action.
5. Use the exact publication action `"up_to_date"` when no publication change
   is needed.
   Other observed actions include `"created"`, `"updated"`, `"recreated"`,
   `"skipped_closed"`, and `"error"`.
6. The summary does not contain review, CI, approval, behind-base, or target
   branch state. After sync, refresh provider-backed state with
   `gg ls --refresh --json` and direct provider inspection when a gate matters.
   Do not treat `gg inbox --json` as authoritative for CI; require a populated
   successful CI result or direct verification that exposes lookup failures.
   Re-run the stack-tip versus `origin/<base>` merge-base check for behind-base
   decisions. If the exact target branch is decision-critical, inspect it
   through the provider rather than inferring it from the sync summary.
7. Surface branch-prefix warnings and `"recreated"` source-branch remaps.
8. Treat managed PR body blocks and stack-navigation comments as gg-owned; do
   not edit them manually.
9. Keep GitHub and GitLab terminology provider-correct while accepting `pr_*`
   JSON fields for both.

## Stop conditions

Stop on lost publishing authority, unrelated dirty state, failed lint,
conflicts, authentication failure, immutable auto-rebase, or terminal sync
failure.

## Verification

Re-inspect the local stack and the provider-backed review state. Confirm the
final summary belongs to the current operation, then use the refreshed state to
confirm current review, CI, and approval. Confirm behind-base status with a
fresh stack-tip versus `origin/<base>` merge-base check.

## Report

Report created, updated, `up_to_date`, recreated, skipped-closed, or failed PRs
or MRs; URLs; refreshed review and CI state; refreshed behind-base state;
warnings; and every remaining non-terminal gate.
