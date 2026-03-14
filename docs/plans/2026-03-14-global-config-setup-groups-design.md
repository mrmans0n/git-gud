# Global Config & Setup Groups Design

**Goal:** Add a global config file for personal defaults and reorganize `gg setup` into grouped categories with a quick vs full mode.

## Feature 1: Global Config File

### Location

`~/.config/gg/config.json` — same JSON schema as the per-repo config (the `defaults` section only, no `stacks`).

Follows XDG convention. On macOS, `~/.config/gg/` is used (not `~/Library/`), consistent with other CLI tools like `gh`.

### Schema

The global config file uses the same `Defaults` struct but wrapped in a simpler container:

```json
{
  "defaults": {
    "provider": "github",
    "base": "main",
    "branch_username": "nacho",
    "auto_add_gg_ids": true,
    "sync_auto_rebase": true,
    "sync_behind_threshold": 1,
    "sync_auto_lint": false,
    "land_auto_clean": false,
    "land_wait_timeout_minutes": 30,
    "unstaged_action": "ask",
    "lint": [],
    "gitlab": {
      "auto_merge_on_land": false
    }
  },
  "worktree_base_path": null
}
```

### Resolution Order

1. Start with hardcoded defaults (what `Defaults::default()` returns today)
2. Merge global config on top (field by field, only non-null values)
3. Merge repo-local config on top (field by field, only non-null values)

This means:
- Global config provides personal defaults across all repos
- Repo-local config overrides specific fields for that repo
- Unset fields fall through to the next layer

### Management

The global config is **manually edited** — no dedicated wizard. Users create `~/.config/gg/config.json` themselves or copy from a repo config.

### Implementation Details

- New function: `Config::load_global()` — reads from `~/.config/gg/config.json`
- New function: `Config::load_with_global(git_dir)` — loads global first, then merges repo-local on top
- Existing `Config::load(git_dir)` behavior unchanged for backwards compat
- All commands that currently call `Config::load()` switch to `Config::load_with_global()`
- The merge is at the `Defaults` level: for each field, if the local value is the "default" sentinel (None for Options, false for bools, 0 for numbers, empty vec for lint), use the global value instead

### Merge Strategy Detail

The tricky part: how to distinguish "user explicitly set `sync_auto_rebase: false`" from "field was never set and got the default `false`". Two options:

**Option A: Use `Option<T>` for all fields in global config**
- Wrap every field in `Option<T>` in a `GlobalDefaults` struct
- Only merge fields that are `Some`
- More code, but semantically correct

**Option B: Simple overlay — global always applies first, local always wins**
- Load global → use as the starting `Defaults`
- Load local → any field that differs from `Defaults::default()` wins
- Simpler, but can't explicitly set a value back to the default in local config

**Recommendation: Option A** — it's more correct and avoids subtle bugs. We create a `GlobalDefaults` struct where every field is `Option<T>`, and merge into a `Defaults` only when `Some`.

Actually, a simpler approach: since the repo config already serializes ALL fields (even defaults), we can use **serde's built-in behavior**. The merge becomes:

1. Start with `Defaults::default()`
2. Deserialize global config on top (serde will overwrite fields present in JSON)
3. Deserialize local config on top (same thing)

Since we serialize all fields always, any field present in the file was explicitly set. This is the simplest approach and already matches how the config works.

**Final decision: simple layered deserialization.** Global JSON fields overwrite defaults, local JSON fields overwrite global.

## Feature 2: Setup Groups & Quick/Full Mode

### Current Behavior

`gg setup` asks ALL questions in a flat sequence:
1. Provider
2. Base branch
3. Branch username
4. Lint commands
5. Auto-lint before sync
6. Auto-add GG-IDs
7. Auto-rebase before sync
8. Sync behind threshold
9. Land auto-clean
10. Land wait timeout
11. Unstaged action
12. GitLab auto-merge (if GitLab)
13. Worktree base path

### New Behavior

#### Quick Mode: `gg setup` (no flags)

Asks only the essentials to get started:
1. Provider (GitHub/GitLab)
2. Base branch
3. Branch username

After completion, shows a hint:
```
Tip: Run `gg setup --all` to configure advanced options (sync, land, lint, etc.)
```

#### Full Mode: `gg setup --all`

Asks everything, organized in groups with section headers:

```
── General ──────────────────────────────
  Provider: [GitHub/GitLab]
  Base branch: [main]
  Branch username: [nacho]
  Auto-add GG-IDs: [yes/no]
  Unstaged action: [ask/add/stash/continue/abort]

── Sync ─────────────────────────────────
  Auto-rebase before sync: [yes/no]
  Behind threshold: [1]

── Land ─────────────────────────────────
  Auto-clean after landing: [yes/no]
  Wait timeout (minutes): [30]

── Lint ─────────────────────────────────
  Lint commands: [cargo fmt --check, ...]
  Auto-lint before sync: [yes/no]

── Worktrees ────────────────────────────
  Worktree base path: [../{repo}.{stack}]

── GitLab ───────────────────────────────  (only if provider=gitlab)
  Auto-merge on land: [yes/no]
```

Each group shows a styled header (using `console::style`). Within each group, the questions use the existing dialoguer prompts.

#### Showing Effective Values

When running `gg setup --all`, each prompt should show the **effective** value as the default (i.e., global merged with local). This way the user sees what's actually in effect and can override per-repo.

### New Options to Add

While reorganizing, add these new config fields:

1. **`sync_draft`** (`bool`, default: `false`) — Create new PRs/MRs as drafts by default during sync. Currently only available as `gg sync --draft` flag.

2. **`sync_update_descriptions`** (`bool`, default: `true`) — Update PR/MR descriptions on re-sync. Currently hardcoded, some users may want to disable to preserve manual edits.

These go in the **Sync** group.

### CLI Changes

```
gg setup          # Quick mode (essentials only)
gg setup --all    # Full mode (all options, grouped)
```

The `--all` flag is added to the `Setup` command variant in clap.

## Non-Goals

- No `gg setup --global` wizard (global config is manually edited)
- No migration tool for existing configs
- No environment variable overrides (keep it simple)
- No per-stack defaults in global config (global only has `defaults` + `worktree_base_path`)

## Backwards Compatibility

- Existing `gg setup` users: behavior changes (fewer questions by default), but `--all` recovers the full experience
- Existing configs: fully compatible, no schema changes needed for current fields
- New fields (`sync_draft`, `sync_update_descriptions`): default to current behavior, so existing users see no change

## File Changes Summary

- `crates/gg-core/src/config.rs` — Add global config loading, merge logic, new fields
- `crates/gg-core/src/commands/setup.rs` — Reorganize into groups, add `--all` flag support
- `crates/gg-core/src/commands/sync.rs` — Read `sync_draft` and `sync_update_descriptions` from config
- `crates/gg-cli/src/main.rs` — Add `--all` flag to Setup command, pass to setup::run()
