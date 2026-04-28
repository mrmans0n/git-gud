---
name: gg
description: Use git-gud (gg) to manage stacked diffs with GitHub PRs or GitLab MRs. Use this when creating stacks, syncing updates, checking CI/review state, and landing approved work safely.
---

# gg

Use this skill to operate **git-gud (`gg`) as a CLI tool** for day-to-day stacked-diff workflows across GitHub and GitLab.

## When to use

- You need multiple PRs/MRs that depend on each other
- You need to sync stack changes and keep review metadata updated (including GG-ID/GG-Parent normalization)
- You need machine-readable command output for automation (`--json`)

## Prerequisites

- `gg` installed
- Provider CLI installed + authenticated:
  - GitHub: `gh auth status`
  - GitLab: `glab auth status`
- Git repo with `gg` initialized (`gg setup`)

> **Note:** Network errors during auth check are non-fatal — gg prints a warning and continues. The operation may fail later if authentication is actually required.

## Setup

### Interactive wizard (recommended)

```bash
gg setup        # Quick mode: essential settings (provider, base, username)
gg setup --all  # Full mode: all settings organized by category
```

**Quick mode** prompts for: provider, base branch, username.

**Full mode** organizes all settings into groups: General, Sync, Land, Lint, Worktrees, and GitLab (if applicable). Includes sync_draft, sync_update_descriptions, and sync_update_title options.

### Global config

Store shared defaults in `~/.config/gg/config.json` that apply to all repos. Local config (`gg setup`) takes precedence.

### Manual setup (`.git/gg/config.json`)

```json
{
  "defaults": {
    "provider": "github",
    "base": "main",
    "branch_username": "your-github-user",
    "lint": ["cargo fmt --all --check", "cargo clippy -- -D warnings"],
    "auto_add_gg_ids": true,
    "sync_auto_rebase": false,
    "sync_behind_threshold": 1,
    "sync_auto_lint": false,
    "sync_draft": false,
    "sync_update_descriptions": true,
    "sync_update_title": false,
    "land_auto_clean": false,
    "land_wait_timeout_minutes": 30,
    "unstaged_action": "ask"
  }
}
```

```json
{
  "defaults": {
    "provider": "gitlab",
    "base": "main",
    "branch_username": "your-gitlab-user",
    "lint": ["cargo fmt --all --check", "cargo clippy -- -D warnings"],
    "gitlab": {
      "auto_merge_on_land": false
    }
  }
}
```

`auto_add_gg_ids` is deprecated and kept only for compatibility with existing configs; runtime behavior always treats it as enabled.

## Core workflow

1. Create/switch stack (prefer worktree):

```bash
gg co -w feature-auth
```

When splitting an existing stack into lower and upper stacks, prefer a managed
worktree for the new upper stack:

```bash
gg unstack --target 3 --name feature-auth-followup --wt
```

2. Commit logical changes:

```bash
git add <files>
git commit -m "feat: add input validation"
```

3. Check stack state:

```bash
gg ls --json        # single-stack details + summary metrics
gg log --json       # smartlog-style view of the current stack
gg inbox --json     # cross-stack triage buckets for action needed
```

4. Publish/update PR/MR chain:

```bash
gg sync --json
```

If a mapped PR/MR still points at an old source branch after a stack split,
`gg sync` recreates that PR/MR with the current entry branch, remaps config to
the new number, comments on the old one, and closes it. JSON action is
`"recreated"`.

5. When approved + green CI, land **only after user confirmation**:

```bash
gg land -a -c --json
```

## Agent operating rules (mandatory)

1. **Never run `gg land` without explicit user confirmation.**
2. **Always use `--json`** for `gg ls`, `gg sync`, `gg land`, `gg clean -a`, and `gg lint`.
3. **Prefer worktrees** for isolation (`gg co -w <stack>`).
4. Verify `approved: true` and `ci_status` success before landing. If the user requests `--admin`, skip the approval check (GitHub only — GitLab ignores the flag).
5. If sync warns stack is behind base, run `gg rebase` first.
6. Prefer `gg absorb -s` for multi-commit edits.
7. **Never use `git add -A` blindly.** Review `git status` first and only stage intended files. Use `git add <specific-files>` to avoid leaking secrets, env files, or unrelated changes.
8. **Respect the immutability guard.** Rewrite-style commands (`gg sc`, `gg absorb`, `gg reorder`/`gg arrange`, `gg split`, `gg unstack`, `gg drop`, `gg rebase`, `gg restack`) refuse to rewrite merged PRs/MRs or commits already on the base branch, except that `gg rebase` silently skips base-ancestor commits that naturally drop out when rebasing onto the refreshed base. If a command exits with `ImmutableTargets`, surface the listed commits and reasons to the user and get explicit confirmation before retrying with `-f` / `--force` (alias `--ignore-immutable`).

