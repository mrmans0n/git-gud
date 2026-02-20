# gg-github Reference

This reference is for using `gg` with **GitHub** (`gh` CLI).

## Prereqs and setup

```bash
gh auth status
gg setup
```

Manual config (`.git/gg/config.json`):

```json
{
  "defaults": {
    "provider": "github",
    "base": "main",
    "branch_username": "your-github-user",
    "lint": ["cargo fmt --all --check"]
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

#### `gg sync [OPTIONS]`
Push and create/update PRs.

- `-d, --draft`
- `-f, --force`
- `--update-descriptions`
- `-l, --lint`
- `--no-lint`
- `--no-rebase-check`
- `-u, --until <UNTIL>`
- `--json`

#### `gg land [OPTIONS]`
Merge approved PRs from bottom up.

- `-a, --all`
- `--no-squash`
- `-w, --wait`
- `-u, --until <UNTIL>`
- `-c, --clean`
- `--no-clean`
- `--json`

> `--auto-merge` is GitLab-only and not used in this GitHub skill.

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

#### `gg absorb [OPTIONS]`
Auto-distribute staged changes to matching commits.

- `--dry-run`
- `-a, --and-rebase`
- `-w, --whole-file`
- `--one-fixup-per-commit`
- `-n, --no-limit`
- `-s, --squash`

#### `gg reorder [OPTIONS]`
Reorder stack entries.

- `-o, --order <ORDER>`

#### `gg rebase [TARGET]`
Rebase current stack onto base or explicit target.

### Utilities

#### `gg lint [OPTIONS]`
Run configured lint checks.

- `-u, --until <UNTIL>`
- `--json`

#### `gg reconcile [OPTIONS]`
Repair metadata after external branch/PR manipulation.

- `-n, --dry-run`

#### `gg continue` / `gg abort`
Resume/abort paused operations.

#### `gg setup`
Interactive config wizard.

#### `gg completions <SHELL>`
Generate shell completion (`bash|elvish|fish|powershell|zsh`).

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
- `pr_number`: `number | null`
- `pr_state`: `string | null`
- `ci_status`: `string | null`
- `in_merge_train`: `boolean`
- `merge_train_position`: `number | null`

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

### `gg sync --json`

```json
{
  "version": 1,
  "sync": {
    "stack": "feature-auth",
    "base": "main",
    "rebased_before_sync": false,
    "warnings": [],
    "entries": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "feat: add validation",
        "gg_id": "c-abc1234",
        "branch": "user/feature-auth--c-abc1234",
        "action": "created",
        "pr_number": 101,
        "pr_url": "https://github.com/org/repo/pull/101",
        "draft": false,
        "pushed": true,
        "error": null
      }
    ]
  }
}
```

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

---

## Operational guardrails for agents

- Never run `gg land` without explicit user approval.
- Prefer `gg co -w` for isolated work.
- Always parse `--json` output, do not scrape text.
- If `gg sync --json` includes warnings about stale base, run `gg rebase`.
