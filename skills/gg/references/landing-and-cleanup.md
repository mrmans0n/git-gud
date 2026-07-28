# Landing and cleanup

## Preconditions

- Fetch the current base first with `git fetch origin <base>`, then refresh
  approval, CI, and draft state with `gg ls --refresh --json`. Check
  behind-base state by comparing the stack tip with the fetched remote base:
  `git merge-base --is-ancestor origin/<base> <stack-tip>`. Do not rely on
  `gg ls` behind counts for this gate; they compare local base freshness, not
  whether the stack contains the latest base commits. For
  mergeability, use direct provider inspection:
  `gh pr view <number> --json mergeable,mergeStateStatus,isDraft` for GitHub,
  and `glab mr view <iid> --output json` with `detailed_merge_status` for
  GitLab. Do not rely on `gg ls` for mergeability; do not rely on `gg inbox`
  alone for GitLab conflict detection.
- Define readiness as current approval, or a verified absence of required
  reviews, plus successful CI or a verified absence of required CI checks, and
  no blocking state.
  If the user separately approved a GitHub admin bypass, approval may be absent.
  If the user explicitly requested or immediately confirmed `--wait` without
  `--all`, pending or running CI and pending approval may be polled, but do not
  rely on `gg land --wait` as the final gate because it does not refresh draft,
  behind-base, or mergeability after waiting. Poll externally, then run the full
  refreshed preflight immediately before the non-wait landing; failed or
  canceled CI still blocks. For full-stack
  `--all --wait`, require current approval before starting unless a GitHub admin
  bypass was separately approved, and reject `--all --wait` when the repository
  can dismiss stale approvals after downstream force-pushes. Use separate
  per-entry landing invocations instead: after each merge, externally refresh
  provider approval, CI, draft state, mergeability, and behind-base status for
  the next retargeted entry before running the next non-wait landing.
  If the user explicitly requested GitLab auto-merge or merge-train queueing,
  pending or running CI may be queued; failed or canceled CI still blocks.
  Treat CI `Unknown` as ready only after direct provider inspection confirms
  there are no required checks or pipelines; stale or unavailable CI still
  blocks.
  Draft, behind-base, mergeability, and conflict gates always apply.
- Treat a general request such as "finish this stack" as insufficient landing
  confirmation. Ask immediately before running `gg land`.
- Immediately before landing, inspect the effective provider and `land_admin`
  setting. The CLI has no config-show command: run
  `git rev-parse --git-common-dir`, then read `~/.config/gg/config.json` and
  `<git-common-dir>/gg/config.json` when present. Hardcoded defaults apply
  first, global config applies next, and a repository-local `defaults` object
  replaces global defaults when the local file exists. Resolve provider the
  same way the CLI does: an effective `defaults.provider` wins; otherwise run
  `git remote get-url origin` and detect GitHub or GitLab from that URL. Stop if
  the provider cannot be resolved.
- If the effective provider is GitHub and effective `land_admin` is `true`,
  disclose that the CLI will use GitHub's admin merge even without a `--admin`
  argument. Obtain separate explicit approval for that bypass or stop. Landing
  confirmation alone is not admin approval. For GitLab, `land_admin` is ignored
  by the provider and does not create an admin-bypass approval gate.
- If the effective provider is GitLab, inspect inherited queue modes before
  landing confirmation. Effective `defaults.gitlab.auto_merge_on_land` or an
  enabled merge-train path can make ordinary `gg land` queue a later merge, and
  the CLI has no disable flag for those inherited modes. Obtain separate
  explicit approval for GitLab auto-merge or merge-train queueing, or stop.

## Procedure

For cleanup-only requests, do not run `gg land`. Verify the exact requested
stack is already merged using provider state, saved land evidence, or current
ancestry evidence before deleting anything. Do not use `gg clean` for a
single-stack cleanup request; it iterates all configured stacks. For one stack,
construct a targeted cleanup plan from exact recorded local refs, remote refs,
config entries, and worktree path, or stop if those targets cannot be proven.
Use `gg clean -a --json` only when the user explicitly authorized cleanup of
every merged stack, and only after inspecting every configured stack for orphan
cases where the main branch is missing. If any orphan stack still has unmerged
or unknown entry-branch work, obtain separate approval for that exact stack or
stop before invoking global cleanup.

1. After the state and effective-config refresh, obtain explicit landing
   confirmation immediately before execution. If cleanup may follow, record the
   exact stack name, local and remote branch names, configured worktree path, and
   PR/MR mappings before landing.
2. Run the user-approved `gg land` scope with `--json` and explicitly without
   cleanup. For multi-entry stacks, do not use a non-wait `gg land -a` snapshot
   to merge every entry after downstream PR retargeting and branch rebases.
   In squash-merge stacks with downstream entries, land the current entry with a
   one-entry multi-land command such as
   `gg land --until <current-entry> --no-clean --json` so gg retargets and
   rebases remaining branches after the merge. Alternatively, explicitly rebase
   and sync the remaining branch before the next readiness check. Then re-run
   the full refreshed preflight before the next entry after retargeting:
   provider approval, CI, draft state, mergeability, and behind-base checks must
   be current immediately before each merge. Do not use a single
   `gg land --until <tip> --wait --no-clean --json` invocation as the freshness
   gate for multiple entries; `--wait` does not replace the direct provider and
   ancestry preflights above. When GitLab `defaults.gitlab.auto_merge_on_land`
   is inherited and merge trains are not enabled, queue or land one entry,
   verify it externally, then reinvoke landing for the next entry after
   retargeting.
3. Add `--admin` only when the user explicitly approves the GitHub bypass. If
   the effective provider is GitHub and effective `land_admin` is already
   `true`, run only after the separate admin approval even when the command
   line omits `--admin`.
4. For GitLab auto-merge or merge trains, treat "not reported yet; still
   polling" as non-terminal.
5. Treat `queued` and `already_queued` as queued, not merged.
6. Verify the remote merge result before cleanup.
7. If cleanup was separately authorized for the just-landed stack and the
   verified remote result makes it safe, do not rely on `gg clean` after
   `gg land --no-clean`; land removes mappings before returning. First verify
   fresh merged state for every recorded stack entry. If any downstream entry is
   still open, queued, or otherwise not merged, refuse stack cleanup. When every
   entry is merged, use the saved targets and verified remote result for a
   targeted cleanup plan, or stop if those exact targets were not recorded. Use
   `gg clean -a --json` only when the user explicitly authorized cleanup of
   every merged stack and the orphan-stack preflight found no unapproved
   unmerged or unknown entry-branch work.

## Stop conditions

Stop on missing confirmation, stale CI, stale or missing approval unless a
GitHub admin bypass was separately approved or non-full-stack `--wait` was
explicitly confirmed, failed or canceled CI, draft state, unapproved effective
GitHub admin mode, unapproved inherited GitLab auto-merge or merge-train
queueing, conflict, timeout, repeated provider errors, or any
non-terminal merge-train state. Pending or running CI is allowed only for
explicitly requested `--wait`, GitLab auto-merge, or merge-train queueing.

## Verification

Verify each remote merge result. If cleanup was authorized and exact targets were
recorded before landing, remove only those exact refs and the configured worktree
path after confirming they match the verified landed stack. Re-inspect local
stacks and worktrees to confirm only safely landed state was removed.

## Report

Report landed, queued, already queued, still polling, failed, and cleaned states
exactly as observed. Include blockers and any confirmation still required.
