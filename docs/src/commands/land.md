# `gg land`

Merge approved PRs/MRs from the bottom of your stack upward.

```bash
gg land [OPTIONS]
```

## Options

- `-a, --all`: Land all approved entries in sequence
- `--auto-merge`: *(GitLab only)* Request auto-merge instead of immediate merge
- `--no-squash`: Disable squash merge (squash is default). On GitLab, gg also turns off the MR's own squash setting before merging or queueing it, and refuses up front when the project requires squashing
- `--no-rebase`: Don't rebase and force-push the remaining stack branches after each merge. Avoids re-triggering CI on every remaining PR/MR; intended for `--no-squash`, where original commit SHAs stay reachable from the base branch
- `--rebase-if-needed`: After each merge, rebase the remaining branches only when needed (see [Rebasing remaining branches](#rebasing-remaining-branches)). Conflicts with `--no-rebase`
- `-w, --wait`: Wait for CI and approvals before merging
- `-u, --until <UNTIL>`: Land up to a target entry (position, GG-ID, SHA)
- `-c, --clean`: Clean stack automatically after landing all
- `--no-clean`: Disable auto-clean for this run
- `--admin`: *(GitHub only)* Use admin privileges to bypass branch protection requirements (see [Admin Override](#admin-override) below)
- `--json`: Emit machine-readable JSON output (no human logs)
- `--jsonl`: Emit flushed NDJSON events as landing and polling progress; conflicts with `--json`

For non-wait landing, gg refreshes approval for each entry immediately before
its final queue or merge request. If approval was dismissed after the initial
stack refresh, that entry is refused without starting the landing mutation.
Only the explicit GitHub admin override bypasses this check; GitLab still
requires approval when `--admin` is supplied.

For non-wait GitLab landing with `--no-squash`, gg also refreshes approval
immediately before changing the MR's squash setting. If approval was dismissed,
gg leaves the setting unchanged and creates no landing recovery record.

In GitLab merge-train wait mode, gg refreshes approval after validating that a
pipeline is still missing and immediately before asking GitLab to create it.
If approval was dismissed while waiting, gg sends no pipeline request and
records the prepared operation as a proven no-op.

## Examples

```bash
# Land one approved entry
gg land

# Land complete stack, waiting for readiness
gg land --all --wait

# Land part of stack
gg land --until 2

# GitLab auto-merge queue
gg land --all --auto-merge

# Land with merge commits; rebase remaining branches only when needed
gg land --all --wait --no-squash --rebase-if-needed

# JSON output for automation
gg land --all --json

# Stream landing progress and wait heartbeats
gg land --all --wait --jsonl

# Bypass approval requirements (GitHub admin)
gg land --admin

# Land full stack with admin override
gg land --all --wait --admin
```

## Admin Override

The `--admin` flag (or `land_admin` config default) passes `--admin` to `gh pr merge`, which uses GitHub's API-level admin merge. This bypasses **all** branch protection rules the merging user has permission to override, which may include both review approvals **and** required status checks depending on your repository settings.

Use `--wait --admin` if you want to wait for CI to pass before merging while still bypassing approval requirements. Without `--wait`, no client-side CI validation is performed.

On GitLab, `--admin` is a no-op — `glab mr merge` has no equivalent flag. A warning is printed and the merge proceeds normally.

A warning (`⚠ Merging with admin override`) is printed before each admin-elevated merge.

## Downstream MR Retargeting

After landing an entry, `gg land` automatically retargets the next MR in the stack so it no longer points at the now-merged intermediate branch:

- **Single entry** (`gg land`): retargets the immediate next MR to `stack.base`.
- **All entries** (`gg land --all`): retargets all remaining MRs to `stack.base` as each entry is landed.

This applies to both GitHub PRs and GitLab MRs. No manual retargeting in the provider UI is needed after landing.

## Rebasing remaining branches

When you land several entries (`--all` or `--until`), `gg land` by default rebases every remaining branch onto the updated base and force-pushes it after each merge. That keeps squash merges working, but every force-push re-runs CI on every remaining PR/MR.

With `--rebase-if-needed`, gg checks two things after each merge and skips the rebase when both hold:

1. The landed commit itself is reachable from the freshly fetched `origin/<base>`, so it landed through a merge commit rather than as a squashed or rebased copy.
2. The forge reports that the next PR/MR, already retargeted to the base, can merge as-is. On GitHub that means `mergeable` is not `CONFLICTING` and `mergeStateStatus` is not `DIRTY` or `BEHIND`. On GitLab it means no conflicts and a `detailed_merge_status` other than `conflict` or `need_rebase`.

Forges compute mergeability in the background after a retarget, so gg polls for up to a minute. If the answer is still unknown, or either check fails, gg rebases exactly as it does by default. Only the next entry is checked: the entries above it target its branch, and that branch stays untouched when it isn't rebased.

`--no-rebase` skips the rebase unconditionally. Prefer `--rebase-if-needed` unless you know the base branch keeps original commits.

## Merge Trains (GitLab)

When merge trains are enabled on the target branch, `gg land` adds MRs to the merge train instead of merging directly. It uses GitLab's auto-merge: if the MR pipeline is still pending or running, GitLab marks the MR "set to be added to the merge train" and adds it once the pipeline passes. You don't have to wait for CI before queueing, and the MR still joins the train if `gg` exits. The squash setting (`--no-squash` to disable) is sent with the request.

After a preceding MR merges, GitLab can retarget the next MR without starting a new pipeline. With `--wait`, if GitLab reports that the next MR needs CI and has no head pipeline, `gg` starts the MR pipeline before requesting auto-merge. An existing pipeline is left alone.

Before processing a GitLab stack, `gg land` reads the project's merge-train setting and stops without mutating the remote if that setting is unavailable or malformed. Immediately before every MR squash update, merge, queue, or pipeline request, it re-reads the MR through the GitLab API and requires an explicit open, non-draft state with the expected target branch.

### Pipeline creation recovery (GitLab)

Pipeline creation is a durable operation. Before the POST, gg records the target
project, MR IID, source project and branch, current source commit, and target
branch, then revalidates that immutable identity immediately before the request.
If the POST response is ambiguous, the attempt remains durably `Attempted` and
gg will not submit it again on restart because doing so could create a duplicate
pipeline.

Inspect the MR's pipelines and exact source/target identity directly in GitLab
before resolving an ambiguous attempt. Do not delete the recovery record or
retry the POST merely because the command returned an error. If the provider
state does not establish what happened, preserve the record and stop; gg has no
automated repair command that can turn an uncertain POST into a known result.

Once GitLab returns a pipeline ID, gg durably records a `Completed` handoff. It
retains that handoff across restarts and stale provider reads until the matching
MR is observed merged or closed, then clears it transactionally before
continuing to another entry.

For `--auto-merge` without merge trains, an ambiguous queue response also stops automatic recovery. A restart reports the saved Land operation ID without issuing another queue request; reconcile the MR in GitLab before changing the saved attempt.

GitLab only applies a squash flag that is true when queueing or merging, so an MR created while the project's squash option is "Encourage" (`default_on`) would still be squashed. With `--no-squash`, gg therefore turns off the MR's squash setting in its own durable Land segment. It rechecks project policy and the complete MR identity immediately before one PUT, records `pr_squash_updated`, and emits `squash_checkpoint` before any merge or queue request. An initially disabled setting is an unjournaled no-op. If the project's squash option is "Require" (`always`), `gg land --no-squash` fails before touching any MR.

If the PUT response is ambiguous, gg never submits it again automatically. On restart it may reconcile only when a fresh, exact-identity read proves `squash=false`; otherwise it preserves the attempt and stops for manual reconciliation. A failed `squash_checkpoint` write also stops downstream work; the completed record is emitted and cleared on restart.

**Approval is always required** before an MR can enter the merge train queue — even with `--all`. If using `--wait`, the command will show "Waiting for approval..." until a reviewer approves the MR. A pipeline that has already failed or been canceled stops the command instead of queueing.

After queueing, GitLab can take time to report the MR in the merge train listing. With `--wait`, `gg` keeps polling until the configured `land_wait_timeout_minutes` instead of failing after a short not-found window. It still stops promptly if the MR is closed, GitLab reports it was skipped from the train, the MR pipeline fails before the MR joins the train, or repeated API errors occur.

If GitLab accepts the initial queue request but the response is lost, `gg` saves the unresolved request and refuses to send it again. Later `gg land` runs stop before contacting GitLab and report the MR and operation ID that need inspection. Check the MR's merge-train state and the saved operation before changing recovery data or rerunning the command. The completed pipeline handoff remains available while this queue request is unresolved.

GitLab cancels auto-merge when the MR's source branch is pushed, and it does not add the MR back by itself. gg records the MR's head commit when it queues the MR. When `--wait` sees the MR still open, off the train, and no longer set to auto-merge, it compares heads. If the head moved, a push cancelled the merge: gg queues the MR again with the same squash choice, records the new head, reports this in a `wait` event with `status: "warning"`, and keeps polling. If the head did not move, the train pipeline failed or someone cancelled auto-merge by hand, so gg stops waiting with an error instead of queueing the MR again.

## CI Failure Details

When using `--wait`, if CI fails on an MR the command stops and shows which jobs failed:

```
OK Landed 1 MR(s)

⚠ Landed 1 MR(s), but encountered an error:

Error: MR !7621 CI failed
  Failed jobs: lint (stage: test), build-android (stage: build)
```

This helps diagnose CI issues without having to open the GitLab UI. Failed job names and stages are fetched from the MR's head pipeline.

## Concurrent commands while waiting

`gg land --wait` releases the repository-wide `gg` lock while it polls CI,
approvals, or merge-train state. You can run commands such as `gg sync` for a
different stack in another worktree during that time. Landing reacquires the
lock before every merge, rebase, push, cleanup, or config update. After
reacquiring it, Land also checks for an unresolved merge-train queue request
created by another process and stops without contacting GitLab.

Do not change the stack being landed while it waits. If its commits, GG-IDs, or
PR/MR mappings change, landing stops before the next merge and asks you to rerun
the command with the latest stack state.

## JSON Output

Example JSON response:

```json
{
  "version": 1,
  "land": {
    "stack": "my-stack",
    "base": "main",
    "landed": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "feat: add parser",
        "gg_id": "c-abc1234",
        "pr_number": 42,
        "action": "merged",
        "error": null
      }
    ],
    "remaining": 0,
    "cleaned": false,
    "warnings": [],
    "error": null
  }
}
```

## Streaming NDJSON (`--jsonl`)

`gg land --jsonl` emits one compact JSON object per line and flushes each line.
Every event has `version`, `command`, `status`, and `event`. Human output is
suppressed. Use this mode to monitor a long-running `--wait` invocation.

Event kinds:

| Event | Fields | Emitted when |
|---|---|---|
| `start` | `stack`, `base`, `total_entries` | A non-empty landing run begins |
| `pipeline_checkpoint` | `pr_number` | A GitLab pipeline result and its operation finalization are durable |
| `squash_checkpoint` | `pr_number`, `squash` | A GitLab squash update, exact effect, and operation finalization are durable |
| `wait` | `position`, `pr_number`, `phase`, `poll`, `elapsed_seconds`, `ci_status`, `approved`, `merge_train_status`, `merge_train_position`, `pipeline_running`, `error` | Each readiness or merge-train provider poll completes |
| `entry` | Same fields as an item in `landed` | An entry reaches an outcome |
| `summary` | Same fields as the `land` object from `--json` | Landing finishes, including partial failures |
| `error` | `message` | A fatal error prevents a summary |

`phase` is `readiness` while checking CI and approval, or `merge_train` after
queueing an MR. Fields that the current phase did not check are `null`.
Transient provider errors appear in a `wait` event with `status: "warning"` and
the command keeps polling. Unchanged states still emit a heartbeat on the
existing 10-second polling cadence.

For a recovered or newly completed pipeline operation, `pipeline_checkpoint`
is emitted only after the durable state is complete. If writing or flushing that
event fails, including a broken output pipe, landing stops before queueing the
MR or taking the next action.

The final event is `summary` when landing reaches its normal result path.
Consumers should inspect its `error` field because partial landing failures keep
the existing `--json` exit behavior. Fatal setup failures emit `error` and exit
nonzero.

```ndjson
{"version":1,"command":"land","status":"ok","event":"start","stack":"my-stack","base":"main","total_entries":1}
{"version":1,"command":"land","status":"ok","event":"wait","position":1,"pr_number":42,"phase":"readiness","poll":1,"elapsed_seconds":0,"ci_status":"running","approved":true,"merge_train_status":null,"merge_train_position":null,"pipeline_running":null,"error":null}
{"version":1,"command":"land","status":"ok","event":"entry","position":1,"sha":"abc1234","title":"feat: add parser","gg_id":"c-abc1234","pr_number":42,"action":"merged","error":null}
{"version":1,"command":"land","status":"ok","event":"summary","stack":"my-stack","base":"main","landed":[{"position":1,"sha":"abc1234","title":"feat: add parser","gg_id":"c-abc1234","pr_number":42,"action":"merged","error":null}],"remaining":0,"cleaned":false,"warnings":[],"error":null}
```
