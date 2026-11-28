# `gg sync`

Push entry branches and create/update PRs/MRs for the current stack.

```bash
gg sync [OPTIONS]
```

## Options

- `-d, --draft`: Create new PRs/MRs as draft (does not affect existing PRs/MRs)
- `-f, --force`: Force push even if remote is ahead
- `--update-descriptions`: Update PR/MR descriptions from commit messages
- `--update-title`: Update PR/MR titles from commit subjects. If the subject has no draft prefix, a GitLab draft MR keeps its existing prefix, such as `[Draft]`.
- `-l, --lint`: Run lint before sync (aborts sync on lint failure and restores repository state to the pre-sync snapshot)
- `--no-lint`: Disable lint before sync (overrides config default)
- `--no-rebase-check`: Skip checking whether your stack base is behind `origin/<base>`
- `--rebase-if-needed`: After a parent lands, keep published branches at their existing SHAs when the next PR/MR can merge into the fetched base. Conflicts with `--no-rebase-check`.
- `--no-verify`: Skip the pre-push hook for pushes performed by this sync (forwards `git push --no-verify`). Opt-in per invocation; does not affect other hooks.
- `-u, --until <UNTIL>`: Sync up to target commit (position, GG-ID, or SHA)
  Lint fixes and order-preserving rebases keep the selected entry in scope even before it has a GG-ID. If a rebase drops entries and sync cannot identify the target, it stops rather than publishing a different commit.
- `--json`: Output structured JSON for automation (suppresses human/progress output)
- `--jsonl`: Output streaming NDJSON for automation (one JSON event per line, flushed after each; see Streaming Events below)

Before pushing, `gg sync` checks whether your stack base is behind `origin/<base>`. If it is behind by at least the configured threshold, git-gud warns and suggests rebasing first (`gg rebase`).

When you run `gg sync --lint`, lint runs before the first push or PR/MR update. A lint failure aborts sync and restores the checkout to its pre-lint state; it cannot undo a target change from a preceding successful lint pass.

## Failure boundaries and safe retries

Before the per-entry sync loop mutates anything, `gg sync` reads every mapped
PR/MR in scope. A failed lookup, missing or null review, or an unexpected source,
target, or state aborts before that loop starts mutating. This is a fail-closed
preflight, not a transaction for the whole command.

Immediately before each remote mutation involving an existing mapped review,
sync reads that review's provider state again and authorizes that specific
change. Unmapped branch pushes and initial review creation have no existing
review to reread, but an ambiguous response leaves the same uncertainty. A race
or lookup failure can stop the command after an earlier operation segment,
branch push, or PR/MR creation or update succeeded. There is no automatic
rollback. When sync creates a replacement for a review whose source branch
changed, it persists the new review mapping immediately after successful
creation, before attempting later updates or closing the old review.

Before any new PR/MR create request, sync durably records the exact source,
target, and owning operation ID in a repository-local creation intent. If the
provider response is ambiguous, sync retains that intent, commits the owning
operation as remote-touched, emits an error checkpoint, and stops before any
later create, prune, stack-link, replacement follow-up, or navigation mutation.
The next sync performs provider/authentication preflight, loads the exact origin
operation, and searches the provider by the saved source branch. Recovery
continues only when one open or draft review has the exact saved source and
target. It records that review on the origin operation and installs the mapping
without issuing another create. Zero, multiple, closed, merged, or mismatched
results fail closed and leave the intent for manual inspection.

A recovered intent deliberately uses its saved topology, even when `gg unstack`
has since moved the entry. After recovery, normal sync topology handling can
create a replacement review for the moved source branch. Successful `gg land`
removes only the landed entry's intent; unrelated unresolved intents remain.
Creation intents and mappings live in `gg/config.json` under the directory
reported by `git rev-parse --git-common-dir`.

Before retrying a failed sync, reconcile what actually happened:

- inspect the mapped review number, state, source, and target on the provider;
- compare the relevant remote branch tips with the intended local commits;
- inspect `gg/config.json` under the directory reported by
  `git rev-parse --git-common-dir` for the current saved mapping;
