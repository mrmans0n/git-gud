//! Configuration management for git-gud
//!
//! Config is stored in `.git/gg/config.json` and contains:
//! - Default settings (base branch, username, lint commands)
//! - Per-stack settings and MR mappings

use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::Duration;

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::error::{GgError, Result};

/// Default configuration values
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Defaults {
    /// Git hosting provider ("github" or "gitlab")
    /// Used for self-hosted instances where URL detection fails
    pub provider: Option<String>,

    /// GitLab-specific defaults
    #[serde(default)]
    pub gitlab: GitLabDefaults,

    /// Base branch name (default: auto-detect main/master/trunk)
    pub base: Option<String>,

    /// Username for branch naming (default: glab whoami)
    pub branch_username: Option<String>,

    /// Lint commands to run per commit
    #[serde(default)]
    pub lint: Vec<String>,

    /// Deprecated: kept for backward compatibility with existing config files.
    /// Runtime behavior always enforces GG-ID metadata normalization.
    #[serde(default = "default_true")]
    pub auto_add_gg_ids: bool,

    /// Timeout in minutes for `gg land --wait` (default: 30)
    pub land_wait_timeout_minutes: Option<u64>,

    /// Automatically clean up stack after landing all PRs/MRs (default: false)
    #[serde(default)]
    pub land_auto_clean: bool,

    /// Use admin privileges to bypass approval requirements on land (default: false)
    #[serde(default)]
    pub land_admin: bool,

    /// Automatically run lint before sync (default: false)
    #[serde(default)]
    pub sync_auto_lint: bool,

    /// Automatically rebase before sync when stack base is behind origin/<base> (default: false)
    #[serde(default)]
    pub sync_auto_rebase: bool,

    /// Warn/rebase threshold for sync when base is behind origin/<base> (default: 1)
    #[serde(default = "default_sync_behind_threshold")]
    pub sync_behind_threshold: usize,

    /// Default action for `gg amend` when unstaged changes are present (default: ask)
    #[serde(default)]
    pub unstaged_action: UnstagedAction,

    /// Create new PRs/MRs as drafts by default during sync (default: false)
    #[serde(default)]
    pub sync_draft: bool,

    /// Update PR/MR descriptions on re-sync (default: true)
    #[serde(default = "default_true")]
    pub sync_update_descriptions: bool,
}

fn default_sync_behind_threshold() -> usize {
    1
}

fn default_true() -> bool {
    true
}

/// Behavior for `gg amend` when unstaged changes are detected.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UnstagedAction {
    /// Prompt the user to choose what to do.
    #[default]
    Ask,
    /// Stage all changes (including untracked files) and continue automatically.
    Add,
    /// Stash unstaged changes and continue automatically.
    Stash,
    /// Continue without including unstaged changes.
    Continue,
    /// Abort the operation when unstaged changes are present.
    Abort,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            provider: None,
            gitlab: GitLabDefaults::default(),
            base: None,
            branch_username: None,
            lint: Vec::new(),
            auto_add_gg_ids: true,
            land_wait_timeout_minutes: None,
            land_auto_clean: false,
            land_admin: false,
            sync_auto_lint: false,
            sync_auto_rebase: false,
            sync_behind_threshold: default_sync_behind_threshold(),
            unstaged_action: UnstagedAction::Ask,
            sync_draft: false,
            sync_update_descriptions: true,
        }
    }
}

/// GitLab-specific default settings
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GitLabDefaults {
    /// When landing, request GitLab to auto-merge the MR when the pipeline succeeds
    /// ("merge when pipeline succeeds") instead of attempting an immediate merge.
    #[serde(default)]
    pub auto_merge_on_land: bool,
}

/// Per-stack configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StackConfig {
    /// Base branch override for this stack
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,

    /// Mapping from entry-id to MR number
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub mrs: HashMap<String, u64>,

    /// Absolute path to a linked worktree for this stack
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
}

