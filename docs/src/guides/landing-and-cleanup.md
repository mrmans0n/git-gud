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

By default, landing several entries rebases and force-pushes the remaining branches after every merge, which re-triggers CI on each of them. On large stacks, land with merge commits and `--rebase-if-needed`. After each merge, gg checks that the landed commit reached the base branch unchanged and that the forge says the next PR/MR still merges cleanly. When both hold, the remaining branches stay as they are; otherwise gg rebases them as usual:

```bash
gg land --all --wait --no-squash --rebase-if-needed
```

On GitLab projects with merge trains, `--wait` starts a required MR pipeline if retargeting leaves the next MR without one. You don't need to push a new commit or click **Run pipeline**.

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
