//! MCP tool definitions for git-gud.
//!
//! Read-only tools that expose stack and PR information.

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ServerHandler,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use gg_core::config::Config;
use gg_core::git;
use gg_core::provider::{CiStatus, PrState, Provider};
use gg_core::stack::Stack;

/// Resolve the repository path from environment or current directory.
fn repo_path() -> PathBuf {
    std::env::var("GG_REPO_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default())
}

/// Open the git repository.
fn open_repo() -> Result<git2::Repository, String> {
    let path = repo_path();
    git2::Repository::discover(&path)
        .map_err(|e| format!("Not in a git repository ({}): {}", path.display(), e))
}

/// Load config from repo.
fn load_config(repo: &git2::Repository) -> Result<Config, String> {
    Config::load(repo.commondir()).map_err(|e| format!("Failed to load config: {}", e))
}

/// Load current stack.
fn load_stack(repo: &git2::Repository, config: &Config) -> Result<Stack, String> {
    Stack::load(repo, config).map_err(|e| format!("Failed to load stack: {}", e))
}

// --- Response types ---

#[derive(Debug, Serialize)]
struct StackEntryInfo {
    position: usize,
    sha: String,
    title: String,
    gg_id: Option<String>,
    pr_number: Option<u64>,
    pr_state: Option<String>,
    approved: bool,
    ci_status: Option<String>,
    is_current: bool,
}

#[derive(Debug, Serialize)]
struct StackInfo {
    name: String,
    base: String,
    total_commits: usize,
    synced_commits: usize,
    current_position: Option<usize>,
    entries: Vec<StackEntryInfo>,
}

#[derive(Debug, Serialize)]
struct StackSummary {
    name: String,
    base: String,
    commit_count: usize,
    is_current: bool,
}

#[derive(Debug, Serialize)]
struct AllStacksInfo {
    current_stack: Option<String>,
    stacks: Vec<StackSummary>,
}

#[derive(Debug, Serialize)]
struct ConfigInfo {
    provider: Option<String>,
    base_branch: Option<String>,
    branch_username: Option<String>,
    lint_commands: Vec<String>,
    auto_add_gg_ids: bool,
    land_auto_clean: bool,
    sync_auto_lint: bool,
    sync_auto_rebase: bool,
}

// --- Tool parameters ---

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackListParams {
    /// Refresh PR/MR status from remote before listing
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrInfoParams {
    /// PR/MR number to look up
    pub number: u64,
}

// --- Helper functions ---

fn pr_state_str(state: &PrState) -> &'static str {
    match state {
        PrState::Open => "open",
        PrState::Merged => "merged",
        PrState::Closed => "closed",
        PrState::Draft => "draft",
    }
}

fn ci_status_str(status: &CiStatus) -> &'static str {
    match status {
        CiStatus::Pending => "pending",
        CiStatus::Running => "running",
        CiStatus::Success => "success",
        CiStatus::Failed => "failed",
        CiStatus::Canceled => "canceled",
        CiStatus::Unknown => "unknown",
    }
}

fn build_stack_info(stack: &Stack, repo: &git2::Repository) -> StackInfo {
    let head_oid = repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_commit().ok())
        .map(|c| c.id());

    let current_position = head_oid.and_then(|oid| {
        stack
            .entries
            .iter()
            .find(|e| e.oid == oid)
            .map(|e| e.position)
    });

    let synced = stack.entries.iter().filter(|e| e.is_synced()).count();

    StackInfo {
        name: stack.name.clone(),
        base: stack.base.clone(),
        total_commits: stack.entries.len(),
        synced_commits: synced,
        current_position,
        entries: stack
            .entries
            .iter()
            .map(|e| StackEntryInfo {
                position: e.position,
                sha: e.short_sha.clone(),
                title: e.title.clone(),
                gg_id: e.gg_id.clone(),
                pr_number: e.mr_number,
                pr_state: e.mr_state.as_ref().map(pr_state_str).map(String::from),
                approved: e.approved,
                ci_status: e.ci_status.as_ref().map(ci_status_str).map(String::from),
                is_current: head_oid.is_some_and(|oid| oid == e.oid),
            })
            .collect(),
    }
}

fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e))
}

// --- MCP Server ---

#[derive(Debug, Clone)]
pub struct GgMcpServer {
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl GgMcpServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router(router = tool_router)]
impl GgMcpServer {
    /// List the current stack with commit entries and PR/MR status.
    /// Returns positions, SHAs, titles, GG-IDs, PR numbers, CI status,
    /// and approval state for each commit in the stack.
    #[tool(description = "List the current stack with commit entries and PR/MR status")]
    fn stack_list(
        &self,
        Parameters(params): Parameters<StackListParams>,
    ) -> Result<String, String> {
        let repo = open_repo()?;
        let config = load_config(&repo)?;
        let mut stack = load_stack(&repo, &config)?;

        if params.refresh {
            let provider =
                Provider::detect(&repo).map_err(|e| format!("Failed to detect provider: {}", e))?;

            for entry in &mut stack.entries {
                if let Some(number) = entry.mr_number {
                    if let Ok(info) = provider.get_pr_info(number) {
                        entry.mr_state = Some(info.state);
                        entry.approved = info.approved;
                    }
                    if let Ok(ci) = provider.get_pr_ci_status(number) {
                        entry.ci_status = Some(ci);
                    }
                }
            }
        }

        let info = build_stack_info(&stack, &repo);
        Ok(to_json(&info))
    }

