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
```