## Common operations

- Navigate: `gg mv`, `gg first`, `gg last`, `gg prev`, `gg next`
- Amend current commit: `gg sc` / `gg sc -a`
- Auto-distribute staged hunks: `gg absorb -s`
- Split a commit into two: `gg split` — opens a two-panel TUI for hunk selection (files on the left, colored diff on the right), followed by inline commit message inputs for both the new and remainder commits. Use `--no-tui` to fall back to sequential `git add -p` style prompts. The `-m` flag bypasses the TUI message input for the new commit. The `--no-edit` flag skips the remainder message input. Pass `FILES...` to auto-select all hunks from those files (e.g., `gg split -c 3 file1.rs file2.rs`).
- Split a stack into two stacks: `gg unstack` — opens a picker by default. The selected entry and descendants become a new independent stack; lower entries remain in the original stack. Use `--target <position|gg-id|sha> --no-tui` for scripts, and `--name <stack>` to choose the new stack name.
- Drop commits from stack: `gg drop <position|sha|gg-id>... -y` (alias: `gg abandon`). Use `-y` / `--yes` to skip confirmation; add `-f` / `--force` only to bypass the immutability guard for merged/base-ancestor commits.
- Reorder/drop stack (TUI): `gg reorder` (or `gg arrange`) — opens interactive TUI for visual reordering and dropping commits. Press `d` to mark a commit for dropping. Use `--no-tui` to fall back to text editor (delete lines to drop).
- Reorder stack (direct): `gg reorder -o "3,1,2"`
- Sync subset: `gg sync -u <position|gg-id|sha> --json`
- Lint stack: `gg lint --json`
- Run a command across the stack: `gg run -- <cmd...>` (see below)
- Triage multiple stacks at once: `gg inbox --json`
- Repair ancestry drift: `gg restack` / `gg restack --dry-run --json` (see below)
- Clean merged stacks: `gg clean -a --json`
- Undo last local mutation: `gg undo` (see below)

## Undoing local mutations (`gg undo`)

`gg undo` reverses the ref/`HEAD` effects of the most recent mutating
`gg` command by replaying a snapshot from the per-repo operation log at
`<commondir>/gg/operations/*.json`. It never touches the working tree,
index, or untracked files — only refs move.

```bash
gg undo              # reverse the most recent local operation
gg undo --list       # see recent operations (newest-first)
gg undo <op_id>      # target a specific record from --list
gg undo; gg undo     # redo: a second undo reverses the first
gg undo --json       # machine-readable output
```

Every mutating command (`sc`, `drop`, `split`, `unstack`, `rebase`, `reorder`,
`absorb`, `reconcile`, `restack`, `checkout`, nav, `clean`, `sync`, `land`,
and `run --amend`) snapshots the refs it will touch before mutating and
records the operation on success. The log keeps the last 100 records;
interrupted/pending records are never pruned.

**Refusal modes** (exit 1, no refs touched, JSON includes `refusal.reason`):

- `remote` — target operation pushed/merged/closed/created a PR/MR.
  gg prints a provider-specific revert hint (`gh pr close <n>`,
  `glab mr close <n>`, `git push --delete …`). Agents must surface the
  hint to the user rather than attempt silent remote rollback.
- `interrupted` — operation crashed or was Ctrl-C'd mid-flight.
- `stale` — refs moved since the target operation finalised. The error
  names the ref, expected OID, and actual OID.
- `unsupported_schema` — record was written by a newer `gg` binary.

`gg undo` does **not** restore working-tree content (use `git reflog` or
`git stash`), does **not** touch remotes, and does **not** support an
`--all` / `--range` mode.

## Repairing stack ancestry (`gg restack`)

`gg restack` detects and repairs ancestry drift — when a commit's `GG-Parent`
trailer no longer matches its expected parent in the stack order. This happens
after manual `git rebase`, `git commit --amend`, cherry-picks, or upstream
rebases that rewrite commit SHAs without updating GG metadata.

```bash
gg restack --dry-run        # show plan without changes
gg restack --dry-run --json # machine-readable plan
gg restack                  # execute full ancestry repair
gg restack --from 3         # repair only from position 3 upward
gg restack --json           # execute with JSON output
```

