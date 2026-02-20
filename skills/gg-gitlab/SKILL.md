---
name: gg-gitlab
description: Use git-gud (gg) to manage stacked diffs with GitLab merge requests, including merge trains and auto-merge workflows.
---

# gg-gitlab

Use this skill to operate **git-gud (`gg`) as a CLI tool** for stacked-diff workflows on GitLab.

## When to use

- You need dependent MRs in a reviewable stack
- You need merge-train-aware landing behavior
- You need structured automation via JSON output

## Prerequisites

- `gg` installed
- `glab` installed and authenticated (`glab auth status`)
- Git repo initialized with `gg setup`

## Setup

### Interactive wizard

```bash
gg setup
```

### Manual setup (`.git/gg/config.json`)

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

1. Start stack in a worktree:

```bash
gg co -w feature-payments
```

2. Create commits and verify:

```bash
git add -A
git commit -m "feat: add payment DTO"
gg ls --json
```

3. Sync to create/update MRs:

```bash
gg sync --json
```

4. Land once approved; use merge-train aware options where needed:

```bash
# direct merge behavior
gg land -a -c --json

# queue via GitLab auto-merge/merge train
gg land -a --auto-merge -w --json
```

## Agent operating rules (mandatory)

1. **Never run `gg land` without explicit user confirmation.**
2. **Always use `--json`** for `gg ls`, `gg sync`, `gg land`, `gg clean`, and `gg lint`.
3. **Prefer worktrees** with `gg co -w <stack>`.
4. Check `approved: true` and CI success in `gg ls --json` before landing.
5. For merge trains, monitor `in_merge_train` and `merge_train_position`.
6. If stack is behind base, run `gg rebase` before syncing.

## GitLab notes

- Use `glab` for any auxiliary GitLab checks/actions.
- `gg land --auto-merge` is GitLab-specific and requests queueing/auto-merge.
- With merge trains enabled, land may enqueue MRs instead of immediate merge.

## See also

- Full command + schema reference: `reference.md`
- End-to-end walkthrough: `examples/basic-flow.md`
- Merge-train focused workflow: `examples/merge-train.md`
