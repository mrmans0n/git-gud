//! gg-core: Core library for git-gud (gg) stacked-diffs CLI tool.
//!
//! This crate contains all the business logic for git-gud operations,
//! separated from the CLI and MCP server entry points.

pub mod commands;
pub mod config;
pub mod context;
pub mod error;
pub mod gh;
pub mod git;
pub mod glab;
pub mod output;
pub mod provider;
pub mod stack;
pub mod template;
