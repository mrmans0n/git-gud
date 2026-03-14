# Global Config & Setup Groups — Implementation Plan

**Goal:** Add global config file support and reorganize `gg setup` into grouped quick/full modes.
**Architecture:** Layered config resolution (hardcoded → global → local), setup refactored into group functions.
**Tech Stack:** Rust, serde, dialoguer, console, dirs crate (for XDG paths)

---

## Task 1: Global Config Loading & Merge

**Files:**
- Modify: `crates/gg-core/Cargo.toml` (add `dirs` dependency)
- Modify: `crates/gg-core/src/config.rs` (add global config functions + new fields)

### Step 1: Add `dirs` dependency

Add `dirs = "6"` to `[dependencies]` in `crates/gg-core/Cargo.toml`.

### Step 2: Add new config fields

Add to the `Defaults` struct in `config.rs`:

```rust
/// Create new PRs/MRs as drafts by default during sync (default: false)
#[serde(default)]
pub sync_draft: bool,

/// Update PR/MR descriptions on re-sync (default: true)
#[serde(default = "default_true")]
pub sync_update_descriptions: bool,
```

Add getter methods to `Config`:

```rust
pub fn get_sync_draft(&self) -> bool {
    self.defaults.sync_draft
}

pub fn get_sync_update_descriptions(&self) -> bool {
    self.defaults.sync_update_descriptions
}
```

Update `Default for Defaults` to include:
```rust
sync_draft: false,
sync_update_descriptions: true,
```

### Step 3: Add global config loading

Add to `config.rs`:

```rust
impl Config {
    /// Get the global config directory path
    pub fn global_config_dir() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("gg"))
    }

    /// Get the global config file path
    pub fn global_config_path() -> Option<PathBuf> {
        Self::global_config_dir().map(|d| d.join("config.json"))
    }

    /// Load global config from ~/.config/gg/config.json
    pub fn load_global() -> Result<Option<Config>> {
        let Some(path) = Self::global_config_path() else {
            return Ok(None);
        };
        if !path.exists() {
            return Ok(None);
        }
        let contents = fs::read_to_string(&path)?;
        let config: Config = serde_json::from_str(&contents)?;
        Ok(Some(config))
    }

    /// Load config with global defaults applied first, then repo-local on top.
    /// Resolution: hardcoded defaults → global config → repo-local config
    pub fn load_with_global(git_dir: &Path) -> Result<Self> {
        // Start with global (or default if no global exists)
        let mut config = match Self::load_global()? {
            Some(global) => global,
            None => Config::default(),
        };

        let local_path = Self::config_path(git_dir);
        if local_path.exists() {
            let lock = Self::acquire_lock(git_dir, false)?;
            let contents = fs::read_to_string(&local_path)?;
            let local: Config = serde_json::from_str(&contents)?;
            drop(lock);

            // Local overrides global — merge field by field
            config.merge_local(local);
        }

        Ok(config)
    }

    /// Merge a local config on top of self (global).
    /// Local stacks always replace. Local defaults override global defaults.
    /// worktree_base_path from local overrides global if set.
    fn merge_local(&mut self, local: Config) {
        // Stacks are always local
        self.stacks = local.stacks;

        // worktree_base_path: local wins if present
        if local.worktree_base_path.is_some() {
            self.worktree_base_path = local.worktree_base_path;
        }

        // Defaults: local wins field by field
        // Since we serialize all fields, the local JSON will have explicit values
        // for every field. So we just take the local defaults entirely.
        // The "layering" already happened: global was the starting point,
        // and local was written by the user with their repo-specific overrides.
        self.defaults = local.defaults;
    }
}
```

### Step 4: Update all call sites

Replace all `Config::load(git_dir)` → `Config::load_with_global(git_dir)` in all command files.

There are ~19 call sites across: absorb.rs, checkout.rs, clean.rs, land.rs, lint.rs, ls.rs, nav.rs, rebase.rs, reorder.rs, squash.rs, sync.rs.

