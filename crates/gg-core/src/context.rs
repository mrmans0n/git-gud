//! Execution context for commands.
//!
//! Provides a trait for abstracting interactive IO, allowing commands
//! to work in both CLI (interactive) and MCP (non-interactive) modes.

use crate::error::Result;

/// Execution context for commands.
///
/// CLI implements this with interactive prompts (dialoguer, skim).
/// MCP implements this with non-interactive defaults.
pub trait Context {
    /// Confirm an action (y/n). Non-interactive returns `default`.
    fn confirm(&self, message: &str, default: bool) -> Result<bool>;

    /// Select from a list of options. Non-interactive returns the first option (index 0).
    fn select(&self, message: &str, options: &[String]) -> Result<usize>;

    /// Report progress to the user.
    fn progress(&self, message: &str);

    /// Whether running in interactive mode.
    fn is_interactive(&self) -> bool;
}

/// Non-interactive context for MCP server and `--json` mode.
///
/// Always returns safe defaults without prompting the user.
pub struct NonInteractiveContext;

impl Context for NonInteractiveContext {
    fn confirm(&self, _message: &str, default: bool) -> Result<bool> {
        Ok(default)
    }

    fn select(&self, _message: &str, _options: &[String]) -> Result<usize> {
        Ok(0)
    }

    fn progress(&self, _message: &str) {
        // Silent in non-interactive mode
    }

    fn is_interactive(&self) -> bool {
        false
    }
}
