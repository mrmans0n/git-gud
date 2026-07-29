# Native clients

## Preconditions

- Load this reference only for MCP or native-client integration.
- Keep CLI semantics canonical and map tools to the corresponding CLI behavior.
- Prefer read-only inspection tools before mutation.

## Procedure

1. A native client that invokes the CLI directly must pass a new global
   `--client-operation-id <ID>` on every mutation. Current MCP tool parameters
   do not expose that flag; do not claim client-operation correlation for an
   MCP mutation.
2. For targeted undo, find the exact flag and value pair in
   `gg undo --list --json`, then use that record's opaque `op_...` ID. Never
   infer a record by timestamp or ordering.
3. Use direct CLI `gg sc --staged-only` for a client-prepared index. Do not use
   native `stack_squash` for this case until it exposes a staged-only option.
4. Use split Describe/Apply only for a client-owned hunk picker. Structured
   Apply has no force override.
5. Require the same land, drop, force, and admin authority as the CLI
   workflows.
6. Before any native landing, perform the landing workflow's refreshed readiness
   preflight: current approval or approved bypass, CI success or verified no
   required checks, not draft, not behind fetched base, and provider
   mergeability. Route to [landing and cleanup](landing-and-cleanup.md) when any
   gate cannot be proven through native-safe inspection.

Use this compact correspondence map; consult installed help or source for
parameters rather than extending it into a schema catalog.

| MCP category and tools | CLI-equivalent behavior |
| --- | --- |
| `stack_list`, `stack_log`, `stack_list_all`, `stack_status` | `gg ls --json`, `gg log --json`, native all-stack summary with base fallback limitations, and the current-stack summary from `gg ls --json` |
| `stack_inbox` | `gg inbox --json` |
| `pr_info` | Direct provider inspection for the requested PR/MR number: `gh pr view <number> --json ...` for GitHub or `glab mr view <iid> --output json` for GitLab, including provider-specific mergeability fields. There is no direct `gg pr-info` command. Do not use native `pr_info` for GitLab mergeability; inspect `detailed_merge_status` with direct `glab`. |
| `config_show` | Effective merged config summary. There is no CLI config-show command, and the current tool does not expose `land_admin`; inspect the config files before landing. |
| `stack_checkout`, `stack_sync`, `stack_land`, `stack_clean` | `gg co`, `gg sync --json`, `gg land --json`, and `gg clean --all --json` only when `stack_clean.all` is true |
| `stack_rebase`, `stack_squash`, `stack_absorb`, `stack_reconcile` | `gg rebase`, `gg sc`, `gg absorb`, `gg reconcile` |
| `stack_drop`, `stack_split`, `stack_reorder`, `stack_restack` | `gg drop --yes --json`, file-based `gg split --no-tui`, `gg reorder --no-tui -o`, `gg restack --json` |
| `stack_move`, `stack_navigate`, `stack_lint` | `gg mv`, `gg first`/`last`/`prev`/`next`, `gg lint --json` |
| `stack_undo`, `stack_undo_list` | `gg undo --json [operation_id]`, `gg undo --list --json` |

For provider-backed decisions, require native inspection calls that support
remote refresh to use it. `stack_list` and `stack_log` must use `refresh: true`
before relying on remote fields they expose. For GitLab approval, do not rely on
`stack_list`; it does not perform the separate approval check. Use refreshed
`stack_log` approval data or direct GitLab approval inspection instead. These
tools do not expose draft, requested-changes, or full review-decision state;
route draft and review-state decisions through `pr_info` when it exposes the
needed provider field, or direct provider inspection. `stack_status` and
`stack_list_all` do not accept a refresh parameter in the current schema; do
not use their cached fields for provider-state decisions. For behind-base
decisions, fetch the base and compare the stack tip with `origin/<base>`
directly instead of relying on native cached summaries.
When effective `defaults.base` is not `main`, do not use native
`stack_list_all` for base or commit-count decisions; route through CLI
`gg ls --all --json` until the native base fallback matches the CLI.

After native `stack_checkout` with `worktree: true`, do not continue mutating
through the same server context. Reconnect or restart the native client with the
printed worktree as `GG_REPO_PATH`, or route through a client that can change
repository context, then re-inspect stack and `HEAD` before further mutations.
Native `stack_checkout` must include an explicit `name`; omitting it can launch
an interactive stack picker that the native caller cannot answer.