**Exception:** `setup.rs` should still use `Config::load()` (not `load_with_global`) since setup writes the local config and needs to know what's local vs inherited. But it should also load the global separately to show effective values as defaults in prompts.

### Step 5: Write tests

Add tests for:
- `load_global()` returns None when no file exists
- `load_global()` reads valid config
- `load_with_global()` uses global defaults when no local exists
- `load_with_global()` local overrides global
- `load_with_global()` stacks come from local only
- New fields `sync_draft` and `sync_update_descriptions` default correctly
- New fields roundtrip through serialization

### Step 6: Wire new config fields into sync

In `crates/gg-core/src/commands/sync.rs`:
- Read `config.get_sync_draft()` and OR it with the `draft` CLI flag
- Read `config.get_sync_update_descriptions()` and AND it with the `update_descriptions` CLI flag

In `sync_stack()` function (around line 148), change the `draft` parameter usage:
```rust
let effective_draft = draft || config.get_sync_draft();
```

For `update_descriptions`, the CLI flag is `--update-descriptions` which defaults to true:
```rust
let effective_update_descriptions = update_descriptions && config.get_sync_update_descriptions();
```

---

## Task 2: Setup Reorganization (Quick + Full/Grouped)

**Files:**
- Modify: `crates/gg-core/src/commands/setup.rs` (reorganize into groups)
- Modify: `crates/gg-cli/src/main.rs` (add `--all` flag)

### Step 1: Add `--all` flag to CLI

In `crates/gg-cli/src/main.rs`, change `Setup` variant:

```rust
/// Set up git-gud config for this repository
#[command(name = "setup")]
Setup {
    /// Configure all options (grouped by category)
    #[arg(long)]
    all: bool,
},
```

Update the match arm:
```rust
Some(Commands::Setup { all }) => (gg_core::commands::setup::run(all), false),
```

### Step 2: Update setup::run() signature

Change `pub fn run() -> Result<()>` to `pub fn run(all: bool) -> Result<()>`.

### Step 3: Implement quick mode

When `all` is false, only ask:
1. Provider
2. Base branch
3. Branch username

Then save and show tip about `--all`.

### Step 4: Implement grouped full mode

When `all` is true, organize prompts into groups with styled headers:

```rust
fn print_group_header(name: &str) {
    println!();
    println!("{}", style(format!("── {} ──", name)).cyan().bold());
}
```

Groups:
1. **General**: provider, base, username, auto_add_gg_ids, unstaged_action
2. **Sync**: sync_auto_rebase, sync_behind_threshold, sync_draft, sync_update_descriptions
3. **Land**: land_auto_clean, land_wait_timeout_minutes
4. **Lint**: lint commands, sync_auto_lint
5. **Worktrees**: worktree_base_path
6. **GitLab** (conditional): gitlab.auto_merge_on_land

### Step 5: Show effective defaults from global config

In setup, load the global config to use as defaults for prompts:

```rust
let global = Config::load_global()?.unwrap_or_default();
// Use global.defaults as the starting point for prompt defaults
// when the local config doesn't have a value set
```

### Step 6: Add prompts for new fields

Add `prompt_sync_draft()` and `prompt_sync_update_descriptions()` functions following the existing pattern.

### Step 7: Write tests

- Test that `run(false)` only prompts essentials (mock context)
- Test that `run(true)` prompts all groups
- Integration test: setup with --all flag

---

## Task 3: Integration Tests

**Files:**
- Modify: `crates/gg-cli/tests/integration_tests.rs`

### Step 1: Test global config resolution

Create a test that:
1. Creates a temp dir for global config
2. Creates a temp git repo with local config
3. Verifies merged config has correct values

### Step 2: Test setup --all flag parsing

Test that `gg setup --all` is accepted by clap.

### Step 3: Test sync respects new config fields

Test that sync uses `sync_draft` from config when `--draft` flag is not passed.

---

## Execution Order

Tasks 1 and 2 have a dependency (Task 2 uses the global config loading from Task 1), so they should be executed sequentially as a single implementation unit.

**Single subagent:** Implement Task 1 → Task 2 → Task 3 in sequence.
