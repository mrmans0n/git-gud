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
- `--admin`: *(GitHub only)* Use admin privileges to bypass branch protection requirements (see [Admin Override](#admin-override) below)
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

# Bypass approval requirements (GitHub admin)
gg land --admin

# Land full stack with admin override
gg land --all --wait --admin
```

## Admin Override

The `--admin` flag (or `land_admin` config default) passes `--admin` to `gh pr merge`, which uses GitHub's API-level admin merge. This bypasses **all** branch protection rules the merging user has permission to override, which may include both review approvals **and** required status checks depending on your repository settings.

Use `--wait --admin` if you want to wait for CI to pass before merging while still bypassing approval requirements. Without `--wait`, no client-side CI validation is performed.

On GitLab, `--admin` is a no-op — `glab mr merge` has no equivalent flag. A warning is printed and the merge proceeds normally.

A warning (`⚠ Merging with admin override`) is printed before each admin-elevated merge.

## Merge Trains (GitLab)

When merge trains are enabled on the target branch, `gg land` automatically adds MRs to the merge train instead of merging directly.

**Approval is always required** before an MR can enter the merge train queue — even with `--all`. If using `--wait`, the command will show "Waiting for approval..." until a reviewer approves the MR.

## CI Failure Details

When using `--wait`, if CI fails on an MR the command stops and shows which jobs failed:

```
OK Landed 1 MR(s)

⚠ Landed 1 MR(s), but encountered an error:

Error: MR !7621 CI failed
  Failed jobs: lint (stage: test), build-android (stage: build)
```

This helps diagnose CI issues without having to open the GitLab UI. Failed job names and stages are fetched from the MR's head pipeline.

## JSON Output

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