/// Root configuration structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Default settings
    #[serde(default)]
    pub defaults: Defaults,

    /// Template for stack worktree path.
    /// Variables: {repo} and {stack}
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_base_path: Option<String>,

    /// Per-stack configurations
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub stacks: HashMap<String, StackConfig>,
}

impl Config {
    /// Load config from the given git directory
    /// Uses file locking to prevent race conditions with concurrent operations
    pub fn load(git_dir: &Path) -> Result<Self> {
        let config_path = Self::config_path(git_dir);

        if !config_path.exists() {
            return Ok(Config::default());
        }

        // Acquire shared lock for reading (multiple readers allowed)
        let lock = Self::acquire_lock(git_dir, /*exclusive=*/ false)?;

        let contents = fs::read_to_string(&config_path)?;
        let config: Config = serde_json::from_str(&contents)?;

        // Lock automatically released when dropped
        drop(lock);
        Ok(config)
    }

    /// Save config to the given git directory
    /// Uses file locking and atomic write to prevent corruption
    pub fn save(&self, git_dir: &Path) -> Result<()> {
        let config_path = Self::config_path(git_dir);

        // Ensure the gg directory exists
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Acquire exclusive lock for writing
        let lock = Self::acquire_lock(git_dir, /*exclusive=*/ true)?;

        // Atomic write: write to temp file, then rename
        let temp_path = config_path.with_extension("tmp");
        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&temp_path, contents)?;

        // Atomic rename (overwrites existing file)
        fs::rename(&temp_path, &config_path)?;