Each step in the plan is one of:
- **ok** — parent already correct, no action needed
- **reattach** — parent differs, needs rebasing
- **skip** — below `--from` threshold, not checked

After a successful restack, run `gg sync` to push the repaired commits.

## Running commands across the stack (`gg run`)

`gg run` walks every commit in the current stack (oldest → newest) and
executes a command at each one. Use it for things that don't fit the
`lint` config — ad-hoc verifications, formatters, single-shot scripts,
etc.

- **Read-only (default)**: `gg run -- cargo test -p mycrate`
  Each commit is checked out, the command runs, and the tree must stay
  clean. Any modification fails that commit (same contract as `gg lint`
  without `--amend`).
- **Amend mode**: `gg run --amend -- cargo fmt`
  Changes the command makes are folded into each commit via
  `git commit --amend`, then the rest of the stack is rebased on top.
  This is the same engine `gg lint` uses.
- **Discard mode**: `gg run --discard -- ./mutating-check.sh`
  Runs the command and throws away any changes (working tree + index +
  untracked). Useful when you only care about the exit code of a
  command that happens to mutate state.
- **Stop on first failure (default)** vs **keep going**:
  Add `--keep-going` / `-k` to continue through failing commits instead
  of aborting at the first one.
- **Limit how far you go**: `--until <position|gg-id|sha>` stops after
  the named commit. Everything above it is left untouched.
- **Parallelize read-only runs**: `-j N` / `--jobs N` spawns isolated
  worktrees per commit. Valid only with the default (read-only) mode.
  The dirty-tree check in each worker matches the sequential path
  (untracked files are ignored, tracked modifications fail).
- **JSON output**: `gg run --json -- <cmd...>` emits a single
  `RunResponse` document on stdout (see `reference.md`). Failures set
  `run.all_passed = false`; the process still exits non-zero, but only
  the run payload is printed — no trailing `{"error":...}` object.

Argument boundaries are preserved: `gg run -- git commit -m "multi word"`
passes exactly those argv elements to `git` without shell splitting, so
quoted args with spaces, globs, or shell metacharacters go through
intact. Use `--` before the command (as shown) so clap treats
subsequent tokens as the command, not as `gg run` flags.

For repeatable linter runs with commands configured in `.git/gg/config.json`,
prefer `gg lint` — it's `gg run --amend` with the command list coming from
config.

## Immutable commits

gg refuses by default to rewrite commits that look "already published":

- the tracked PR/MR is merged, or
- the commit is already reachable from `origin/<base>` (or the local base, as
  a fallback).

The guard protects `gg sc`, `gg absorb`, `gg reorder` / `gg arrange`,
`gg split`, `gg unstack`, `gg drop`, `gg rebase`, and `gg restack`. `gg rebase` is a special case: merged commits (both base-ancestor commits and squash-merged PRs) are silently skipped because `git rebase` would drop them naturally via patch-id matching instead of rewriting them. For the remaining commands the command exits with an `ImmutableTargets` error listing every affected position, short SHA, title, and reason (e.g. `merged as !123`, `already in origin/main`).

To bypass it intentionally, pass `-f` / `--force` (long alias
`--ignore-immutable`). Always surface the listed commits and reasons to the
user first; the override still emits a warning and proceeds.

`gg land`'s post-merge cleanup bypasses the guard by design, and
`gg absorb --dry-run` skips it (no rewrite happens). See
`reference.md` → "Immutable commits" for details.

## PR/MR body ownership

`gg sync` uses managed markers to separate generated content from user edits:

```
(user content — preserved across syncs)

<!-- gg:managed:start -->
(generated by gg — regenerated on every sync)
<!-- gg:managed:end -->

(user content — preserved across syncs)
```

- **New PRs/MRs**: Body is wrapped in managed markers automatically.
- **Re-sync with markers**: Only the managed block is replaced; user content above/below is preserved.
- **Legacy PRs (no markers)**: Body is left untouched with a warning — no risk of clobbering manual edits.
- **Content inside the managed block** is regenerated on each sync. Place persistent checklists and notes outside the markers.

## Stack-navigation comments

If the repo's `.git/gg/config.json` has `defaults.stack_nav_comments: true`,
`gg sync` posts and maintains a managed comment on each open PR/MR in the
stack. The comment lists every entry (`#N` on GitHub, `!N` on GitLab) with a
👉 marker on the current one. The comment is identified by a hidden
`<!-- gg:stack-nav -->` marker and managed entirely by git-gud — don't edit
these comments manually, and don't be surprised when `gg sync` adds, updates,
or removes them automatically.

