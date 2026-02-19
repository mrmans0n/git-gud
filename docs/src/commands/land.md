# `gg land`

Merge approved PRs/MRs from the bottom of your stack upward.

```bash
gg land [OPTIONS]
```

## Options

- `-a, --all`: Land all approved entries in sequence
- `--auto-merge`: *(GitLab only)* Request auto-merge instead of immediate merge
- `--no-squash`: Disable squash merge (squash is default)
- `-w, --wait`: Wait for CI and approvals before merging
- `-u, --until <UNTIL>`: Land up to a target entry (position, GG-ID, SHA)
- `-c, --clean`: Clean stack automatically after landing all
- `--no-clean`: Disable auto-clean for this run
- `--json`: Emit machine-readable JSON output (no human logs)

## Examples

```bash
# Land one approved entry
gg land

# Land complete stack, waiting for readiness
gg land --all --wait

# Land part of stack
gg land --until 2

# GitLab auto-merge queue
gg land --all --auto-merge

# JSON output for automation
gg land --all --json
```

Example JSON response:

```json
{
  "version": 1,
  "land": {
    "stack": "my-stack",
    "base": "main",
    "landed": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "feat: add parser",
        "gg_id": "c-abc1234",
        "pr_number": 42,
        "action": "merged",
        "error": null
      }
    ],
    "remaining": 0,
    "cleaned": false,
    "warnings": [],
    "error": null
  }
}
```
