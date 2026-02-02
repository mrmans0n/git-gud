//! Configuration management for git-gud
//!
//! Config is stored in `.git/gg/config.json` and contains:
//! - Default settings (base branch, username, lint commands)
//! - Per-stack settings and MR mappings

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Default configuration values
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Defaults {
    /// Git hosting provider ("github" or "gitlab")
    /// Used for self-hosted instances where URL detection fails
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,

    /// Base branch name (default: auto-detect main/master/trunk)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,

    /// Username for branch naming (default: glab whoami)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_username: Option<String>,

    /// Lint commands to run per commit
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lint: Vec<String>,

    /// Automatically add GG-IDs to commits without prompting (default: true)
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub auto_add_gg_ids: bool,

    /// Timeout in minutes for `gg land --wait` (default: 30)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub land_wait_timeout_minutes: Option<u64>,

    /// Automatically clean up stack after landing all PRs/MRs (default: false)
    #[serde(default, skip_serializing_if = "is_false")]
    pub land_auto_clean: bool,
}

fn default_true() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

fn is_false(b: &bool) -> bool {
    !*b
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
}

/// Root configuration structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Default settings
    #[serde(default)]
    pub defaults: Defaults,

    /// Per-stack configurations
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub stacks: HashMap<String, StackConfig>,
}

impl Config {
    /// Load config from the given git directory
    pub fn load(git_dir: &Path) -> Result<Self> {
        let config_path = Self::config_path(git_dir);

        if !config_path.exists() {
            return Ok(Config::default());
        }

        let contents = fs::read_to_string(&config_path)?;
        let config: Config = serde_json::from_str(&contents)?;
        Ok(config)
    }

    /// Save config to the given git directory
    pub fn save(&self, git_dir: &Path) -> Result<()> {
        let config_path = Self::config_path(git_dir);

        // Ensure the gg directory exists
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&config_path, contents)?;
        Ok(())
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

    /// Get the land wait timeout in minutes (default: 30)
    pub fn get_land_wait_timeout_minutes(&self) -> u64 {
        self.defaults.land_wait_timeout_minutes.unwrap_or(30)
    }

    /// Get whether to auto-clean after landing all PRs/MRs (default: false)
    pub fn get_land_auto_clean(&self) -> bool {
        self.defaults.land_auto_clean
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
    fn test_provider_config_not_serialized_when_none() {
        let temp_dir = TempDir::new().unwrap();
        let git_dir = temp_dir.path();

        let config = Config::default();
        config.save(git_dir).unwrap();

        // Read the raw JSON to verify provider is not included
        let contents = std::fs::read_to_string(Config::config_path(git_dir)).unwrap();
        assert!(
            !contents.contains("provider"),
            "provider should not be serialized when None"
        );
    }
}