- enumerate all matching operation records with `gg undo --list --json` and
  check `status`, `touched_remote`, and `remote_effects`; and
- if a rebase is paused, use `gg ls --json` to identify the owning operation
  before choosing `gg continue` or `gg abort`.

Do not blindly retry an uncertain push or remote deletion: an ambiguous response
may hide a successful mutation. Retry only after provider state, remote refs,
local config, and operation records agree on the current state.

### Sync after a parent lands

Use `gg sync --rebase-if-needed` to publish the rest of a stack without restarting CI just because its bottom PR/MR merged. Sync fetches `origin/<base>`, excludes bottom entries reachable by their original SHA (even if their PR/MR mappings are gone), and retargets the next review to the base. It checks the forge's mergeability **on that target and source SHA**. If the review can merge as-is, sync leaves the surviving branches alone. It neither merges nor queues a review.

If a mapped parent was squashed, sync checks that the forge's merge result reached the base and that the original source matches the local entry. It rebases just the survivors in that case, or if the new target requires a rebase. A new entry without a published review has no mergeability verdict, so sync rebases it before publishing. If the review, source, or merge result cannot be verified, sync stops rather than guessing. A target edit can already have succeeded when this happens. Inspect the review and retry.

A sync that edits that target records and finalizes the remote retarget as one
operation segment before it starts any later replay, lint, or publication work.
Those later mutations use a successor segment, so one `gg sync` invocation can
produce more than one operation ID. This keeps a completed remote mutation from
being hidden inside a later local failure. For recovery, inspect every matching
record with `gg undo --list --json` instead of assuming one invocation has one
record or selecting by newest timestamp. A remote-touched segment refuses local
undo; a later local-only successor may be undoable when its recorded refs still
match. If a rebase is paused, confirm the exact `operation_id` reported by
`gg ls --json`, then run the argument-free `gg continue` or `gg abort`; those
commands recover the operation named by the saved marker. Pass an operation ID
explicitly only when targeting a completed record with `gg undo <operation_id>`.

`--until` still names the entry from before the fetch, not a different entry that inherited its position. Lint skips landed entries. If the retarget requires a rebase, `--lint` checks the rebased commits again before pushing. Lint that changes files can still amend commits. Later `gg ls` commands use the confirmed base for this stack without moving your local base branch or checking it out in another worktree. When no parent has landed, the ordinary behind-base threshold and auto-rebase setting still apply.

Before pushing, `gg sync` normalizes `GG-ID` and `GG-Parent` metadata for the stack. It keeps a surviving entry's existing `GG-Parent` when it identifies the actual Git parent on the confirmed base. `--rebase-if-needed` stops if unrelated trailer repairs would rewrite an otherwise unchanged published branch.

After syncing the selected entries, `gg sync` consumes durable deletion intent
created by a successful local `gg drop`. Each intent is bound to the exact
remote-tracking OID known at Drop time; a fresh server tip is never adopted as
deletion authority. If the remote branch changed, sync fails with the expected
and current OIDs and leaves it intact. If no trusted remote version existed at
Drop time, no intent is created and the branch is retained.

Intent is one-shot and independent of the 100-record undo journal. Confirmed
deletion or observed remote absence consumes it, while `gg undo` of the Drop or
recreation of the entry cancels it. Partial pruning persists confirmed progress per
branch. Any unsuccessful deletion attempt remains uncertain and cannot be
retried automatically, even when stderr reports a rejection. This also applies to
`--until`: the complete local stack protects every retained or recreated entry.
Before deleting, sync asks GitHub/GitLab whether any open PR/MR still targets
the branch (for example, a later entry outside the `--until` range, or one whose
base update failed). After persisting the deletion checkpoint, it immediately
repeats that provider check before the exact-OID deletion push; the earlier
classification is not deletion authority. If either check finds a dependent
review, or either check fails, the branch is kept, the intent remains safely
pending, and sync prints a warning (also in the JSON `warnings`).
Retarget the review, usually with a full `gg sync`, and sync again. `gg clean`
applies the same check and keeps the stack when it defers a deletion.
Remote deletions make the sync operation ineligible for local `gg undo`.

