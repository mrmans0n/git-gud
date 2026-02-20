# `gg ls`

List the current stack, all local stacks, or remote-only stacks.

When the stack base is behind `origin/<base>`, output includes a `↓N` indicator (`N` = commits behind).

```bash
gg ls [OPTIONS]
```

## Options

- `-a, --all`: Show all local stacks
- `-r, --refresh`: Refresh PR/MR status from remote
- `--remote`: List remote stacks not checked out locally. Stacks whose PRs/MRs are all merged are shown in a separate "Landed" section at the bottom with a `✓` marker
- `--json`: Print structured JSON output (for scripts and automation). Automatically performs a best-effort refresh of PR/MR state from the provider API, so `pr_state` and `ci_status` fields are populated without needing `--refresh`.

## Examples

```bash
# Current stack status
gg ls

# All local stacks
gg ls --all

# Remote stacks (active first, then landed)
gg ls --remote

# Refresh status badges from provider
gg ls --refresh

# Structured JSON for automation
gg ls --json
gg ls --all --json
gg ls --remote --json
```
