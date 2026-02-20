---
name: gg-github
description: Use git-gud (gg) to manage stacked diffs with GitHub PRs. Use this when creating stacks, syncing updates, checking CI/review state, and landing approved work safely.
---

# gg-github

Use this skill to operate **git-gud (`gg`) as a CLI tool** for day-to-day stacked-diff workflows on GitHub.

## When to use

- You need multiple PRs that depend on each other
- You need to sync stack changes and keep review metadata updated
- You need machine-readable command output for automation (`--json`)

## Prerequisites

- `gg` installed
- `gh` installed and authenticated (`gh auth status`)
- Git repo with `gg` initialized (`gg setup`)

## Setup

### Interactive wizard (recommended)

```bash
gg setup
```

### Manual setup (`.git/gg/config.json`)

```json
{
  "version": 2,
  "base_branch": "main",
  "username": "your-github-user",
  "provider": "github",
  "lint_commands": ["cargo fmt --all --check", "cargo clippy -- -D warnings"]
}
```

## Core workflow

1. Create/switch stack (prefer worktree):

```bash
gg co -w feature-auth
```

2. Commit logical changes:

```bash
git add -A
git commit -m "feat: add input validation"
```

3. Check stack state:

```bash
gg ls --json
```

4. Publish/update PR chain:

```bash
gg sync --json
```

5. When approved + green CI, land **only after user confirmation**:

```bash
gg land -a -c --json
```

## Agent operating rules (mandatory)

1. **Never run `gg land` without explicit user confirmation.**
2. **Always use `--json`** for `gg ls`, `gg sync`, `gg land`, `gg clean`, and `gg lint`.
3. **Prefer worktrees** for isolation (`gg co -w <stack>`).
4. Verify `approved: true` and `ci_status` success before landing.
5. If sync warns stack is behind base, run `gg rebase` first.
6. Prefer `gg absorb -s` for multi-commit edits.

## Common operations

- Navigate: `gg mv`, `gg first`, `gg last`, `gg prev`, `gg next`
- Amend current commit: `gg sc` / `gg sc -a`
- Auto-distribute staged hunks: `gg absorb -s`
- Reorder stack: `gg reorder -o "3,1,2"`
- Sync subset: `gg sync -u <position|gg-id|sha> --json`
- Lint stack: `gg lint --json`
- Clean merged stacks: `gg clean --json`

## See also

- Full command + schema reference: `reference.md`
- End-to-end walkthrough: `examples/basic-flow.md`
- Advanced stack editing: `examples/multi-commit.md`