If a deletion attempt fails, or sync reports an uncertain prior deletion outcome, inspect the named remote
branch and the repository-local `pending_remote_branch_deletions` config entry
before manually removing or resetting it. This conservative stop prevents an
ambiguous prior success from deleting a later same-OID republication.

If the current stack branch has a valid stack shape but uses a different prefix
than `defaults.branch_username`, `gg sync` continues and warns that stack
discovery, listing, and saved PR/MR mappings may be inaccurate. In `--json`
mode, this message is included in `sync.warnings`.

If an existing mapped PR/MR is attached to the wrong source branch (for
example after moving commits into a new stack with `gg unstack`), providers
cannot retarget that source branch in place. `gg sync` creates a replacement
PR/MR with the correct branch, updates the local mapping, comments on the old
PR/MR, and closes it. In JSON output that entry uses action `"recreated"`.

You can control this behavior with config:

- `defaults.sync_auto_rebase` (`sync.auto_rebase`): automatically run `gg rebase` before sync when behind threshold is reached
- `defaults.sync_behind_threshold` (`sync.behind_threshold`): minimum number of commits behind before warning/rebase logic applies (`0` disables the check)

## Examples

```bash
# First publish as drafts
gg sync --draft

# Sync only first two entries
gg sync --until 2

# Refresh PR/MR descriptions after commit message edits
gg sync --update-descriptions

# Also update PR/MR titles to match commit subjects
gg sync --update-title

# Run lint as part of sync
gg sync --lint

# Skip behind-base check once
gg sync --no-rebase-check

# Machine-readable output
# (useful in scripts/agents)
gg sync --json

# Skip pre-push hooks for this sync only
gg sync --no-verify
```

## Target Branch Resolution

When computing the target branch for each PR/MR, `gg sync` walks backwards through predecessor entries and skips any that are already merged or closed. If all predecessors have been merged, the target falls back to `stack.base`. This ensures downstream MRs are correctly retargeted after an upstream MR is merged — whether merged via `gg land` or directly in the provider UI.

## GitHub Stacked PRs (public preview)

For a full GitHub `gg sync` (without `--until`), git-gud collects the current
stack's active open and draft PR numbers in bottom-to-top order. When there are
at least two resolved PRs and `defaults.github.stacks_integration` is not
`off`, it can reconcile GitHub's native Stacked PRs map through the optional
official `gh stack` extension. This does not replace or change git-gud's local
stack metadata, and GitLab sync never uses this integration.

Install the extension yourself if you want the native UI:

```sh
gh extension install github/gh-stack
```

git-gud never installs or upgrades it automatically. The minimum supported
extension version is `0.1.0`.

### Eligibility and safe outcomes

The integration is skipped when it is disabled, the sync is partial, fewer than
two active PRs are available, or an active PR cannot be resolved. Full syncs
create GitHub Stacked PRs when none of the active PRs is already associated,
and append only new top PRs when the native active sequence is an exact ordered
prefix of git-gud's sequence. Appending by stack number preserves a merged
prefix; git-gud does not relink merged PRs or retarget a PR onto a merged
branch.

If the native and local sequences diverge, git-gud performs no mutation and
returns a warning. Inspect the GitHub Stacked PRs map, explicitly unstack or
repair it with GitHub's tools, then rerun `gg sync`. Version one never removes,
reorders, unlinks, or recreates GitHub Stacked PRs.

`auto` silently records a skipped result when the extension is missing or
outdated, or the repository does not support the public preview. `force` uses
the same create/append-only safety checks but surfaces those capability states
as warnings. Divergence, malformed responses, authentication or network errors,
and a failed `gh stack link` are warnings in both modes. Every native-integration
failure is non-fatal to an otherwise successful `gg sync`.

### JSON and JSONL output

Atomic `gg sync --json` includes a `sync.github_stack` object for GitHub syncs
(and `null` when the provider is not GitHub or provider detection did not
complete). It always contains `mode`, `action`, `reason`, `stack_number`,
`pr_numbers`, and `message`:

