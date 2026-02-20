# gg-gitlab Reference

This reference is for using `gg` with **GitLab** (`glab` CLI), including merge trains.

## Prereqs and setup

```bash
glab auth status
gg setup
```

Manual config (`.git/gg/config.json`):

```json
{
  "defaults": {
    "provider": "gitlab",
    "base": "main",
    "branch_username": "your-gitlab-user",
    "lint": ["cargo fmt --all --check"],
    "gitlab": {
      "auto_merge_on_land": false
    }
  }
}
```

---

## Commands and flags

### Stack lifecycle

#### `gg co [OPTIONS] [STACK_NAME]`
- `-b, --base <BASE>`
- `-w, --worktree`

#### `gg ls [OPTIONS]`
- `-a, --all`
- `-r, --refresh`
- `--remote`
- `--json`

#### `gg sync [OPTIONS]`
- `-d, --draft`
- `-f, --force`
- `--update-descriptions`
- `-l, --lint`
- `--no-lint`
- `--no-rebase-check`
- `-u, --until <UNTIL>`
- `--json`

#### `gg land [OPTIONS]`
- `-a, --all`
- `--auto-merge` *(GitLab only)*
- `--no-squash`
- `-w, --wait`
- `-u, --until <UNTIL>`
- `-c, --clean`
- `--no-clean`
- `--json`

#### `gg clean [OPTIONS]`
- `-a, --all`
- `--json`

### Editing and navigation

#### `gg mv <TARGET>` / `gg first` / `gg last` / `gg prev` / `gg next`
Move between stack entries.

#### `gg sc [OPTIONS]`
- `-a, --all`

#### `gg absorb [OPTIONS]`
- `--dry-run`
- `-a, --and-rebase`
- `-w, --whole-file`
- `--one-fixup-per-commit`
- `-n, --no-limit`
- `-s, --squash`

#### `gg reorder [OPTIONS]`
- `-o, --order <ORDER>`

#### `gg rebase [TARGET]`
Rebase stack onto base/target.

### Utilities

#### `gg lint [OPTIONS]`
- `-u, --until <UNTIL>`
- `--json`

#### `gg reconcile [OPTIONS]`
- `-n, --dry-run`

#### `gg continue` / `gg abort`
Continue/abort paused operations.

#### `gg setup`
Interactive setup.

#### `gg completions <SHELL>`
Generate shell completion scripts.

---

## Merge trains and auto-merge (GitLab)

- `gg land --auto-merge` requests GitLab auto-merge instead of immediate merge.
- If merge trains are required by branch policy, MRs are queued into the train.
- Track train state using `gg ls --json` entry fields:
  - `in_merge_train: boolean`
  - `merge_train_position: number | null`
- With `-w/--wait`, `gg land` can wait for approval/readiness transitions.

You can use `glab` for extra inspection (examples):

```bash
glab mr view <iid>
glab mr checks <iid>
```

---

## JSON output schemas (from Rust structs)

All JSON payloads include `version` (`u32`, currently `1`).

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
        "pr_state": "opened",
        "approved": false,
        "ci_status": "success",
        "is_current": true,
        "in_merge_train": true,
        "merge_train_position": 2
      }
    ]
  }
}
```

### `gg ls --all --json`

```json
{
  "version": 1,
  "current_stack": "feature-payments",
  "stacks": [
    {
      "name": "feature-payments",
      "base": "main",
      "commit_count": 3,
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

### `gg ls --remote --json`

```json
{
  "version": 1,
  "stacks": [
    {
      "name": "feature-payments",
      "commit_count": 3,
      "pr_numbers": [21, 22, 23]
    }
  ]
}
```

### `gg sync --json`

```json
{
  "version": 1,
  "sync": {
    "stack": "feature-payments",
    "base": "main",
    "rebased_before_sync": false,
    "warnings": [],
    "entries": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "feat: add payments DTO",
        "gg_id": "c-abc1234",
        "branch": "user/feature-payments--c-abc1234",
        "action": "updated",
        "pr_number": 21,
        "pr_url": "https://gitlab.com/group/proj/-/merge_requests/21",
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
        "title": "feat: add payments DTO",
        "passed": true,
        "commands": [
          { "command": "cargo fmt --all --check", "passed": true, "output": null }
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
    "stack": "feature-payments",
    "base": "main",
    "landed": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "feat: add payments DTO",
        "gg_id": "c-abc1234",
        "pr_number": 21,
        "action": "auto-merge-requested",
        "error": null
      }
    ],
    "remaining": 2,
    "cleaned": false,
    "warnings": [],
    "error": null
  }
}
```

### `gg clean --json`

```json
{
  "version": 1,
  "clean": {
    "cleaned": ["feature-payments"],
    "skipped": []
  }
}
```

---

## Operational guardrails for agents

- Never land without explicit user confirmation.
- Always call JSON-capable commands with `--json`.
- Prefer worktree stacks (`gg co -w`).
- Rebase if behind-base warnings appear before syncing.
- For merge trains, verify `in_merge_train` and `merge_train_position` in `gg ls --json`.
