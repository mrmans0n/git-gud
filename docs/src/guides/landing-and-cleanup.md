# Landing and Cleanup

Use `gg land` to merge entries in order, from the bottom of the stack upward.

## Land one approved entry

```bash
gg land
```

## Land the whole stack

```bash
gg land --all
```

Approval is checked again for each entry immediately before its final queue or
merge request. This prevents `--all` from landing an entry whose approval was
dismissed while earlier entries were being processed. GitHub's explicit admin
override is the only exception; GitLab always requires approval.

## Wait for CI and approvals

```bash
gg land --all --wait
```

## Land only part of a stack

```bash
gg land --until 2
# or by GG-ID / SHA
```

## Merge strategy and provider-specific behavior

```bash
gg land --no-squash
```

On GitLab, a non-wait `--no-squash` run checks approval again immediately
before turning off the MR's squash setting. If that approval has been
dismissed, the command refuses before changing the MR or creating landing
recovery state.

By default, landing several entries rebases and force-pushes the remaining branches after every merge, which re-triggers CI on each of them. On large stacks, land with merge commits and `--rebase-if-needed`. After each merge, gg checks that the landed commit reached the base branch unchanged and that the forge says the next PR/MR still merges cleanly. When both hold, the remaining branches stay as they are; otherwise gg rebases them as usual:

```bash
gg land --all --wait --no-squash --rebase-if-needed
```

On GitLab projects with merge trains, `--wait` starts a required MR pipeline if retargeting leaves the next MR without one. You don't need to push a new commit or click **Run pipeline**. Approval is refreshed after confirming that the pipeline is still missing and immediately before the creation request. If approval was dismissed while waiting, no pipeline request or later queue mutation is sent.

`gg land` stops before changing the remote if it cannot read a valid merge-train setting. It also revalidates each MR immediately before mutation and requires an explicit open, non-draft state with the expected target branch. Retry only after direct provider inspection confirms those values.

The initial merge-train queue request is recovery-safe. `gg` records the request before sending it. If GitLab accepts the request but the response is lost, later Land runs stop before provider access instead of queueing the MR again. Inspect the reported MR and operation ID in GitLab and the local operation log before changing recovery data. A completed pipeline handoff is kept separately, so investigating the queue request does not discard that evidence.

`--no-rebase` skips the rebase without checking. Use it only when you know the base branch keeps original commits.

GitLab auto-merge queue:

```bash
gg land --auto-merge
```

## Auto-clean after landing

One-off:

```bash
gg land --all --clean
```

Or make it default with `land_auto_clean` in config.

Manual cleanup remains available:

```bash
gg clean
```