Before native `stack_navigate` with `next` or `last`, or native `stack_move`
toward descendants from a detached stack entry, perform the same changed-`HEAD`
check as terminal navigation: compare `git rev-parse HEAD` with the recorded
current entry SHA from refreshed stack data. If they differ, require explicit
local-history mutation authority and the fresh immutable-target preflight for
every downstream entry that can be replayed, or stop.

`stack_drop` always supplies `--yes`; its separate `force` parameter controls
the immutability override. Current `stack_clean` is usable only with `all: true`;
for current-stack-only cleanup, construct an exact targeted cleanup plan or stop
instead of falling back to interactive `gg clean`. Before native
`stack_clean all:true`, require explicit cleanup authorization for every merged
stack and inspect every configured stack for orphan cases where the main branch
is missing. If any orphan stack has unmerged or unknown entry-branch work, obtain
separate approval for that exact stack or stop before invoking native global
cleanup. MCP `stack_split` is file-based, not the structured hunk Describe/Apply
protocol. It is safe for noninteractive use only with an explicit `message` and
`no_edit: true`; otherwise route through a genuinely noninteractive structured
split path. Current `stack_squash` invokes bare `gg sc` and can inherit broad
staging from `defaults.unstaged_action`; route native squashes through direct
CLI `gg sc --staged-only` unless the user explicitly approved staging exact
broader changes after status and config inspection.
Current `stack_lint` invokes `gg lint --json`, which can amend commits; require
lint-amend authority, the zero-untracked or exact-file approval gate, and
immutable-target checks before using it. Before every native `stack_sync`,
fetch the base and compare the stack tip with `origin/<base>`; run
`stack_rebase` first or stop when the stack is behind. Also perform the sync
metadata-normalization immutable-target preflight; if lint is requested or
inherited from `defaults.sync_auto_lint`, apply the lint preflight too. For new
PRs/MRs, pass `draft: true` unless the user explicitly requested non-draft
publication; this protects native sync callers even when effective
`defaults.sync_draft` is false. Do not claim this parameter changes existing
reviews. Current `stack_reconcile` can rewrite metadata
without an immutability guard. Keep it noninteractive: use `dry_run: true` for
inspection, then only after explicit mutation approval use `yes: true` for
execution. Before approving `yes: true`, fetch the base, refresh mapped provider
state, reject base-ancestor, merged, or closed targets, and require separate
immutable-target approval for any unsafe target.
Current `stack_restack` can replay entries without an immutability guard; before
using it, fetch the base, refresh mapped provider state for every entry it will
replay, reject base-ancestor, merged, or closed targets, and stop unless unsafe
targets have separate immutable-target approval.
Current `stack_land` parameters do not expose CLI `--admin`, `--wait`, or GitLab
`--auto-merge`. Its optional `squash` and `auto_clean` parameters currently
expand to flag names the installed CLI does not accept. `squash: false` is also
unsupported because omitting a strategy lets the CLI default to squash; route
non-squash landings through direct `gg land --no-squash --json`. `all: true` is
unsupported because native `stack_land` has no wait/revalidation option; route
full-stack landings through the direct per-entry workflow in
[landing and cleanup](landing-and-cleanup.md). Multi-entry `until` targets are
unsupported for the same reason; route them through the same direct per-entry
workflow with refreshed preflights before every merge. For GitLab native
landing, inspect direct config/provider state for inherited
`defaults.gitlab.auto_merge_on_land` and enabled merge trains; obtain separate
queueing approval or route to [landing and cleanup](landing-and-cleanup.md).
If cleanup has not been separately approved, inspect `defaults.land_auto_clean`;
when it is true, use the direct CLI landing workflow with
`gg land --no-clean --json` so inherited auto-clean cannot remove the stack or
worktree before remote verification.

Keep only decision-critical protocol fields here. Use the
[hosted MCP reference](https://mrmans0n.github.io/git-gud/mcp-server.html) and
source for complete schemas.

## Stop conditions

For CLI-invoking clients, stop on a missing client operation ID. For every
native client, stop on an ambiguous operation record or target, missing mutation
authority, unsupported installed CLI behavior, or a failed structured Apply.

## Verification

Use the corresponding read-only CLI inspection to verify stack order, `HEAD`,
working-tree state, operation correlation when a CLI client operation ID exists,
and any affected remote state.

## Report

Report the native operation, client operation ID when one was available, opaque
undo record ID when created, CLI-equivalent behavior, affected state, and
remaining authority or compatibility blockers.