Disable the feature by setting `defaults.stack_nav_comments: false` (the
default). The next `gg sync` then cleans up any existing managed comments.
Reconcile is skipped under `--until` to avoid partial-stack inconsistencies.

## GitLab-specific

- `gg land --auto-merge` is GitLab-only and requests queueing/auto-merge.
- With merge trains enabled, landing may enqueue MRs instead of immediate merge.
- Track train state in `gg ls --json` with:
  - `in_merge_train`
  - `merge_train_position`
- Land action values on GitLab may include `queued` / `already_queued` (in addition to `merged`).
- When `--wait` detects CI failure, the error message includes the names and stages of failed pipeline jobs (fetched from the MR's head pipeline).
- After landing an MR, downstream MRs are automatically retargeted away from the merged branch — no manual retargeting in GitLab UI is needed. `gg sync` also handles this if an MR was merged directly in the UI.
- Use `glab` for auxiliary GitLab checks/actions.
- JSON fields always use `pr_*` naming, even for GitLab MRs (`pr_number`, `pr_state`).

## Provider-neutral notes

- `pr_state` values: `open`, `merged`, `closed`, `draft` (same for both GitHub and GitLab).
- `pr_url` format varies by provider (`/pull/N` for GitHub, `/-/merge_requests/N` for GitLab).

## See also

- Full command + schema reference: `reference.md`
- End-to-end walkthrough: `examples/basic-flow.md`
- Multi-commit editing: `examples/multi-commit.md`
- Merge trains (GitLab): `examples/merge-train.md`
- MCP server tools & schemas: `reference.md` → MCP Server section

## MCP Server Usage for Agents

The `gg-mcp` binary exposes git-gud as an MCP server (stdio transport). Set `GG_REPO_PATH` to the target repo.

### Read-only tools (safe, no side effects)
- `stack_list` / `stack_log` / `stack_list_all` / `stack_status` — inspect stacks (`stack_log` gives a smartlog-style view of the current stack; `stack_list_all` is cross-stack)
- `pr_info` — check PR state, CI, approval
- `config_show` — read repo configuration
- `stack_undo_list` — list recent operations from the per-repo operation log

### Write tools (mutating, use with care)
- `stack_checkout` — create or switch stacks
- `stack_sync` — push and create/update PRs (use `draft: true` for safety)
- `stack_land` — merge approved PRs (**always confirm with user first**)
- `stack_clean` — remove merged stacks
- `stack_rebase` — rebase onto latest base
- `stack_squash` / `stack_absorb` — amend commits
- `stack_reconcile` — fix out-of-sync remote branches
- `stack_drop` — remove commits from the stack (always passes `--yes`; set `force: true` only to bypass the immutability guard for merged/base commits; agent confirms with user before any drop)
- `stack_split` — split a commit using interactive hunk selection (TUI opens by default; pass FILES... to auto-select all hunks for those files)
- `stack_reorder` — reorder commits with explicit order string (no TUI)
- `stack_restack` — repair stack ancestry drift (`dry_run`, `from` params)
- `stack_undo` — reverse the ref/HEAD effects of the most recent mutating `gg` command (refuses on remote-touching ops, returns provider-specific revert hints; agents must surface those hints rather than attempt silent remote rollback)

### Navigation tools
- `stack_move` — jump to a commit by position, GG-ID, or SHA
- `stack_navigate` — move first/last/prev/next in the stack

### Agent guidelines for MCP
- Prefer read-only tools to understand state before writing.
- Use `stack_sync` with `draft: true` for new PRs unless the user asks for non-draft. Note: `draft: true` only affects newly created PRs, not existing ones.
- **Never call `stack_land` without explicit user approval.**
- Parse JSON output from `stack_sync`, `stack_land`, `stack_clean`, and `stack_lint`.
- If `stack_status` shows `behind_base > 0`, run `stack_rebase` before syncing.
- Rewrite tools (`stack_squash`, `stack_absorb`, `stack_reorder`, `stack_split`, `stack_drop`, `stack_rebase`, plus CLI `gg unstack`) will fail with `ImmutableTargets` when a target commit is merged or already on the base branch. Each tool accepts a `force: bool` parameter that maps to `--force` / `--ignore-immutable`. Only set `force: true` after surfacing the affected commits to the user and getting explicit approval. `stack_drop` always passes `--yes` (MCP is non-interactive), but its `force: bool` param is separate from the confirmation-skip — leave it `false` unless the user has approved rewriting merged/base commits.
