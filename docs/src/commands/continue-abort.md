# `gg continue` / `gg abort`

Control paused operations (typically rebases with conflicts).

```bash
gg continue
gg abort
```

Use `gg continue` after resolving conflicts and staging files.
Use `gg abort` when you want to stop and roll back the in-progress operation.

When a recorded `gg` operation stops on a rebase conflict, `gg continue`
finalizes that original operation in the undo log after the rebase completes.
That means the completed operation can still be reversed with `gg undo`.