```json
{
  "version": 1,
  "sync": {
    "stack": "feature",
    "base": "main",
    "github_stack": {
      "mode": "auto",
      "action": "created",
      "reason": null,
      "stack_number": 7,
      "pr_numbers": [41, 42],
      "message": null
    }
  }
}
```

`action` is one of `created`, `appended`, `unchanged`, `skipped`, or `warning`.
Stable reasons include `disabled`, `partial_sync`, `insufficient_prs`,
`unresolved_prs`, `missing_extension`, `outdated_extension`,
`unsupported_repository`, `diverged`, and `backend_failed`.

`gg sync --jsonl` emits a line-delimited `github_stack` event immediately after
reconciliation and repeats the same result in the deterministic final `summary`
event. Its `status` is `ok` for created, appended, unchanged, and skipped
results, and `warning` for warning results. For example, a divergence event is:

```ndjson
{"version":1,"command":"sync","status":"warning","event":"github_stack","mode":"auto","action":"warning","reason":"diverged","stack_number":null,"pr_numbers":[41,42],"message":"Local pull requests do not match the native stack active sequence"}
```

## PR/MR Body Ownership

When `gg sync` creates a new PR/MR, the generated description is wrapped in invisible HTML comment markers:

```
<!-- gg:managed:start -->
(generated content from commit message / template)
<!-- gg:managed:end -->
```

On subsequent syncs with `--update-descriptions` (or when `sync_update_descriptions` is enabled in config), only the content inside the managed block is regenerated. Any text you add **above or below** the markers on GitHub/GitLab is preserved across syncs.

This means you can safely:

- Add review checklists above or below the managed block
- Write reviewer notes that survive re-syncs
- Check/uncheck task boxes outside the managed section

Content **inside** the managed block (the generated description) is regenerated on every sync. If your PR template includes a checklist, place persistent checklists outside the markers after creation.

**Legacy PRs** (created before this feature) have no managed markers. `gg sync` will skip body updates for these PRs and log a warning, to avoid overwriting manual edits.

## Stack navigation comments

If `defaults.stack_nav_comments` is enabled in `gg/config.json` under the
directory reported by `git rev-parse --git-common-dir`, every
full `gg sync` (no `--until`) reconciles a managed comment on each PR/MR in
the stack. The comment shows all entries in the stack in bottom-up order,
with a 👉 marker on the entry that PR corresponds to — letting reviewers see
where they are in the chain and click through to siblings.

The comment is identified by a hidden HTML marker (`<!-- gg:stack-nav -->`)
and never touches comments git-gud didn't create. Disabling the setting and
re-syncing cleans up any previously-posted comments automatically.

Merged or closed PRs are left alone — `gg sync` never modifies comments on
historical PRs.

When running with `--json` or `--jsonl`, each entry includes an optional
`nav_comment_action` field (one of `"created"`, `"updated"`, `"unchanged"`,
`"deleted"`, `"error"`) when a reconcile decision was made.

## Streaming NDJSON (`--jsonl`)

`gg sync --jsonl` emits one compact JSON object per line on stdout and flushes
after each line. Human/progress output is suppressed on stdout but diagnostics
may still appear on stderr. This mode is intended for agents and long-running
pipelines that want incremental updates instead of waiting for a final aggregate
payload.

Every line is a valid JSON object with at least:

- `version`: output schema version (`1`)
- `command`: always `"sync"`
- `event`: event kind (see below)

Event kinds:

