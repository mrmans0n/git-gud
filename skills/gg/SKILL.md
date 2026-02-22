---
name: gg
description: Use git-gud (gg) to manage stacked diffs with GitHub PRs or GitLab MRs. Use this when creating stacks, syncing updates, checking CI/review state, and landing approved work safely.
---

# gg

Use this skill to operate **git-gud (`gg`) as a CLI tool** for day-to-day stacked-diff workflows across GitHub and GitLab.

## When to use

- You need multiple PRs/MRs that depend on each other
- You need to sync stack changes and keep review metadata updated
- You need machine-readable command output for automation (`--json`)

## Prerequisites

- `gg` installed
- Provider CLI installed + authenticated:
  - GitHub: `gh auth status`
  - GitLab: `glab auth status`
- Git repo with `gg` initialized (`gg setup`)

## Setup

### Interactive wizard (recommended)

```bash
gg setup
```

### Manual setup (`.git/gg/config.json`)

```json
{
  "defaults": {
    "provider": "github",
    "base": "main",
    "branch_username": "your-github-user",
    "lint": ["cargo fmt --all --check", "cargo clippy -- -D warnings"]
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

## Core workflow

1. Create/switch stack (prefer worktree):

```bash
gg co -w feature-auth
```

2. Commit logical changes:

```bash
git add <files>
git commit -m "feat: add input validation"
```

3. Check stack state:

```bash
gg ls --json
```

4. Publish/update PR/MR chain:

```bash
gg sync --json
```

5. When approved + green CI, land **only after user confirmation**:

```bash
gg land -a -c --json
```

## Agent operating rules (mandatory)

1. **Never run `gg land` without explicit user confirmation.**
2. **Always use `--json`** for `gg ls`, `gg sync`, `gg land`, `gg clean -a`, and `gg lint`.
3. **Prefer worktrees** for isolation (`gg co -w <stack>`).
4. Verify `approved: true` and `ci_status` success before landing.
5. If sync warns stack is behind base, run `gg rebase` first.
6. Prefer `gg absorb -s` for multi-commit edits.
7. **Never use `git add -A` blindly.** Review `git status` first and only stage intended files. Use `git add <specific-files>` to avoid leaking secrets, env files, or unrelated changes.

## Common operations

- Navigate: `gg mv`, `gg first`, `gg last`, `gg prev`, `gg next`
- Amend current commit: `gg sc` / `gg sc -a`
- Auto-distribute staged hunks: `gg absorb -s`
- Reorder stack: `gg reorder -o "3,1,2"`
- Sync subset: `gg sync -u <position|gg-id|sha> --json`
- Lint stack: `gg lint --json`
- Clean merged stacks: `gg clean -a --json`

## GitLab-specific

- `gg land --auto-merge` is GitLab-only and requests queueing/auto-merge.
- With merge trains enabled, landing may enqueue MRs instead of immediate merge.
- Track train state in `gg ls --json` with:
  - `in_merge_train`
  - `merge_train_position`
- Land action values on GitLab may include `queued` / `already_queued` (in addition to `merged`).
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
- `stack_list` / `stack_list_all` / `stack_status` — inspect stacks
- `pr_info` — check PR state, CI, approval
- `config_show` — read repo configuration

### Write tools (mutating, use with care)
- `stack_checkout` — create or switch stacks
- `stack_sync` — push and create/update PRs (use `draft: true` for safety)
- `stack_land` — merge approved PRs (**always confirm with user first**)
- `stack_clean` — remove merged stacks
- `stack_rebase` — rebase onto latest base
- `stack_squash` / `stack_absorb` — amend commits
- `stack_reconcile` — fix out-of-sync remote branches

### Navigation tools
- `stack_move` — jump to a commit by position, GG-ID, or SHA
- `stack_navigate` — move first/last/prev/next in the stack

### Agent guidelines for MCP
- Prefer read-only tools to understand state before writing.
- Use `stack_sync` with `draft: true` unless the user asks for non-draft PRs.
- **Never call `stack_land` without explicit user approval.**
- Parse JSON output from `stack_sync`, `stack_land`, `stack_clean`, and `stack_lint`.
- If `stack_status` shows `behind_base > 0`, run `stack_rebase` before syncing.
