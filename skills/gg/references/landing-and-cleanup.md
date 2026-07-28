# Landing and cleanup

## Preconditions

- Refresh current approval, CI, draft, mergeability, and behind-base state.
- Define readiness as current approval plus successful CI and no blocking
  state.
- Treat a general request such as "finish this stack" as insufficient landing
  confirmation. Ask immediately before running `gg land`.

## Procedure

1. After explicit confirmation, run `gg land -a -c --json`.
2. Use `--admin` only when the user explicitly approves the GitHub bypass.
3. For GitLab auto-merge or merge trains, treat "not reported yet; still
   polling" as non-terminal.
4. Treat `queued` and `already_queued` as queued, not merged.
5. Verify the remote merge result before `gg clean -a --json`.
6. Clean only when landing or cleanup was requested and the remote result makes
   it safe.

## Stop conditions

Stop on missing confirmation, stale approval or CI, failed CI, draft state,
conflict, timeout, repeated provider errors, or any non-terminal merge-train
state.

## Verification

Verify each remote merge result. If cleanup was authorized and performed,
re-inspect local stacks and worktrees to confirm only safely landed state was
removed.

## Report

Report landed, queued, already queued, still polling, failed, and cleaned states
exactly as observed. Include blockers and any confirmation still required.