| Event | Fields | Emitted when |
|---|---|---|
| `start` | `stack`, `base`, `total_entries` | Sync begins |
| `lint_started` | `total_entries`, `commands` | Lint begins (only when lint runs; follows `start`) |
| `lint_entry_started` | `position`, `sha`, `title` | Lint begins on a commit |
| `lint_command_done` | `position`, `command`, `passed`, `duration_ms`, `output` | One lint command finished; `output` (last 8 KB) only on failure |
| `lint_entry_done` | `position`, `sha`, `passed`, `amended` | Lint finished on a commit; `amended: true` means its SHA changed (`sha` is the new one) |
| `lint_done` | `all_passed` | Lint phase finished; on failure sync aborts with an `error` event naming the first failing commit and command |
| `entry_started` | `position`, `sha`, `title`, `gg_id`, `branch` | Processing begins for a stack entry |
| `push_started` | `position`, `branch` | A push is about to start |
| `push_done` | `position`, `branch`, `forced` | Push succeeded |
| `push_error` | `position`, `branch`, `error` | Push failed; sync stops because the remote result may be ambiguous |
| `pr_created` | `position`, `pr_number`, `pr_url`, `draft` | New PR/MR created |
| `pr_create_error` | `position`, `branch`, `error` | PR/MR creation failed; sync stops because the remote result may be ambiguous |
| `pr_comment` | `position`, `pr_number`, `action`, `error` | Replacement notice comment checkpointed (`action` is `created` or `error`) |
| `pr_closed` | `position`, `pr_number`, `error` | Replaced PR/MR close attempted |
| `pr_updated` | `position`, `pr_number`, `action` | Existing PR/MR updated; mutation checkpoints use `title_updated`, `description_updated`, or `retargeted` (`updated`/`recreated` remain aggregate publication actions) |
| `pr_skipped_closed` | `position`, `pr_number` | Existing PR/MR is merged/closed and skipped |
| `nav_comment` | `position`, `pr_number`, `action`, `error` | Managed nav comment reconciled (`created`/`updated`/`unchanged`/`deleted`/`error`/`skip`) |
| `github_stack` | `mode`, `action`, `reason`, `stack_number`, `pr_numbers`, `message` | GitHub Stacked PRs reconciliation completed; `status` is `ok` or `warning` |
| `error` | `message` | Fatal error before completion |
| `summary` | same shape as `--json` `sync` object | Sync finished successfully |

On successful completion, the last event is `summary`, so consumers can detect
completion without reconstructing state. If an unrecoverable error happens
before completion, the final event is `error` and the process exits non-zero.

### Example `--jsonl` output

```ndjson
{"version":1,"command":"sync","event":"start","stack":"my-stack","base":"main","total_entries":2}
{"version":1,"command":"sync","event":"entry_started","position":1,"sha":"abc1234","title":"Add feature","gg_id":"c-abc1234","branch":"user/my-stack--c-abc1234"}
{"version":1,"command":"sync","event":"push_started","position":1,"branch":"user/my-stack--c-abc1234"}
{"version":1,"command":"sync","event":"push_done","position":1,"branch":"user/my-stack--c-abc1234","forced":false}
{"version":1,"command":"sync","event":"pr_created","position":1,"pr_number":42,"pr_url":"https://github.com/org/repo/pull/42","draft":false}
{"version":1,"command":"sync","event":"summary","stack":"my-stack","base":"main","rebased_before_sync":false,"warnings":[],"metadata":{"gg_ids_added":0,"gg_parents_updated":0,"gg_parents_removed":0},"entries":[{"position":1,"sha":"abc1234","title":"Add feature","gg_id":"c-abc1234","branch":"user/my-stack--c-abc1234","action":"created","pr_number":42,"pr_url":"https://github.com/org/repo/pull/42","draft":false,"pushed":true,"error":null}]}
```

### When to use `--json` vs `--jsonl`

- Use `--json` when you want a single, complete aggregate payload at the end and
  do not need progress while the command runs.
- Use `--jsonl` when you want to stream events as they happen, monitor progress,
  or pipe the output to a line-oriented consumer. Every line is self-contained.

Example JSON (shape):

```json
{
  "version": 1,
  "sync": {
    "stack": "my-stack",
    "base": "main",
    "rebased_before_sync": false,
    "metadata": {
      "gg_ids_added": 0,
      "gg_parents_updated": 1,
      "gg_parents_removed": 0
    },
    "entries": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "Add feature",
        "gg_id": "c-abc1234",
        "branch": "user/my-stack--c-abc1234",
        "action": "created",
        "pr_number": 42,
        "pr_url": "https://github.com/org/repo/pull/42",
        "draft": false,
        "pushed": true,
        "error": null
      }
    ]
  }
}
```
