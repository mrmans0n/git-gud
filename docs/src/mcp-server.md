# MCP Server

git-gud includes an MCP (Model Context Protocol) server that allows AI assistants like Claude Desktop, Cursor, and other MCP-compatible tools to interact with your stacked-diffs workflows programmatically.

## Installation

The `gg-mcp` binary is distributed alongside `gg`. If you installed via Homebrew or cargo-dist, it should already be available.

## Configuration

### Claude Desktop

Add this to your Claude Desktop MCP configuration (`~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "git-gud": {
      "command": "gg-mcp",
      "env": {
        "GG_REPO_PATH": "/path/to/your/repo"
      }
    }
  }
}
```

### Cursor / Other MCP Clients

Configure `gg-mcp` as a stdio-based MCP server. Set `GG_REPO_PATH` to point to your repository.

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `GG_REPO_PATH` | Path to the git repository | Current working directory |

## Available Tools

### `stack_list`

List the current stack with commit entries and PR/MR status.

**Parameters:**
- `refresh` (boolean, optional): Refresh PR/MR status from remote before listing. Default: `false`.

**Returns:** Stack name, base branch, commit entries with positions, SHAs, titles, GG-IDs, PR numbers, states, CI status, and approval status.

### `stack_log`

Render the current stack as a smartlog-style view (stack-scoped). Mirrors the CLI `gg log --json` output.

**Parameters:**
- `refresh` (boolean, optional): Refresh PR/MR status from remote before rendering. Default: `false`.

**Returns:** `{ stack, base, current_position, entries: [...] }`. Entry fields match `stack_list`. Use `stack_list_all` when you need a cross-stack overview.

### `stack_list_all`

List all stacks in the repository with summary information.

**Parameters:** None.

**Returns:** Current stack name and a list of all stacks with name, base branch, commit count, and whether each is the current stack.

### `stack_status`

Get a quick status summary of the current stack.

**Parameters:** None.

**Returns:** Stack name, base branch, total commits, synced commits, current position, and how many commits behind the base branch.

### `pr_info`

Get detailed information about a specific PR/MR by number.

**Parameters:**
- `number` (integer, required): The PR/MR number to look up.

**Returns:** PR number, title, state (open/merged/closed/draft), URL, draft status, approval status, mergeability, and CI status.

### `config_show`

Show the current git-gud configuration for this repository.

**Parameters:** None.

**Returns:** Provider, base branch, branch username, lint commands, and boolean settings (including compatibility field `auto_add_gg_ids`, which is always returned as `true`).

## Write Tools

### `stack_checkout`

Create a new stack or switch to an existing one.

**Parameters:**
- `name` (string, optional): Stack name.
- `base` (string, optional): Base branch (default: main/master).
- `worktree` (boolean, optional): Use a git worktree for isolation.

### `stack_sync`

Push branches and create/update PRs/MRs for the current stack.

**Parameters:**
- `draft` (boolean, optional): Create PRs as draft.
- `force` (boolean, optional): Force-push branches.
- `update_descriptions` (boolean, optional): Update PR descriptions from commit messages.
- `no_rebase_check` (boolean, optional): Skip rebase-needed check.
- `lint` (boolean, optional): Run lint before syncing.
- `until` (string, optional): Only sync up to this position/GG-ID/SHA.

### `stack_land`

Merge approved PRs/MRs from the current stack.

**Parameters:**
- `all` (boolean, optional): Land all approved PRs.
- `squash` (boolean, optional): Use squash merge.
- `auto_clean` (boolean, optional): Auto-clean the stack after landing.
- `until` (string, optional): Only land up to this position/GG-ID/SHA.

### `stack_clean`

Clean up stacks whose PRs have been merged.

**Parameters:**
- `all` (boolean, optional): Clean all merged stacks.

### `stack_rebase`

Rebase the current stack onto the latest base branch.

**Parameters:**
- `target` (string, optional): Target branch to rebase onto.

### `stack_squash`

Squash (amend) staged changes into the current commit.

**Parameters:**
- `all` (boolean, optional): Stage all changes first.

### `stack_absorb`

Auto-absorb staged changes into the correct commits.

**Parameters:**
- `dry_run` (boolean, optional): Show what would be absorbed.
- `and_rebase` (boolean, optional): Rebase after absorbing.
- `whole_file` (boolean, optional): Absorb whole files.
- `one_fixup_per_commit` (boolean, optional): One fixup per target commit.
- `squash` (boolean, optional): Squash fixups immediately.

### `stack_reconcile`

Reconcile out-of-sync branches pushed outside of gg.

**Parameters:**
- `dry_run` (boolean, optional): Show what would change.

### `stack_move`

Move to a specific commit in the stack.

**Parameters:**
- `target` (string, required): Position number, GG-ID, or SHA prefix.

### `stack_navigate`

Navigate within the stack.

**Parameters:**
- `direction` (string, required): `"first"`, `"last"`, `"prev"`, or `"next"`.

### `stack_lint`

Run configured lint commands on each commit.

**Parameters:**
- `until` (integer, optional): Only lint up to this position.

### `stack_drop`

Remove commits from the stack.

**Parameters:**
- `targets` (array of strings, required): Commits to drop—positions (1-indexed), short SHAs, or GG-IDs.

**Notes:** Always uses `--force` (the agent is expected to confirm with the user before calling). Returns JSON with dropped commits.

### `stack_split`

Split a commit into two by moving specified files to a new commit.

**Parameters:**
- `commit` (string, optional): Target commit—position (1-indexed), short SHA, or GG-ID. Defaults to the current commit.
- `files` (array of strings, required): Files to include in the new commit.
- `message` (string, optional): Message for the new (first) commit.
- `no_edit` (boolean, optional): Don't prompt for the remainder commit message.

**Notes:** File-level only (no hunk selection via MCP). The new commit is inserted *before* the original.

### `stack_reorder`

Reorder commits in the stack with an explicit order.

**Parameters:**
- `order` (string, required): New order as positions (1-indexed), e.g., `"3,1,2"` or `"3 1 2"`.

**Notes:** No TUI via MCP. The order specifies the new bottom-to-top arrangement of commits.

### `stack_undo`

Undo the most recent locally-undoable operation (or a specific one by id).

**Parameters:**
- `operation_id` (string, optional): Target a specific operation id from
  `stack_undo_list`. When omitted, the most recent locally-undoable op is
  rolled back.

**Notes:** Refuses operations that touched a remote (sync/land — use
the provider's own tooling), interrupted operations, and stale operations
whose captured refs have since moved. Redo is a second call with no
`operation_id` — the undo itself is recorded, so undoing it replays the
original change. Returns JSON with `status: "succeeded" | "refused"`.

### `stack_undo_list`

List recent operations from the operation log, newest first.

**Parameters:**
- `limit` (integer, optional): Maximum entries to return. Default `100`.

**Notes:** Returns a JSON `{ version, operations: [...] }` payload. Each
entry carries `id`, `kind`, `status`, `created_at_ms`, `args`,
`touched_remote`, and `is_undoable`. Use an entry's `id` with
`stack_undo`.

## Transport

The MCP server uses **stdio** transport (JSON-RPC over stdin/stdout), which is the standard for local MCP tools. No network configuration is needed.
