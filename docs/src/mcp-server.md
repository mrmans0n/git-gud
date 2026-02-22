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

**Returns:** Provider, base branch, branch username, lint commands, and all boolean settings (auto_add_gg_ids, land_auto_clean, sync_auto_lint, sync_auto_rebase).

## Transport

The MCP server uses **stdio** transport (JSON-RPC over stdin/stdout), which is the standard for local MCP tools. No network configuration is needed.
