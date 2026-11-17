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

By default, landing several entries rebases and force-pushes the remaining branches after every merge, which re-triggers CI on each of them. On large stacks, combine merge commits with `--no-rebase` to skip that. Original commit SHAs then reach the base branch, so the remaining PRs/MRs don't need rebasing:

```bash
gg land --all --no-squash --no-rebase
```

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
