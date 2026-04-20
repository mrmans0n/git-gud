# gg Reference

This reference is for using `gg` with **GitHub** (`gh` CLI) or **GitLab** (`glab` CLI).

## Prereqs and setup

```bash
# provider auth
gh auth status      # GitHub
glab auth status    # GitLab

gg setup        # Quick mode: essential settings
gg setup --all  # Full mode: all settings organized by category
```

> **Note:** Network errors during auth check are non-fatal — gg prints a warning and continues. The operation may fail later if authentication is actually required.

Global config (`~/.config/gg/config.json`) provides defaults for all repos. Local config (`.git/gg/config.json`) overrides global.

Example local config:

```json
{
  "defaults": {
    "provider": "github",
    "base": "main",
    "branch_username": "your-github-user",
    "lint": ["cargo fmt --all --check"],
    "sync_draft": false,
    "sync_update_descriptions": true
  }
}
```

```json
{
  "defaults": {
    "provider": "gitlab",
    "base": "main",
    "branch_username": "your-gitlab-user",
    "lint": ["cargo fmt --all --check"],
    "gitlab": {
      "auto_merge_on_land": false
    }
  }
}
```

---

## Commands and flags

### Stack lifecycle

#### `gg co [OPTIONS] [STACK_NAME]`
Create/switch stack, optionally worktree-backed.

- `-b, --base <BASE>`
- `-w, --worktree`

#### `gg ls [OPTIONS]`
List current/all/remote stacks.

- `-a, --all`
- `-r, --refresh`
- `--remote`
- `--json`

#### `gg log [OPTIONS]`
Smartlog-style view of the **current** stack (tree with HEAD marker, PR/CI
badges). Stack-scoped — use `gg ls --all` for cross-stack browsing.

- `-r, --refresh`
- `--json` (auto-refreshes PR/MR state; shape mirrors `gg ls --json` entries
  under a `log` key)

#### `gg inbox [OPTIONS]`
Cross-stack actionable triage view for local stacks.

- `-a, --all` — include merged items too
- `--json` — emit `InboxResponse` with bucketed entries

Buckets are priority-ordered and mutually exclusive:
`ready_to_land`, `changes_requested`, `blocked_on_ci`,
`awaiting_review`, `behind_base`, `draft`, `merged`.

Notes:
- canceled CI is treated as blocked
- transient PR refresh failures keep the entry visible instead of dropping it
- `behind_base` compares the stack tip against `origin/<base>` rather than the local base branch

#### `gg sync [OPTIONS]`
Push and create/update PRs/MRs.

- `-d, --draft`: Create new PRs/MRs as draft (does not convert existing PRs to draft)
- `-f, --force`
- `--update-descriptions`: Update PR/MR titles and descriptions. On update, only the managed block (`<!-- gg:managed:start/end -->`) is replaced — user content outside the markers is preserved. Legacy PRs without markers skip the body update with a warning.
- `-l, --lint` *(aborts sync on lint failure and restores repository state to the pre-sync snapshot)*
- `--no-lint`
- `--no-rebase-check`
- `--no-verify`: Skip the pre-push hook for pushes performed by this sync (forwards `git push --no-verify`)
- `-u, --until <UNTIL>`
- `--json`

#### `gg land [OPTIONS]`
Merge approved PRs/MRs from bottom up. Automatically retargets downstream MRs after each merge (next entry for single land, all remaining for `--all`).

- `-a, --all`
- `--auto-merge` *(GitLab only)*
- `--no-squash`
- `-w, --wait`
- `-u, --until <UNTIL>`
- `-c, --clean`
- `--no-clean`
- `--admin` *(GitHub only)* — bypass branch protection approval requirements
- `--json`

#### `gg clean [OPTIONS]`
Delete merged stacks/worktrees.

- `-a, --all`
- `--json`

### Editing and navigation

#### `gg mv <TARGET>` / `gg first` / `gg last` / `gg prev` / `gg next`
Move around stack entries.

#### `gg sc [OPTIONS]`
Squash changes into current stack commit.