    /// List all stacks in the repository with summary information.
    #[tool(
        description = "List all stacks in the repository with summary information (name, base branch, commit count)"
    )]
    fn stack_list_all(&self) -> Result<String, String> {
        let repo = open_repo()?;
        let config = load_config(&repo)?;

        let current_branch = git::current_branch_name(&repo);
        let current_stack = current_branch
            .as_deref()
            .and_then(git::parse_stack_branch)
            .map(|(_, name)| name);

        let stacks = config.list_stacks();
        let mut summaries = Vec::new();

        for stack_name in &stacks {
            if let Some(stack_config) = config.get_stack(stack_name) {
                let base = stack_config
                    .base
                    .clone()
                    .unwrap_or_else(|| "main".to_string());
                let is_current = current_stack.as_deref() == Some(stack_name);

                let branch_username = config
                    .defaults
                    .branch_username
                    .as_deref()
                    .unwrap_or("unknown");
                let branch = format!("{}/{}", branch_username, stack_name);
                let commit_count = git::get_stack_commit_oids(&repo, &base, Some(&branch))
                    .map(|oids| oids.len())
                    .unwrap_or(0);

                summaries.push(StackSummary {
                    name: stack_name.to_string(),
                    base,
                    commit_count,
                    is_current,
                });
            }
        }

        let result = AllStacksInfo {
            current_stack,
            stacks: summaries,
        };
        Ok(to_json(&result))
    }

    /// Get a quick status summary of the current stack.
    #[tool(
        description = "Get current stack status: name, base branch, commit counts, position, and how far behind base"
    )]
    fn stack_status(&self) -> Result<String, String> {
        let repo = open_repo()?;
        let config = load_config(&repo)?;
        let stack = load_stack(&repo, &config)?;
        let info = build_stack_info(&stack, &repo);
        let upstream = format!("origin/{}", &stack.base);
        let behind = git::count_commits_behind(&repo, &stack.base, &upstream).unwrap_or(0);

        let status = serde_json::json!({
            "stack_name": info.name,
            "base_branch": info.base,
            "total_commits": info.total_commits,
            "synced_commits": info.synced_commits,
            "current_position": info.current_position,
            "behind_base": behind,
        });
        Ok(to_json(&status))
    }

    /// Get detailed information about a specific PR/MR by number.
    #[tool(
        description = "Get PR/MR details: state, title, URL, approval status, mergeability, and CI status"
    )]
    fn pr_info(&self, Parameters(params): Parameters<PrInfoParams>) -> Result<String, String> {
        let repo = open_repo()?;
        let provider =
            Provider::detect(&repo).map_err(|e| format!("Failed to detect provider: {}", e))?;

        let info = provider
            .get_pr_info(params.number)
            .map_err(|e| format!("Failed to get PR #{}: {}", params.number, e))?;

        let ci = provider
            .get_pr_ci_status(params.number)
            .ok()
            .map(|s| ci_status_str(&s).to_string());

        let mut result = serde_json::json!({
            "number": info.number,
            "title": info.title,
            "state": pr_state_str(&info.state),
            "url": info.url,
            "draft": info.draft,
            "approved": info.approved,
            "mergeable": info.mergeable,
        });
        if let Some(ci_status) = ci {
            result["ci_status"] = serde_json::Value::String(ci_status);
        }
        Ok(to_json(&result))
    }

    /// Show the current git-gud configuration for this repository.
    #[tool(
        description = "Show repository git-gud config: provider, base branch, lint commands, and all settings"
    )]
    fn config_show(&self) -> Result<String, String> {
        let repo = open_repo()?;
        let config = load_config(&repo)?;

        let result = ConfigInfo {
            provider: config.defaults.provider.clone(),
            base_branch: config.defaults.base.clone(),
            branch_username: config.defaults.branch_username.clone(),
            lint_commands: config.defaults.lint.clone(),
            auto_add_gg_ids: config.defaults.auto_add_gg_ids,
            land_auto_clean: config.defaults.land_auto_clean,
            sync_auto_lint: config.defaults.sync_auto_lint,
            sync_auto_rebase: config.defaults.sync_auto_rebase,
        };
        Ok(to_json(&result))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for GgMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "git-gud (gg) MCP server. Provides tools to inspect and manage stacked-diffs \
                 workflows for GitHub and GitLab repositories."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