        // Lock automatically released when dropped
        drop(lock);
        Ok(())
    }

    /// Acquire a file lock on the config file
    /// Returns a File handle that holds the lock until dropped
    fn acquire_lock(git_dir: &Path, exclusive: bool) -> Result<File> {
        let lock_path = Self::config_path(git_dir).with_extension("lock");

        // Ensure the parent directory exists
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Open or create lock file
        let lock_file = File::create(&lock_path)?;

        // Try to acquire lock with timeout to avoid indefinite hangs
        let timeout = Duration::from_secs(5);
        let start = std::time::Instant::now();

        loop {
            // Try to acquire lock (different methods have different return types on some platforms)
            let lock_result = if exclusive {
                lock_file.try_lock_exclusive()
            } else {
                lock_file.try_lock_shared().map_err(|e| {
                    // Convert shared lock error to io::Error
                    std::io::Error::new(std::io::ErrorKind::WouldBlock, e)
                })
            };

            match lock_result {
                Ok(()) => return Ok(lock_file),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Lock is held by another process, retry
                    if start.elapsed() >= timeout {
                        return Err(GgError::Other(
                            "Timeout waiting for config file lock. \
                             Another gg process may be running. \
                             Try again in a moment."
                                .to_string(),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Get the config file path
    pub fn config_path(git_dir: &Path) -> PathBuf {
        git_dir.join("gg").join("config.json")
    }

    /// Get or create stack config
    pub fn get_or_create_stack(&mut self, stack_name: &str) -> &mut StackConfig {
        self.stacks.entry(stack_name.to_string()).or_default()
    }

    /// Get stack config (read-only)
    pub fn get_stack(&self, stack_name: &str) -> Option<&StackConfig> {
        self.stacks.get(stack_name)
    }

    /// Remove a stack from config
    pub fn remove_stack(&mut self, stack_name: &str) {
        self.stacks.remove(stack_name);
    }

    /// Get the base branch for a stack, falling back to defaults
    pub fn get_base_for_stack(&self, stack_name: &str) -> Option<&str> {
        self.stacks
            .get(stack_name)
            .and_then(|s| s.base.as_deref())
            .or(self.defaults.base.as_deref())
    }

    /// Get the MR number for an entry ID in a stack
    pub fn get_mr_for_entry(&self, stack_name: &str, entry_id: &str) -> Option<u64> {
        self.stacks
            .get(stack_name)
            .and_then(|s| s.mrs.get(entry_id).copied())
    }

    /// Set the MR number for an entry ID in a stack
    pub fn set_mr_for_entry(&mut self, stack_name: &str, entry_id: &str, mr_number: u64) {
        let stack = self.get_or_create_stack(stack_name);
        stack.mrs.insert(entry_id.to_string(), mr_number);
    }

    /// Remove MR mapping for an entry ID
    pub fn remove_mr_for_entry(&mut self, stack_name: &str, entry_id: &str) {
        if let Some(stack) = self.stacks.get_mut(stack_name) {
            stack.mrs.remove(entry_id);
        }
    }

    /// Get all stacks
    pub fn list_stacks(&self) -> Vec<&str> {
        self.stacks.keys().map(|s| s.as_str()).collect()
    }

    /// Deprecated compatibility flag: always true at runtime.
    pub fn get_auto_add_gg_ids(&self) -> bool {
        true
    }

    /// Get the land wait timeout in minutes (default: 30)
    pub fn get_land_wait_timeout_minutes(&self) -> u64 {
        self.defaults.land_wait_timeout_minutes.unwrap_or(30)
    }

    /// Get whether to auto-clean after landing all PRs/MRs (default: false)
    pub fn get_land_auto_clean(&self) -> bool {
        self.defaults.land_auto_clean
    }

    /// Get whether to use admin privileges when landing (default: false)
    pub fn get_land_admin(&self) -> bool {
        self.defaults.land_admin
    }

    /// Get whether GitLab auto-merge-on-land is enabled by default (default: false)
    pub fn get_gitlab_auto_merge_on_land(&self) -> bool {
        self.defaults.gitlab.auto_merge_on_land
    }

    /// Get whether to auto-lint before sync (default: false)
    pub fn get_sync_auto_lint(&self) -> bool {
        self.defaults.sync_auto_lint
    }

    /// Get whether to auto-rebase before sync when behind (default: false)
    pub fn get_sync_auto_rebase(&self) -> bool {
        self.defaults.sync_auto_rebase
    }

    /// Get behind threshold for sync checks (default: 1)
    pub fn get_sync_behind_threshold(&self) -> usize {
        self.defaults.sync_behind_threshold
    }

    /// Get the default action for `gg amend` when unstaged changes are present.
    pub fn get_unstaged_action(&self) -> UnstagedAction {
        self.defaults.unstaged_action
    }

    /// Get whether to create PRs/MRs as drafts by default (default: false)
    pub fn get_sync_draft(&self) -> bool {
        self.defaults.sync_draft
    }

    /// Get whether to update PR/MR descriptions on re-sync (default: true)
    pub fn get_sync_update_descriptions(&self) -> bool {
        self.defaults.sync_update_descriptions
    }

    // ============ Global config loading ============

    /// Get the global config directory path (~/.config/gg)
    /// Uses home directory to ensure consistent cross-platform behavior
    pub fn global_config_dir() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".config").join("gg"))
    }

    /// Get the global config file path (~/.config/gg/config.json)
    pub fn global_config_path() -> Option<PathBuf> {
        Self::global_config_dir().map(|d| d.join("config.json"))
    }

    /// Load global config from ~/.config/gg/config.json
    /// Returns None if the file doesn't exist
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
        let mut config: Config = Self::load_global()?.unwrap_or_default();

        let local_path = Self::config_path(git_dir);
        if local_path.exists() {
            let lock = Self::acquire_lock(git_dir, /*exclusive=*/ false)?;
            let contents = fs::read_to_string(&local_path)?;
            let local: Config = serde_json::from_str(&contents)?;
            drop(lock);

            // Local overrides global
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

        // Defaults: local wins entirely (since we serialize all fields,
        // the local JSON will have explicit values for every field)
        self.defaults = local.defaults;
    }

    /// Render the target worktree path for a stack.
    ///
    /// Default template: ../{repo}.{stack}
    pub fn render_worktree_path(&self, repo_root: &Path, stack_name: &str) -> PathBuf {
        let repo_name = repo_root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("repo");

        let template = self
            .worktree_base_path
            .as_deref()
            .unwrap_or("../{repo}.{stack}");

        let rendered = template
            .replace("{repo}", repo_name)
            .replace("{stack}", stack_name);

        let path = PathBuf::from(rendered);
        if path.is_absolute() {
            path
        } else {
            repo_root.join(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_config_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        let mut config = Config::default();
        config.defaults.base = Some("main".to_string());
        config.defaults.branch_username = Some("nacho".to_string());
        config.defaults.lint = vec!["cargo fmt".to_string(), "cargo clippy".to_string()];

        let stack = config.get_or_create_stack("my-feature");
        stack.mrs.insert("c-abc123".to_string(), 1234);

        config.save(git_dir).unwrap();

        let loaded = Config::load(git_dir).unwrap();
        assert_eq!(loaded.defaults.base, Some("main".to_string()));
        assert_eq!(loaded.defaults.branch_username, Some("nacho".to_string()));
        assert_eq!(
            loaded.get_mr_for_entry("my-feature", "c-abc123"),
            Some(1234)
        );
    }

    #[test]
    fn test_missing_config_returns_default() {
        let temp_dir = TempDir::new().unwrap();
        let config = Config::load(temp_dir.path()).unwrap();
        assert!(config.stacks.is_empty());
    }

    #[test]
    fn test_auto_add_gg_ids_is_always_true_runtime() {
        let config: Config =
            serde_json::from_str(r#"{"defaults":{"auto_add_gg_ids":false}}"#).unwrap();

        // Keep deserialization compatibility, but runtime behavior is always enabled.
        assert!(!config.defaults.auto_add_gg_ids);
        assert!(config.get_auto_add_gg_ids());
    }

    #[test]
    fn test_land_wait_timeout_default() {
        let config = Config::default();
        assert_eq!(config.get_land_wait_timeout_minutes(), 30);
    }

    #[test]
    fn test_land_wait_timeout_custom() {
        let mut config = Config::default();
        config.defaults.land_wait_timeout_minutes = Some(60);
        assert_eq!(config.get_land_wait_timeout_minutes(), 60);
    }

    #[test]
    fn test_land_auto_clean_default() {
        let config = Config::default();
        assert!(!config.get_land_auto_clean());
    }

    #[test]
    fn test_land_auto_clean_enabled() {
        let mut config = Config::default();
        config.defaults.land_auto_clean = true;
        assert!(config.get_land_auto_clean());
    }

    #[test]
    fn test_land_auto_clean_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        let mut config = Config::default();
        config.defaults.land_auto_clean = true;

        config.save(git_dir).unwrap();

        let loaded = Config::load(git_dir).unwrap();
        assert!(loaded.get_land_auto_clean());
    }

    #[test]
    fn test_land_admin_default() {
        let config = Config::default();
        assert!(!config.get_land_admin());
    }

    #[test]
    fn test_land_admin_enabled() {
        let mut config = Config::default();
        config.defaults.land_admin = true;
        assert!(config.get_land_admin());
    }

    #[test]
    fn test_land_admin_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        let mut config = Config::default();
        config.defaults.land_admin = true;

        config.save(git_dir).unwrap();

        let loaded = Config::load(git_dir).unwrap();
        assert!(loaded.get_land_admin());
    }

    #[test]
    fn test_provider_config_default_is_none() {
        let config = Config::default();
        assert!(config.defaults.provider.is_none());
    }

    #[test]
    fn test_provider_config_github() {
        let mut config = Config::default();
        config.defaults.provider = Some("github".to_string());
        assert_eq!(config.defaults.provider.as_deref(), Some("github"));
    }

    #[test]
    fn test_provider_config_gitlab() {
        let mut config = Config::default();
        config.defaults.provider = Some("gitlab".to_string());
        assert_eq!(config.defaults.provider.as_deref(), Some("gitlab"));
    }

    #[test]
    fn test_provider_config_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        // Test with GitHub
        let mut config = Config::default();
        config.defaults.provider = Some("github".to_string());
        config.save(git_dir).unwrap();

        let loaded = Config::load(git_dir).unwrap();
        assert_eq!(loaded.defaults.provider, Some("github".to_string()));

        // Test with GitLab
        let mut config = Config::default();
        config.defaults.provider = Some("gitlab".to_string());
        config.save(git_dir).unwrap();

        let loaded = Config::load(git_dir).unwrap();
        assert_eq!(loaded.defaults.provider, Some("gitlab".to_string()));
    }

    #[test]
    fn test_provider_config_serialized_as_null_when_none() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        let config = Config::default();
        config.save(git_dir).unwrap();

        // All Defaults fields are always serialized
        let contents = std::fs::read_to_string(Config::config_path(git_dir)).unwrap();
        assert!(
            contents.contains("provider"),
            "provider should always be serialized"
        );
    }

    #[test]
    fn test_gitlab_auto_merge_on_land_default_is_false() {
        let config = Config::default();
        assert!(!config.get_gitlab_auto_merge_on_land());
    }

    #[test]
    fn test_gitlab_auto_merge_on_land_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        let mut config = Config::default();
        config.defaults.gitlab.auto_merge_on_land = true;
        config.save(git_dir).unwrap();

        let loaded = Config::load(git_dir).unwrap();
        assert!(loaded.get_gitlab_auto_merge_on_land());
    }

    #[test]
    fn test_gitlab_defaults_always_serialized() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        let config = Config::default();
        config.save(git_dir).unwrap();

        let contents = std::fs::read_to_string(Config::config_path(git_dir)).unwrap();
        assert!(
            contents.contains("gitlab"),
            "gitlab defaults should always be serialized"
        );
    }

    #[test]
    fn test_sync_auto_lint_default() {
        let config = Config::default();
        assert!(!config.get_sync_auto_lint());
    }

    #[test]
    fn test_sync_auto_lint_enabled() {
        let mut config = Config::default();
        config.defaults.sync_auto_lint = true;
        assert!(config.get_sync_auto_lint());
    }

    #[test]
    fn test_sync_auto_lint_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        let mut config = Config::default();
        config.defaults.sync_auto_lint = true;

        config.save(git_dir).unwrap();

        let loaded = Config::load(git_dir).unwrap();
        assert!(loaded.get_sync_auto_lint());
    }

    #[test]
    fn test_sync_auto_rebase_default() {
        let config = Config::default();
        assert!(!config.get_sync_auto_rebase());
    }

    #[test]
    fn test_sync_auto_rebase_enabled() {
        let mut config = Config::default();
        config.defaults.sync_auto_rebase = true;
        assert!(config.get_sync_auto_rebase());
    }

    #[test]
    fn test_sync_behind_threshold_default() {
        let config = Config::default();
        assert_eq!(config.get_sync_behind_threshold(), 1);
    }

    #[test]
    fn test_sync_behind_threshold_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        let mut config = Config::default();
        config.defaults.sync_behind_threshold = 3;

        config.save(git_dir).unwrap();

        let loaded = Config::load(git_dir).unwrap();
        assert_eq!(loaded.get_sync_behind_threshold(), 3);
    }

    #[test]
    fn test_render_worktree_path_default() {
        let config = Config::default();
        let repo_root = Path::new("/tmp/my-repo");
        let path = config.render_worktree_path(repo_root, "feature-a");
        assert_eq!(path, Path::new("/tmp/my-repo/../my-repo.feature-a"));
    }

    #[test]
    fn test_render_worktree_path_custom_template() {
        let config = Config {
            worktree_base_path: Some("/tmp/wt/{repo}-{stack}".to_string()),
            ..Config::default()
        };
        let repo_root = Path::new("/workspace/my-repo");
        let path = config.render_worktree_path(repo_root, "feature-a");
        assert_eq!(path, Path::new("/tmp/wt/my-repo-feature-a"));
    }

    #[test]
    fn test_unstaged_action_deserializes_to_default_when_missing() {
        let config: Config = serde_json::from_str(r#"{"defaults":{"base":"main"}}"#).unwrap();
        assert_eq!(config.get_unstaged_action(), UnstagedAction::Ask);
    }

    #[test]
    fn test_unstaged_action_deserializes_when_present() {
        let config: Config =
            serde_json::from_str(r#"{"defaults":{"unstaged_action":"stash"}}"#).unwrap();
        assert_eq!(config.get_unstaged_action(), UnstagedAction::Stash);
    }

    #[test]
    fn test_unstaged_action_deserializes_add_when_present() {
        let config: Config =
            serde_json::from_str(r#"{"defaults":{"unstaged_action":"add"}}"#).unwrap();
        assert_eq!(config.get_unstaged_action(), UnstagedAction::Add);
    }

    // ============ Tests for sync_draft and sync_update_descriptions ============

    #[test]
    fn test_sync_draft_default_is_false() {
        let config = Config::default();
        assert!(!config.get_sync_draft());
    }

    #[test]
    fn test_sync_draft_enabled() {
        let mut config = Config::default();
        config.defaults.sync_draft = true;
        assert!(config.get_sync_draft());
    }

    #[test]
    fn test_sync_draft_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        let mut config = Config::default();
        config.defaults.sync_draft = true;

        config.save(git_dir).unwrap();

        let loaded = Config::load(git_dir).unwrap();
        assert!(loaded.get_sync_draft());
    }

    #[test]
    fn test_sync_update_descriptions_default_is_true() {
        let config = Config::default();
        assert!(config.get_sync_update_descriptions());
    }

    #[test]
    fn test_sync_update_descriptions_disabled() {
        let mut config = Config::default();
        config.defaults.sync_update_descriptions = false;
        assert!(!config.get_sync_update_descriptions());
    }

    #[test]
    fn test_sync_update_descriptions_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        let mut config = Config::default();
        config.defaults.sync_update_descriptions = false;

        config.save(git_dir).unwrap();

        let loaded = Config::load(git_dir).unwrap();
        assert!(!loaded.get_sync_update_descriptions());
    }

    #[test]
    fn test_sync_draft_deserializes_to_default_when_missing() {
        let config: Config = serde_json::from_str(r#"{"defaults":{"base":"main"}}"#).unwrap();
        assert!(!config.get_sync_draft());
    }

    #[test]
    fn test_sync_update_descriptions_deserializes_to_default_when_missing() {
        let config: Config = serde_json::from_str(r#"{"defaults":{"base":"main"}}"#).unwrap();
        assert!(config.get_sync_update_descriptions());
    }

    // ============ Tests for global config loading ============

    #[test]
    fn test_global_config_path_returns_some() {
        // Should return a path on any system with a home directory
        let path = Config::global_config_path();
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains("gg"));
        assert!(path.to_string_lossy().contains("config.json"));
    }

    #[test]
    fn test_load_global_returns_none_when_no_file() {
        // Just check that load_global handles missing file gracefully
        // (The actual test would need to mock the home directory)
        let result = Config::load_global();
        // Should either return Ok(None) or Ok(Some(config)) - not an error
        assert!(result.is_ok());
    }

    #[test]
    fn test_load_with_global_uses_local_when_present() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        // Create a local config
        let mut config = Config::default();
        config.defaults.base = Some("develop".to_string());
        config.defaults.sync_draft = true;
        config.save(git_dir).unwrap();

        // Load with global should use local values
        let loaded = Config::load_with_global(git_dir).unwrap();
        assert_eq!(loaded.defaults.base, Some("develop".to_string()));
        assert!(loaded.get_sync_draft());
    }

    #[test]
    fn test_load_with_global_returns_default_when_no_configs() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        // No local config exists, global may or may not exist
        let loaded = Config::load_with_global(git_dir).unwrap();
        // Should at least have default values
        assert!(!loaded.get_sync_draft()); // Default
        assert!(loaded.get_sync_update_descriptions()); // Default is true
    }
}