- `-a, --all`
- `-f, --force` (alias: `--ignore-immutable`) — bypass the [immutability guard](#immutable-commits)

#### `gg absorb [OPTIONS]`
Auto-distribute staged changes to matching commits.

- `--dry-run`
- `-a, --and-rebase`
- `-w, --whole-file`
- `--one-fixup-per-commit`
- `-n, --no-limit`
- `-s, --squash`
- `-f, --force` (alias: `--ignore-immutable`) — bypass the [immutability guard](#immutable-commits)

#### `gg reorder [OPTIONS]` (alias: `gg arrange`)
Reorder and/or drop stack entries. Opens an interactive TUI by default where you can move commits with `J`/`K` (or Shift+arrows) and mark commits for dropping with `d`.
#### `gg drop <TARGET>...` *(alias: `gg abandon`)*
Remove one or more commits from the stack. Targets can be positions (1-indexed), short SHAs, or GG-IDs.

- `-y, --yes` — skip the confirmation prompt without bypassing the [immutability guard](#immutable-commits). Use this for non-interactive callers (CI, MCP) that still want merged/base commits protected.
- `-f, --force` (alias: `--ignore-immutable`) — bypass the [immutability guard](#immutable-commits). Implies `--yes`.
- `--json`

#### `gg reorder [OPTIONS]`
Reorder stack entries. Opens an interactive TUI by default where you can move commits with `J`/`K` (or Shift+arrows).

- `-o, --order <ORDER>` — reorder only (no dropping via CLI flag)
- `--no-tui` — disable TUI, use text editor instead (delete lines to drop commits)
- `-f, --force` (alias: `--ignore-immutable`) — bypass the [immutability guard](#immutable-commits)

#### `gg split [OPTIONS] [FILES...]`
Split a commit into two. Selected files/hunks become a new commit inserted before the original.

- `-c, --commit <TARGET>` — target commit (position, SHA, or GG-ID; default: current)
- `-m, --message <MSG>` — message for the new commit
- `--no-edit` — keep original message for remainder, don't prompt
- `--no-tui` — disable TUI, use sequential prompt instead (legacy `git add -p` style)
- `-f, --force` (alias: `--ignore-immutable`) — bypass the [immutability guard](#immutable-commits)
- `FILES...` — auto-select all hunks from these files (opens interactive hunk picker if omitted)

#### `gg rebase [TARGET]`
Rebase current stack onto base or explicit target.

- `-f, --force` (alias: `--ignore-immutable`) — bypass the [immutability guard](#immutable-commits)

#### `gg restack [OPTIONS]`
Repair stack ancestry after manual history changes (amend, cherry-pick, upstream rebase).

- `-n, --dry-run`: Show what would be done without making changes
- `--from <TARGET>`: Repair only from this commit upward (position, SHA, or GG-ID)
- `--json`

### Utilities

#### `gg lint [OPTIONS]`
Run configured lint checks.

- `-u, --until <UNTIL>`
- `--json`

#### `gg run [OPTIONS] -- <COMMAND>...`
Run an arbitrary shell command on each commit in the stack (like `jj run`).

- `--amend`: fold working-tree changes into each commit (formatters, codemods).
- `--discard`: revert working-tree changes after each commit.
- Default (no flag): read-only — fail the run if the command modifies tracked files.
- `--keep-going`: continue past failing commits.
- `-u, --until <UNTIL>`: stop at this commit position.
- `-j, --jobs <N>`: parallel workers for read-only mode (0 = auto, 1 = sequential). Parallel mode uses isolated worktrees per commit.
- `--json`: emit `RunResponse` (see JSON schemas below).

Argv boundaries are preserved across the CLI — `gg run -- git commit -m "msg with spaces"` is passed to the subprocess as five argv elements, not whitespace-split.

#### `gg reconcile [OPTIONS]`
Repair metadata after external branch/PR/MR manipulation.

- Normalizes `GG-ID` and `GG-Parent` trailers across the stack
- `-n, --dry-run`

#### `gg continue` / `gg abort`
Resume/abort paused operations.

#### `gg undo [OPERATION_ID] [--json]` / `gg undo --list [--limit N] [--json]`
Reverse the local ref/HEAD effects of the most recent mutating `gg`
command, backed by a per-repo operation log at
`<commondir>/gg/operations/*.json` (ring buffer, 100 records;
`Pending`/`Interrupted` records never pruned).

- `OPERATION_ID` — target a specific record (`op_…`). When omitted,
  undoes the most recent locally-undoable operation.
- `--list` — show recent operations newest-first.
- `--limit N` — cap `--list` output (default: 20).
- `--json` — emit machine-readable JSON.

Every mutating command (`sc`, `drop`, `split`, `rebase`, `reorder`,
`absorb`, `reconcile`, `restack`, `checkout`, `mv`/`first`/`last`/`prev`/`next`,
`clean`, `sync`, `land`, `run --amend`) snapshots refs before mutating
and records the operation on success. A second `gg undo` redoes the
first — `undo` itself is recorded.

**Refusals** (exit 1, no refs touched):

| `refusal.reason` | Condition | Handling |
|---|---|---|
| `remote` | Op pushed/merged/closed/created a PR or MR | Use the printed provider hint (`gh pr close <n>`, `glab mr close <n>`, `git push --delete …`). |
| `interrupted` | Op was Ctrl-C'd or crashed mid-flight | Fix state manually; stale Pending records are swept on the next lock-acquiring op. |
| `stale` | Refs moved since the target op finalised | Run `gg undo --list` and pick a more recent record. Error names the ref, expected OID, actual OID. |
| `unsupported_schema` | Record was written by a newer `gg` binary | Upgrade `gg` or delete the record. |

`gg undo` never touches the working tree, index, untracked files, or
remotes. It does not support `--all` / `--range` — one operation per
call.

#### `gg setup`
Interactive config wizard.
- **Quick mode** (`gg setup`): Essential settings (provider, base, username)
- **Full mode** (`gg setup --all`): All settings organized by category (General, Sync, Land, Lint, Worktrees, GitLab)

Supports global config at `~/.config/gg/config.json` for shared defaults across repos. New fields: `sync_draft` (create PRs as drafts) and `sync_update_descriptions` (update PR descriptions on re-sync).

#### `defaults.stack_nav_comments`

- **Type:** `boolean`
- **Default:** `false`
- **Effect:** When `true`, `gg sync` posts and maintains a managed "stack
  navigation" comment on each open PR/MR in a multi-entry stack. When `false`
  (default), no such comments are posted; any pre-existing managed comments
  are removed on the next sync. The reconcile pass is skipped when `--until`
  limits a sync.

#### `gg completions <SHELL>`
Generate shell completion (`bash|elvish|fish|powershell|zsh`).

---

## Merge trains and auto-merge (GitLab)

- `gg land --auto-merge` requests GitLab auto-merge instead of immediate merge.
- If merge trains are required by branch policy, MRs are queued into the train.
- Track train state using `gg ls --json` entry fields:
  - `in_merge_train: boolean`
  - `merge_train_position: number | null`
- With `-w/--wait`, `gg land` can wait for approval/readiness transitions.
- GitLab land actions can be `queued`/`already_queued` (in addition to `merged`).
- When `--wait` detects CI failure, the error includes failed job names and stages (e.g., `Failed jobs: lint (stage: test), build-android (stage: build)`).

You can use `glab` for extra inspection (examples):

```bash
glab mr view <iid>
glab mr checks <iid>
```

---

## JSON output schemas (from Rust structs)

All JSON payloads include `version` (`u32`, current value: `1`).

### Common error shape

```json
{
  "version": 1,
  "error": "string"
}
```

### `gg ls --json` (single stack)

```json
{
  "version": 1,
  "stack": {
    "name": "string",
    "base": "string",
    "total_commits": 0,
    "synced_commits": 0,
    "current_position": 1,
    "behind_base": 0,
    "entries": [
      {
        "position": 1,
        "sha": "string",
        "title": "string",
        "gg_id": "c-...",
        "gg_parent": "c-...",
        "pr_number": 123,
        "pr_state": "open",
        "approved": false,
        "ci_status": "success",
        "is_current": true,
        "in_merge_train": false,
        "merge_train_position": null
      }
    ]
  }
}
```

Field types:
- `current_position`: `number | null`
- `behind_base`: `number | null`
- `gg_id`: `string | null`
- `gg_parent`: `string | null`
- `pr_number`: `number | null`
- `pr_state`: `"open" | "merged" | "closed" | "draft" | null`
- `ci_status`: `string | null`
- `in_merge_train`: `boolean` *(GitLab-specific)*
- `merge_train_position`: `number | null` *(GitLab-specific)*

### `gg ls --all --json` (all local stacks)

```json
{
  "version": 1,
  "current_stack": "feature-auth",
  "stacks": [
    {
      "name": "feature-auth",
      "base": "main",
      "commit_count": 2,
      "is_current": true,
      "has_worktree": true,
      "behind_base": 0,
      "commits": [
        { "position": 1, "sha": "abc1234", "title": "feat: ..." }
      ]
    }
  ]
}
```

### `gg ls --remote --json` (remote stacks)

```json
{
  "version": 1,
  "stacks": [
    {
      "name": "feature-auth",
      "commit_count": 2,
      "pr_numbers": [101, 102]
    }
  ]
}
```

### `gg log --json`

```json
{
  "version": 1,
  "log": {
    "stack": "feature-auth",
    "base": "main",
    "current_position": 2,
    "entries": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "feat: add parser",
        "gg_id": "c-abc1234",
        "gg_parent": null,
        "pr_number": 101,
        "pr_state": "open",
        "approved": false,
        "ci_status": "success",
        "is_current": false,
        "in_merge_train": false,
        "merge_train_position": null
      }
    ]
  }
}
```

Entry fields match `gg ls --json` so consumers can share parsers.

### `gg sync --json`

```json
{
  "version": 1,
  "sync": {
    "stack": "feature-auth",
    "base": "main",
    "rebased_before_sync": false,
    "warnings": [],
    "metadata": {
      "gg_ids_added": 0,
      "gg_parents_updated": 0,
      "gg_parents_removed": 0
    },
    "entries": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "feat: add validation",
        "gg_id": "c-abc1234",
        "branch": "user/feature-auth--c-abc1234",
        "action": "created",
        "pr_number": 101,
        "pr_url": "https://host/org/repo/...",
        "draft": false,
        "pushed": true,
        "nav_comment_action": "created",
        "error": null
      }
    ]
  }
}
```

Field types for `entries`:
- `nav_comment_action` (string, optional): action taken on the managed
  stack-nav comment for this entry's PR during this sync. One of
  `"created"`, `"updated"`, `"unchanged"`, `"deleted"`, or `"error"`.
  Omitted when no reconcile action was required.

### `gg lint --json`

```json
{
  "version": 1,
  "lint": {
    "results": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "feat: add validation",
        "passed": true,
        "commands": [
          {
            "command": "cargo clippy",
            "passed": true,
            "output": null
          }
        ]
      }
    ],
    "all_passed": true
  }
}
```

### `gg land --json`

```json
{
  "version": 1,
  "land": {
    "stack": "feature-auth",
    "base": "main",
    "landed": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "feat: add validation",
        "gg_id": "c-abc1234",
        "pr_number": 101,
        "action": "merged",
        "error": null
      }
    ],
    "remaining": 0,
    "cleaned": true,
    "warnings": [],
    "error": null
  }
}
```

> On GitLab with `--auto-merge`, `action` may be `queued` or `already_queued`.

### `gg drop --json`

```json
{
  "version": 1,
  "drop": {
    "dropped": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "feat: add validation"
      }
    ],
    "remaining": 2
  }
}
```

### `gg clean -a --json`

```json
{
  "version": 1,
  "clean": {
    "cleaned": ["feature-auth"],
    "skipped": []
  }
}
```

### `gg restack --json`

```json
{
  "version": 1,
  "restack": {
    "stack_name": "my-feature",
    "total_entries": 4,
    "entries_restacked": 2,
    "entries_ok": 2,
    "dry_run": false,
    "steps": [
      {
        "position": 1,
        "gg_id": "c-abc1234",
        "title": "Add login form",
        "action": "ok",
        "current_parent": null,
        "expected_parent": null
      },
      {
        "position": 2,
        "gg_id": "c-def5678",
        "title": "Add validation",
        "action": "reattach",
        "current_parent": "c-old1111",
        "expected_parent": "c-abc1234"
      }
    ]
  }
}
```

`action` values: `"ok"`, `"reattach"`, `"skip"` (when `--from` is set, entries below the threshold are `"skip"`).

### `gg undo --json`

```json
{
  "version": 1,
  "status": "succeeded",
  "undone": {
    "id": "op_0000001750000000_018f…",
    "kind": "drop",
    "status": "committed",
    "created_at_ms": 1750000000000,
    "args": ["drop", "3"],
    "stack_name": "feature-auth",
    "touched_remote": false,
    "is_undoable": true,
    "is_undo": false,
    "remote_effects": []
  }
}
```

On refusal:

```json
{
  "version": 1,
  "status": "refused",
  "refusal": {
    "reason": "remote",
    "message": "Cannot locally undo 'sync': it touched a remote.",
    "target": { "id": "op_…", "kind": "sync", "touched_remote": true },
    "hints": [
      "Close PR #42: gh pr close 42",
      "Delete remote branch: git push --delete origin user/feature-auth--c-abc1234"
    ]
  }
}
```

`refusal.reason` values: `"remote" | "interrupted" | "stale" | "unsupported_schema"`.

### `gg undo --list --json`

```json
{
  "version": 1,
  "operations": [
    {
      "id": "op_…",
      "kind": "undo",
      "status": "committed",
      "created_at_ms": 1750000100000,
      "args": ["undo"],
      "stack_name": "feature-auth",
      "touched_remote": false,
      "is_undoable": true,
      "is_undo": true,
      "undoes": "op_previous…"
    }
  ]
}
```

Entries are newest-first. Use `is_undoable` to gate UI/agent actions;
`is_undo` + `undoes` render redo markers. Remote-touching ops appear
with `is_undoable: false` and `touched_remote: true`.

---

## Immutable commits

gg refuses by default to let rewrite-style commands (`gg sc`, `gg absorb`,
`gg reorder`/`gg arrange`, `gg split`, `gg drop`, `gg rebase`, `gg restack`) touch commits
that look "already published". A commit is considered immutable when any of
these is true:

- **Merged PR/MR** — the entry's tracked PR/MR state is `merged`.
- **Base ancestor** — the commit is already reachable from `origin/<base>`
  (falling back to the local base branch when the remote ref isn't
  available). Covers anything already landed on the trunk.

When the guard fires, the command exits with an `ImmutableTargets` error
listing each affected position, short SHA, title, and reason, for example:

```
error: cannot rewrite immutable commits (use --force / --ignore-immutable to override):
  #2  abc1234  Fix typo in parser          (merged as !123)
  #3  def5678  Bump dependency             (already in origin/main)
```

To override intentionally, pass `-f` / `--force` (or the longer alias
`--ignore-immutable`) to the command. The override emits a warning listing
what is being rewritten and then proceeds.

Notes for agents:

- Before offering `--force`, surface the guard output to the user and get
  explicit confirmation — bypassing is a footgun.
- `gg rebase` silently skips merged commits — both base-ancestor commits and
  squash-merged PRs (printing `→ Skipping N merged commit(s) already on <base>`)
  — because `git rebase` naturally drops patches already applied upstream via
  patch-id matching.
- `gg sync`'s internal auto-rebase only considers a commit a
  base ancestor **after** the remote ref is fetched, so the guard reflects
  freshly-updated state rather than stale local refs.
- `gg sync`'s other paths (push, PR create/update) are not guarded; only
  history-rewriting commands are.
- `gg land` performs a post-merge cleanup rebase with the guard
  intentionally bypassed (the commits it touches are by definition just
  merged).

## Operational guardrails for agents

- Never run `gg land` without explicit user approval.
- Prefer `gg co -w` for isolated work.
- Always parse `--json` output, do not scrape text.
- If `gg sync --json` includes warnings about stale base, run `gg rebase`.
- For GitLab merge trains, monitor `in_merge_train` and `merge_train_position` in `gg ls --json`.
- If a rewrite command fails with `ImmutableTargets`, explain to the user which commits are immutable and why before offering `--force`.

---

## MCP Server (`gg-mcp`)

An MCP (Model Context Protocol) server binary that exposes git-gud operations as structured tools for AI assistants.

### Setup

```bash
# Set repo path and run
GG_REPO_PATH=/path/to/repo gg-mcp
```

Transport: stdio (JSON-RPC over stdin/stdout).

### Available Tools

#### `stack_list`
List the current stack with commit entries and PR/MR status.
- **Params:** `refresh` (bool, default false) — refresh PR status from remote
- **Returns:** `{ name, base, total_commits, synced_commits, current_position, entries: [{ position, sha, title, gg_id, gg_parent, pr_number, pr_state, approved, ci_status, is_current }] }`

#### `stack_log`
Smartlog-style view of the current stack (stack-scoped). Mirrors `gg log --json`.
- **Params:** `refresh` (bool, default false) — refresh PR status from remote
- **Returns:** `{ stack, base, current_position, entries: [...] }` (entry fields match `stack_list`)

#### `stack_list_all`
List all stacks in the repository.
- **Params:** none
- **Returns:** `{ current_stack, stacks: [{ name, base, commit_count, is_current }] }`

#### `stack_inbox`
Show actionable triage across local stacks. Mirrors `gg inbox --json`.
- **Params:** `all` (bool, default false) — include merged items too
- **Returns:** `{ version, total_items, buckets: { ready_to_land, changes_requested, blocked_on_ci, awaiting_review, behind_base, draft, merged } }` where each bucket entry is `{ stack_name, position, sha, title, pr_number, pr_url, ci_status, behind_base }`

#### `stack_status`
Quick status summary of the current stack.
- **Params:** none
- **Returns:** `{ stack_name, base_branch, total_commits, synced_commits, current_position, behind_base }`

#### `pr_info`
Get detailed PR/MR information by number.
- **Params:** `number` (u64, required)
- **Returns:** `{ number, title, state, url, draft, approved, mergeable, ci_status }`

#### `config_show`
Show repository git-gud configuration.
- **Params:** none
- **Returns:** `{ provider, base_branch, branch_username, lint_commands, auto_add_gg_ids, land_admin, land_auto_clean, sync_auto_lint, sync_auto_rebase }` (`auto_add_gg_ids` is a compatibility field and is always `true`).

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `GG_REPO_PATH` | Path to git repository | Current working directory |

#### `stack_checkout`
Create or switch to a stack.
- **Params:** `name` (string, optional), `base` (string, optional), `worktree` (bool, default false)

#### `stack_sync`
Push branches and create/update PRs.
- **Params:** `draft` (bool), `force` (bool), `update_descriptions` (bool), `no_rebase_check` (bool), `lint` (bool), `until` (string), `no_verify` (bool — skip pre-push hook)
- **Returns:** JSON sync results with PR URLs

#### `stack_land`
Merge approved PRs.
- **Params:** `all` (bool), `squash` (bool), `auto_clean` (bool), `until` (string), `admin` (bool)
- **Returns:** JSON land results

#### `stack_clean`
Clean up merged stacks.
- **Params:** `all` (bool)
- **Returns:** JSON with cleaned stacks

#### `stack_rebase`
Rebase stack onto latest base.
- **Params:** `target` (string, optional), `force` (bool, default false) — bypass the [immutability guard](#immutable-commits)

#### `stack_squash`
Squash staged changes into current commit.
- **Params:** `all` (bool), `force` (bool, default false) — bypass the [immutability guard](#immutable-commits)

#### `stack_absorb`
Auto-absorb staged changes into correct commits.
- **Params:** `dry_run` (bool), `and_rebase` (bool), `whole_file` (bool), `one_fixup_per_commit` (bool), `squash` (bool), `force` (bool, default false) — bypass the [immutability guard](#immutable-commits)

#### `stack_reconcile`
Reconcile out-of-sync remote branches.
- **Params:** `dry_run` (bool)

#### `stack_move`
Move to a specific commit in the stack.
- **Params:** `target` (string, required) — position, GG-ID, or SHA

#### `stack_navigate`
Navigate within the stack.
- **Params:** `direction` (string, required) — "first", "last", "prev", "next"

#### `stack_lint`
Run lint on stack commits.
- **Params:** `until` (usize, optional) — lint up to this position
- **Returns:** JSON with per-commit lint results

#### `stack_drop`
Remove commits from the stack.
- **Params:**
  - `targets` (string[], required) — commits to drop: positions (1-indexed), short SHAs, or GG-IDs
  - `force` (bool, optional, default `false`) — bypass the [immutability guard](#immutable-commits) for merged/base-ancestor commits. When `false`, drops still succeed for regular commits; merged/base commits are refused with `ImmutableTargets`.
- **Notes:** Always passes `--yes` to skip the interactive prompt (MCP is non-interactive). `force` is a separate opt-in so MCP drop does not silently rewrite already-published commits. Agent must confirm any drop with the user beforehand, and must surface the merged/base-ancestor reasons before retrying with `force: true`.
- **Returns:** JSON with dropped commits and remaining count

#### `stack_split`
Split a commit by moving specified files to a new commit.
- **Params:**
  - `commit` (string, optional) — target commit: position, SHA, or GG-ID (default: current)
  - `files` (string[], required) — files to include in the new commit
  - `message` (string, optional) — message for the new commit
  - `no_edit` (bool, default false) — skip prompt for remainder commit message
  - `force` (bool, default false) — bypass the [immutability guard](#immutable-commits)
- **Notes:** File-level only (`--no-tui` implicit). Hunk-level selection not available via MCP.
- **Returns:** Result of the split operation

#### `stack_reorder`
Reorder commits in the stack.
- **Params:**
  - `order` (string, required) — new order as positions (1-indexed), e.g., "3,1,2" or "3 1 2". Position 1 = bottom (closest to base).
  - `force` (bool, default false) — bypass the [immutability guard](#immutable-commits)
- **Notes:** Direct mode only (`--no-tui` implicit). No interactive TUI via MCP.
- **Returns:** Result of the reorder operation

#### `stack_restack`
Repair stack ancestry drift after manual history changes.
- **Params:**
  - `dry_run` (bool, default false) — show plan without making changes
  - `from` (string, optional) — repair only from this position, GG-ID, or SHA upward
- **Returns:** JSON `RestackResponse` with per-step plan and execution results

#### `stack_undo`
Reverse the ref/HEAD effects of the most recent mutating `gg` command
(or a specific op by id). Shell-out wrapper around `gg undo --json`.
- **Params:**
  - `operation_id` (string, optional) — target a specific record from
    `stack_undo_list`. Defaults to the most-recent-undoable operation.
- **Notes:** Refuses on remote-touching ops (`sync`, `land`) and on
  `interrupted`, `stale`, or `unsupported_schema` records. On refusal
  the payload includes `refusal.reason` plus provider-specific revert
  hints (`gh pr close <n>`, `glab mr close <n>`, `git push --delete …`).
  Agents must surface the hints to the user rather than attempt silent
  remote rollback. Working tree is never modified.
- **Returns:** `{ status: "succeeded", undone: {…} }` or
  `{ status: "refused", refusal: { reason, message, target, hints } }`.

#### `stack_undo_list`
List recent operations from the per-repo operation log, newest-first.
Shell-out wrapper around `gg undo --list --json`.
- **Params:**
  - `limit` (usize, optional) — cap output (default: 20)
- **Notes:** Each entry carries `is_undoable` (gate for safe local
  replay), `is_undo` + `undoes` (redo markers), and `touched_remote`
  (set on remote-touching ops, which appear with `is_undoable: false`).
- **Returns:** `{ operations: [{ id, kind, status, created_at_ms, args,
  stack_name, touched_remote, is_undoable, is_undo, undoes? }] }`.
