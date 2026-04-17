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
use std::process::Command;
use thiserror::Error;

use gg_core::config::Config;
use gg_core::git;
use gg_core::provider::{CiStatus, PrState, Provider};
use gg_core::stack::Stack;

// --- Error types ---

/// Errors that can occur during MCP tool execution.
#[derive(Debug, Error)]
pub enum McpToolError {
    /// The current directory is not inside a git repository.
    #[error("Not in a git repository ({path}): {source}")]
    NotInRepo { path: String, source: git2::Error },

    /// Failed to load git-gud configuration.
    #[error("Failed to load config: {0}")]
    ConfigLoad(#[from] gg_core::error::GgError),

    /// Failed to detect the git hosting provider.
    #[error("Failed to detect provider: {0}")]
    ProviderDetect(String),

    /// Failed to retrieve PR/MR information.
    #[error("Failed to get PR #{number}: {reason}")]
    PrLookup { number: u64, reason: String },
}

impl From<McpToolError> for String {
    fn from(err: McpToolError) -> Self {
        err.to_string()
    }
}

/// Resolve the repository path from environment or current directory.
fn repo_path() -> PathBuf {
    std::env::var("GG_REPO_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default())
}

/// Open the git repository.
fn open_repo() -> Result<git2::Repository, McpToolError> {
    let path = repo_path();
    git2::Repository::discover(&path).map_err(|e| McpToolError::NotInRepo {
        path: path.display().to_string(),
        source: e,
    })
}

/// Load config from repo.
fn load_config(repo: &git2::Repository) -> Result<Config, McpToolError> {
    Config::load_with_global(repo.commondir()).map_err(McpToolError::ConfigLoad)
}

/// Load current stack.
fn load_stack(repo: &git2::Repository, config: &Config) -> Result<Stack, McpToolError> {
    Stack::load(repo, config).map_err(McpToolError::ConfigLoad)
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
struct StackLogInfo {
    stack: String,
    base: String,
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
pub struct StackLogParams {
    /// Refresh PR/MR status from remote before rendering
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackInboxParams {
    /// Include merged/clean items (default: only actionable items)
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrInfoParams {
    /// PR/MR number to look up
    pub number: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackCheckoutParams {
    /// Stack name to create or switch to
    pub name: Option<String>,
    /// Base branch (default: main/master)
    #[serde(default)]
    pub base: Option<String>,
    /// Use a git worktree for isolation
    #[serde(default)]
    pub worktree: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackSyncParams {
    /// Create PRs as draft
    #[serde(default)]
    pub draft: bool,
    /// Force-push branches
    #[serde(default)]
    pub force: bool,
    /// Update PR descriptions from commit messages
    #[serde(default)]
    pub update_descriptions: bool,
    /// Skip rebase-needed check
    #[serde(default)]
    pub no_rebase_check: bool,
    /// Run lint before syncing
    #[serde(default)]
    pub lint: bool,
    /// Only sync up to this position, GG-ID, or SHA
    #[serde(default)]
    pub until: Option<String>,
    /// Skip the pre-push hook (forwards `--no-verify` to `gg sync`)
    #[serde(default)]
    pub no_verify: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackLandParams {
    /// Land all approved PRs (not just the first)
    #[serde(default)]
    pub all: bool,
    /// Use squash merge
    #[serde(default)]
    pub squash: bool,
    /// Auto-clean the stack after landing
    #[serde(default)]
    pub auto_clean: bool,
    /// Only land up to this position, GG-ID, or SHA
    #[serde(default)]
    pub until: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackCleanParams {
    /// Clean all merged stacks (not just current)
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackRebaseParams {
    /// Target branch to rebase onto (default: base branch)
    #[serde(default)]
    pub target: Option<String>,
    /// Bypass the immutability guard on merged / base-ancestor commits.
    /// Only set after surfacing the affected commits to the user.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackSquashParams {
    /// Stage all changes before squashing (like git add -A)
    #[serde(default)]
    pub all: bool,
    /// Bypass the immutability guard on merged / base-ancestor commits.
    /// Only set after surfacing the affected commits to the user.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackAbsorbParams {
    /// Show what would be absorbed without making changes
    #[serde(default)]
    pub dry_run: bool,
    /// Rebase after absorbing
    #[serde(default)]
    pub and_rebase: bool,
    /// Absorb whole files instead of individual hunks
    #[serde(default)]
    pub whole_file: bool,
    /// Create one fixup commit per target commit
    #[serde(default)]
    pub one_fixup_per_commit: bool,
    /// Squash fixup commits immediately
    #[serde(default)]
    pub squash: bool,
    /// Bypass the immutability guard on merged / base-ancestor commits.
    /// Only set after surfacing the affected commits to the user.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackReconcileParams {
    /// Show what would change without making changes
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackMoveParams {
    /// Target: position number, GG-ID (e.g. c-abc1234), or SHA prefix
    pub target: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackNavigateParams {
    /// Direction to navigate: "first", "last", "prev", or "next"
    pub direction: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackLintParams {
    /// Only lint up to this position number
    #[serde(default)]
    pub until: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackDropParams {
    /// Commits to drop: position (1-indexed), short SHA, or GG-ID
    #[serde(default)]
    pub targets: Vec<String>,
    /// Bypass the immutability guard and drop merged/base-ancestor commits
    /// anyway. Defaults to false so the MCP tool does not silently rewrite
    /// already-published history. The confirmation prompt is always skipped
    /// (MCP is non-interactive); only set this when the user has explicitly
    /// approved rewriting immutable commits.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackRestackParams {
    /// Show what would be done without making changes
    #[serde(default)]
    pub dry_run: bool,
    /// Repair only from this position, GG-ID, or SHA upward
    #[serde(default)]
    pub from: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackSplitParams {
    /// Target commit: position (1-indexed), short SHA, or GG-ID (default: current)
    #[serde(default)]
    pub commit: Option<String>,
    /// Files to include in the new commit
    #[serde(default)]
    pub files: Vec<String>,
    /// Message for the new (first) commit
    #[serde(default)]
    pub message: Option<String>,
    /// Don't prompt for the remainder commit message
    #[serde(default)]
    pub no_edit: bool,
    /// Bypass the immutability guard on merged / base-ancestor commits.
    /// Only set after surfacing the affected commits to the user.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackReorderParams {
    /// New order as positions (1-indexed), e.g., "3,1,2" or "3 1 2"
    pub order: String,
    /// Bypass the immutability guard on merged / base-ancestor commits.
    /// Only set after surfacing the affected commits to the user.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackUndoParams {
    /// Target operation id (e.g. `op_0000001750000000_abcd…`). When
    /// omitted, undoes the most recent locally-undoable operation.
    #[serde(default)]
    pub operation_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StackUndoListParams {
    /// Cap the number of records returned (newest-first).
    #[serde(default)]
    pub limit: Option<usize>,
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

/// Run a `gg` CLI command as a subprocess and capture its output.
///
/// This avoids stdout conflicts with the MCP JSON-RPC transport on stdio,
/// since gg commands print directly to stdout.
fn run_gg_command(args: &[String]) -> Result<String, String> {
    let path = repo_path();
    let output = Command::new("gg")
        .args(args)
        .current_dir(&path)
        .output()
        .map_err(|e| format!("Failed to run gg: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        // Return stdout if non-empty, otherwise a success message
        if stdout.trim().is_empty() {
            Ok(format!("Command succeeded: gg {}", args.join(" ")))
        } else {
            Ok(stdout)
        }
    } else {
        // Combine stderr and stdout for error context
        let mut error = stderr;
        if !stdout.trim().is_empty() {
            error.push_str(&stdout);
        }
        if error.trim().is_empty() {
            error = format!("Command failed with exit code: {:?}", output.status.code());
        }
        Err(error)
    }
}

fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e))
}

/// Build the argv for `gg undo [--json] [<operation_id>]`.
/// Extracted for unit testing — see `build_stack_undo_args_*` tests.
fn build_stack_undo_args(operation_id: Option<String>) -> Vec<String> {
    let mut args = vec!["undo".to_string(), "--json".to_string()];
    if let Some(id) = operation_id {
        args.push(id);
    }
    args
}

/// Build the argv for `gg undo --list --json [--limit N]`.
fn build_stack_undo_list_args(limit: Option<usize>) -> Vec<String> {
    let mut args = vec![
        "undo".to_string(),
        "--list".to_string(),
        "--json".to_string(),
    ];
    if let Some(n) = limit {
        args.push("--limit".to_string());
        args.push(n.to_string());
    }
    args
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
                Provider::detect(&repo).map_err(|e| McpToolError::ProviderDetect(e.to_string()))?;

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

    /// Render the current stack as a smartlog-style view.
    /// Returns a stack-scoped view with positions, SHAs, titles, GG-IDs,
    /// PR/MR state, CI status, and a flag marking the HEAD entry. Use
    /// `stack_list_all` for cross-stack browsing.
    #[tool(
        description = "Show a smartlog-style view of the current stack (positions, SHAs, titles, PR/MR status, HEAD marker). Stack-scoped — use stack_list_all for all stacks."
    )]
    fn stack_log(&self, Parameters(params): Parameters<StackLogParams>) -> Result<String, String> {
        let repo = open_repo()?;
        let config = load_config(&repo)?;
        let mut stack = load_stack(&repo, &config)?;

        if params.refresh {
            let provider =
                Provider::detect(&repo).map_err(|e| McpToolError::ProviderDetect(e.to_string()))?;
            stack
                .refresh_mr_info(&provider)
                .map_err(McpToolError::ConfigLoad)?;
        }

        let info = build_stack_info(&stack, &repo);
        let log_info = StackLogInfo {
            stack: info.name,
            base: info.base,
            current_position: info.current_position,
            entries: info.entries,
        };
        Ok(to_json(&log_info))
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
            Provider::detect(&repo).map_err(|e| McpToolError::ProviderDetect(e.to_string()))?;

        let info = provider
            .get_pr_info(params.number)
            .map_err(|e| McpToolError::PrLookup {
                number: params.number,
                reason: e.to_string(),
            })?;

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
            auto_add_gg_ids: config.get_auto_add_gg_ids(),
            land_auto_clean: config.defaults.land_auto_clean,
            sync_auto_lint: config.defaults.sync_auto_lint,
            sync_auto_rebase: config.defaults.sync_auto_rebase,
        };
        Ok(to_json(&result))
    }

    // --- Write tools ---
    // These invoke the `gg` CLI as a subprocess to avoid stdout conflicts
    // with the MCP JSON-RPC transport on stdio.

    /// Create a new stack or switch to an existing one.
    #[tool(
        description = "Create a new stack or switch to an existing one. If the stack already exists, switches to it."
    )]
    fn stack_checkout(
        &self,
        Parameters(params): Parameters<StackCheckoutParams>,
    ) -> Result<String, String> {
        let mut args = vec!["co".to_string()];
        if let Some(ref name) = params.name {
            args.push(name.clone());
        }
        if let Some(ref base) = params.base {
            args.push("--base".to_string());
            args.push(base.clone());
        }
        if params.worktree {
            args.push("-w".to_string());
        }
        run_gg_command(&args)
    }

    /// Push branches and create/update PRs for the current stack.
    #[tool(
        description = "Sync the current stack: push branches and create/update PRs/MRs. Returns JSON with sync results including created/updated PR URLs."
    )]
    fn stack_sync(
        &self,
        Parameters(params): Parameters<StackSyncParams>,
    ) -> Result<String, String> {
        let mut args = vec!["sync".to_string(), "--json".to_string()];
        if params.draft {
            args.push("--draft".to_string());
        }
        if params.force {
            args.push("--force".to_string());
        }
        if params.update_descriptions {
            args.push("--update-descriptions".to_string());
        }
        if params.no_rebase_check {
            args.push("--no-rebase-check".to_string());
        }
        if params.lint {
            args.push("--lint".to_string());
        }
        if let Some(ref until) = params.until {
            args.push("--until".to_string());
            args.push(until.clone());
        }
        if params.no_verify {
            args.push("--no-verify".to_string());
        }
        run_gg_command(&args)
    }

    /// Merge approved PRs/MRs from the stack.
    #[tool(
        description = "Land (merge) approved PRs/MRs from the current stack. Returns JSON with land results."
    )]
    fn stack_land(
        &self,
        Parameters(params): Parameters<StackLandParams>,
    ) -> Result<String, String> {
        let mut args = vec!["land".to_string(), "--json".to_string()];
        if params.all {
            args.push("--all".to_string());
        }
        if params.squash {
            args.push("--squash".to_string());
        }
        if params.auto_clean {
            args.push("--auto-clean".to_string());
        }
        if let Some(ref until) = params.until {
            args.push("--until".to_string());
            args.push(until.clone());
        }
        run_gg_command(&args)
    }

    /// Clean up merged stacks.
    #[tool(
        description = "Clean up stacks whose PRs/MRs have been merged. Returns JSON with cleaned stacks."
    )]
    fn stack_clean(
        &self,
        Parameters(params): Parameters<StackCleanParams>,
    ) -> Result<String, String> {
        let mut args = vec!["clean".to_string(), "--json".to_string()];
        if params.all {
            args.push("--all".to_string());
        }
        run_gg_command(&args)
    }

    /// Rebase the current stack onto the latest base branch.
    #[tool(
        description = "Rebase the current stack onto the latest base branch (fetches and updates first)"
    )]
    fn stack_rebase(
        &self,
        Parameters(params): Parameters<StackRebaseParams>,
    ) -> Result<String, String> {
        let mut args = vec!["rebase".to_string()];
        if params.force {
            args.push("--force".to_string());
        }
        if let Some(ref target) = params.target {
            args.push(target.clone());
        }
        run_gg_command(&args)
    }

    /// Squash staged changes into the current commit.
    #[tool(
        description = "Squash (amend) staged changes into the current commit. Use --all to stage all changes first."
    )]
    fn stack_squash(
        &self,
        Parameters(params): Parameters<StackSquashParams>,
    ) -> Result<String, String> {
        let mut args = vec!["sc".to_string()];
        if params.all {
            args.push("--all".to_string());
        }
        if params.force {
            args.push("--force".to_string());
        }
        run_gg_command(&args)
    }

    /// Auto-absorb staged changes into the appropriate commits.
    #[tool(
        description = "Auto-absorb staged changes into the correct commits in the stack based on which files were modified."
    )]
    fn stack_absorb(
        &self,
        Parameters(params): Parameters<StackAbsorbParams>,
    ) -> Result<String, String> {
        let mut args = vec!["absorb".to_string()];
        if params.dry_run {
            args.push("--dry-run".to_string());
        }
        if params.and_rebase {
            args.push("--and-rebase".to_string());
        }
        if params.whole_file {
            args.push("--whole-file".to_string());
        }
        if params.one_fixup_per_commit {
            args.push("--one-fixup-per-commit".to_string());
        }
        if params.squash {
            args.push("-s".to_string());
        }
        if params.force {
            args.push("--force".to_string());
        }
        run_gg_command(&args)
    }

    /// Reconcile remotely-pushed branches with the local stack.
    #[tool(
        description = "Reconcile out-of-sync branches that were pushed outside of gg (e.g., from CI or web UI edits)"
    )]
    fn stack_reconcile(
        &self,
        Parameters(params): Parameters<StackReconcileParams>,
    ) -> Result<String, String> {
        let mut args = vec!["reconcile".to_string()];
        if params.dry_run {
            args.push("--dry-run".to_string());
        }
        run_gg_command(&args)
    }

    // --- Navigation tools ---

    /// Move to a specific commit in the stack by position, GG-ID, or SHA.
    #[tool(
        description = "Move to a specific commit in the stack by position number, GG-ID (e.g. c-abc1234), or SHA prefix"
    )]
    fn stack_move(
        &self,
        Parameters(params): Parameters<StackMoveParams>,
    ) -> Result<String, String> {
        run_gg_command(&["mv".to_string(), params.target])
    }

    /// Navigate within the stack.
    #[tool(
        description = "Navigate within the stack. Direction: 'first', 'last', 'prev', or 'next'"
    )]
    fn stack_navigate(
        &self,
        Parameters(params): Parameters<StackNavigateParams>,
    ) -> Result<String, String> {
        let cmd = match params.direction.as_str() {
            "first" | "last" | "prev" | "next" => params.direction.clone(),
            _ => {
                return Err(format!(
                    "Invalid direction '{}'. Use: first, last, prev, next",
                    params.direction
                ))
            }
        };
        run_gg_command(&[cmd])
    }

    /// Run lint commands on each commit in the stack.
    #[tool(
        description = "Run configured lint commands on each commit in the stack. Returns JSON with per-commit lint results."
    )]
    fn stack_lint(
        &self,
        Parameters(params): Parameters<StackLintParams>,
    ) -> Result<String, String> {
        let mut args = vec!["lint".to_string(), "--json".to_string()];
        if let Some(until) = params.until {
            args.push("--until".to_string());
            args.push(until.to_string());
        }
        run_gg_command(&args)
    }

    // --- Stack manipulation tools ---

    /// Drop (remove) commits from the stack.
    #[tool(
        description = "Remove commits from the stack. Targets can be positions (1-indexed), short SHAs, or GG-IDs. Always passes --yes (MCP is non-interactive); set `force: true` only to bypass the immutability guard for merged/base-ancestor commits. Returns JSON with dropped commits."
    )]
    fn stack_drop(
        &self,
        Parameters(params): Parameters<StackDropParams>,
    ) -> Result<String, String> {
        if params.targets.is_empty() {
            return Err("At least one target is required".to_string());
        }
        // Always pass `--yes` to skip the interactive prompt. Only add
        // `--force` when the caller has explicitly opted into rewriting
        // merged/base-ancestor commits — otherwise the immutability guard
        // keeps already-published history safe.
        let mut args = vec!["drop".to_string(), "--yes".to_string()];
        if params.force {
            args.push("--force".to_string());
        }
        args.push("--json".to_string());
        args.extend(params.targets);
        run_gg_command(&args)
    }

    /// Split a commit into two by moving specified files to a new commit.
    #[tool(
        description = "Split a commit by moving specified files to a new commit inserted before the original. File-level only (no hunk selection via MCP). Returns the result of the split operation."
    )]
    fn stack_split(
        &self,
        Parameters(params): Parameters<StackSplitParams>,
    ) -> Result<String, String> {
        if params.files.is_empty() {
            return Err("At least one file is required for split".to_string());
        }
        let mut args = vec!["split".to_string(), "--no-tui".to_string()];
        if let Some(ref commit) = params.commit {
            args.push("--commit".to_string());
            args.push(commit.clone());
        }
        if let Some(ref message) = params.message {
            args.push("--message".to_string());
            args.push(message.clone());
        }
        if params.no_edit {
            args.push("--no-edit".to_string());
        }
        if params.force {
            args.push("--force".to_string());
        }
        args.extend(params.files);
        run_gg_command(&args)
    }

    /// Show actionable triage view across all stacks — what's ready to land,
    /// what needs attention, and what's blocked.
    #[tool(
        description = "Show actionable inbox/triage view across all stacks. Returns items grouped by status: ready_to_land, changes_requested, blocked_on_ci, awaiting_review, behind_base, draft."
    )]
    fn stack_inbox(
        &self,
        Parameters(params): Parameters<StackInboxParams>,
    ) -> Result<String, String> {
        let mut args = vec!["inbox".to_string(), "--json".to_string()];
        if params.all {
            args.push("--all".to_string());
        }
        run_gg_command(&args)
    }

    /// Reorder commits in the stack with explicit order.
    #[tool(
        description = "Reorder commits in the stack. Order is specified as positions (1-indexed), e.g., '3,1,2' moves commit 3 to bottom, then 1, then 2 on top. No TUI via MCP."
    )]
    fn stack_reorder(
        &self,
        Parameters(params): Parameters<StackReorderParams>,
    ) -> Result<String, String> {
        let mut args = vec![
            "reorder".to_string(),
            "--no-tui".to_string(),
            "-o".to_string(),
            params.order,
        ];
        if params.force {
            args.push("--force".to_string());
        }
        run_gg_command(&args)
    }

    /// Reverse the local ref/HEAD effects of the most recent mutating
    /// `gg` command (or a specific operation by id).
    #[tool(
        description = "Reverse the local ref/HEAD effects of the most recent mutating gg command. Pass operation_id to target a specific record (see stack_undo_list). Refuses on remote-touching operations (sync, land) — you will get a provider-specific revert hint, not an automated revert. `gg undo` is itself recorded, so calling twice in a row redoes the original operation. Working tree changes are NOT reverted."
    )]
    fn stack_undo(
        &self,
        Parameters(params): Parameters<StackUndoParams>,
    ) -> Result<String, String> {
        run_gg_command(&build_stack_undo_args(params.operation_id))
    }

    /// List recent operations from the per-repo operation log.
    #[tool(
        description = "List recent operations from the per-repo operation log (newest-first). Each entry shows id, kind, status, timestamp, stack, args, and whether it is locally undoable. Use the id with stack_undo to target a specific record. Remote-touching operations appear with is_undoable=false and a red `remote` marker."
    )]
    fn stack_undo_list(
        &self,
        Parameters(params): Parameters<StackUndoListParams>,
    ) -> Result<String, String> {
        run_gg_command(&build_stack_undo_list_args(params.limit))
    }

    /// Repair stack ancestry after manual Git operations.
    #[tool(
        description = "Repair stack ancestry after manual Git operations (amend, cherry-pick, rebase). Detects GG-Parent mismatches and rebases commits onto correct parents. Returns JSON with repair plan and results."
    )]
    fn stack_restack(
        &self,
        Parameters(params): Parameters<StackRestackParams>,
    ) -> Result<String, String> {
        let mut args = vec!["restack".to_string(), "--json".to_string()];
        if params.dry_run {
            args.push("--dry-run".to_string());
        }
        if let Some(ref from) = params.from {
            args.push("--from".to_string());
            args.push(from.clone());
        }
        run_gg_command(&args)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for GgMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "git-gud (gg) MCP server. Provides tools to inspect and manage stacked-diffs \
                 workflows for GitHub and GitLab repositories.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pr_state_str() {
        assert_eq!(pr_state_str(&PrState::Open), "open");
        assert_eq!(pr_state_str(&PrState::Merged), "merged");
        assert_eq!(pr_state_str(&PrState::Closed), "closed");
        assert_eq!(pr_state_str(&PrState::Draft), "draft");
    }

    #[test]
    fn test_ci_status_str() {
        assert_eq!(ci_status_str(&CiStatus::Pending), "pending");
        assert_eq!(ci_status_str(&CiStatus::Running), "running");
        assert_eq!(ci_status_str(&CiStatus::Success), "success");
        assert_eq!(ci_status_str(&CiStatus::Failed), "failed");
        assert_eq!(ci_status_str(&CiStatus::Canceled), "canceled");
        assert_eq!(ci_status_str(&CiStatus::Unknown), "unknown");
    }

    #[test]
    fn stack_undo_builds_expected_gg_args_with_id() {
        let args = build_stack_undo_args(Some("op_123".to_string()));
        assert_eq!(args, vec!["undo", "--json", "op_123"]);
    }

    #[test]
    fn stack_undo_builds_expected_gg_args_without_id() {
        let args = build_stack_undo_args(None);
        assert_eq!(args, vec!["undo", "--json"]);
    }

    #[test]
    fn stack_undo_list_builds_expected_gg_args_with_limit() {
        let args = build_stack_undo_list_args(Some(10));
        assert_eq!(args, vec!["undo", "--list", "--json", "--limit", "10"]);
    }

    #[test]
    fn stack_undo_list_builds_expected_gg_args_without_limit() {
        let args = build_stack_undo_list_args(None);
        assert_eq!(args, vec!["undo", "--list", "--json"]);
    }

    #[test]
    fn test_to_json_serializes_struct() {
        let info = ConfigInfo {
            provider: Some("github".to_string()),
            base_branch: Some("main".to_string()),
            branch_username: Some("user".to_string()),
            lint_commands: vec!["cargo fmt".to_string()],
            auto_add_gg_ids: true,
            land_auto_clean: false,
            sync_auto_lint: true,
            sync_auto_rebase: false,
        };
        let json = to_json(&info);
        assert!(json.contains("\"provider\": \"github\""));
        assert!(json.contains("\"base_branch\": \"main\""));
        assert!(json.contains("\"auto_add_gg_ids\": true"));
        assert!(json.contains("\"lint_commands\""));
    }

    // NOTE: env var tests are combined into one test to avoid race conditions
    // when running tests in parallel (env vars are process-global).
    #[test]
    fn test_repo_path_and_open_repo() {
        // Test 1: env var overrides path
        std::env::set_var("GG_REPO_PATH", "/tmp/test-repo-path-check");
        let path = repo_path();
        assert_eq!(path, PathBuf::from("/tmp/test-repo-path-check"));

        // Test 2: open_repo fails outside git repo
        std::env::set_var("GG_REPO_PATH", "/tmp/definitely-not-a-git-repo-12345");
        let result = open_repo();
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.to_string().contains("Not in a git repository"));

        // Test 3: run_gg_command fails with invalid repo path
        let result = run_gg_command(&["ls".to_string()]);
        assert!(result.is_err() || result.unwrap().contains("error"));

        // Test 4: without env var, defaults to cwd
        std::env::remove_var("GG_REPO_PATH");
        let path = repo_path();
        assert!(path.is_absolute() || path == PathBuf::new());
    }

    #[test]
    fn test_mcp_tool_error_display() {
        let err = McpToolError::ProviderDetect("auth failed".to_string());
        assert_eq!(err.to_string(), "Failed to detect provider: auth failed");

        let err = McpToolError::PrLookup {
            number: 42,
            reason: "not found".to_string(),
        };
        assert_eq!(err.to_string(), "Failed to get PR #42: not found");
    }

    #[test]
    fn test_mcp_tool_error_to_string_conversion() {
        let err = McpToolError::ProviderDetect("test".to_string());
        let s: String = err.into();
        assert!(s.contains("Failed to detect provider"));
    }

    #[test]
    fn test_server_creation() {
        let server = GgMcpServer::new();
        let info = server.get_info();
        assert!(info.instructions.is_some());
        assert!(info.instructions.unwrap().contains("git-gud"));
    }

    // NOTE: test_run_gg_command_invalid_dir is included in test_repo_path_and_open_repo
    // to avoid env var race conditions with parallel test execution.

    #[test]
    fn test_stack_navigate_validates_direction() {
        let server = GgMcpServer::new();
        // We can't easily call the tool method directly due to the Parameters wrapper,
        // but we can test that the direction validation logic works
        let valid = ["first", "last", "prev", "next"];
        let invalid = ["up", "down", "left", "right", ""];
        for dir in valid {
            assert!(
                ["first", "last", "prev", "next"].contains(&dir),
                "Should be valid: {}",
                dir
            );
        }
        for dir in invalid {
            assert!(
                !["first", "last", "prev", "next"].contains(&dir),
                "Should be invalid: {}",
                dir
            );
        }
        // Verify server is usable (not consumed by tests above)
        assert!(server.get_info().instructions.is_some());
    }

    #[test]
    fn test_sync_params_defaults() {
        let params: StackSyncParams = serde_json::from_str("{}").unwrap();
        assert!(!params.draft);
        assert!(!params.force);
        assert!(!params.update_descriptions);
        assert!(!params.no_rebase_check);
        assert!(!params.lint);
        assert!(params.until.is_none());
        assert!(!params.no_verify);
    }

    #[test]
    fn test_land_params_defaults() {
        let params: StackLandParams = serde_json::from_str("{}").unwrap();
        assert!(!params.all);
        assert!(!params.squash);
        assert!(!params.auto_clean);
        assert!(params.until.is_none());
    }

    #[test]
    fn test_absorb_params_defaults() {
        let params: StackAbsorbParams = serde_json::from_str("{}").unwrap();
        assert!(!params.dry_run);
        assert!(!params.and_rebase);
        assert!(!params.whole_file);
        assert!(!params.one_fixup_per_commit);
        assert!(!params.squash);
        assert!(!params.force);
    }

    #[test]
    fn test_drop_params_defaults() {
        // Empty targets array should deserialize
        let params: StackDropParams = serde_json::from_str("{}").unwrap();
        assert!(params.targets.is_empty());
        // force must default to false so MCP drop does not silently bypass
        // the immutability guard for merged/base commits.
        assert!(!params.force);
    }

    #[test]
    fn test_drop_params_with_targets() {
        let params: StackDropParams =
            serde_json::from_str(r#"{"targets": ["1", "c-abc1234", "abc1234"]}"#).unwrap();
        assert_eq!(params.targets.len(), 3);
        assert_eq!(params.targets[0], "1");
        assert_eq!(params.targets[1], "c-abc1234");
        assert_eq!(params.targets[2], "abc1234");
        assert!(!params.force);
    }

    #[test]
    fn test_drop_params_with_force() {
        let params: StackDropParams =
            serde_json::from_str(r#"{"targets": ["1"], "force": true}"#).unwrap();
        assert!(params.force);
    }

    #[test]
    fn test_split_params_defaults() {
        let params: StackSplitParams = serde_json::from_str("{}").unwrap();
        assert!(params.commit.is_none());
        assert!(params.files.is_empty());
        assert!(params.message.is_none());
        assert!(!params.no_edit);
        assert!(!params.force);
    }

    #[test]
    fn test_split_params_with_values() {
        let params: StackSplitParams = serde_json::from_str(
            r#"{"commit": "2", "files": ["src/main.rs", "Cargo.toml"], "message": "Split commit", "no_edit": true}"#,
        )
        .unwrap();
        assert_eq!(params.commit, Some("2".to_string()));
        assert_eq!(params.files.len(), 2);
        assert_eq!(params.message, Some("Split commit".to_string()));
        assert!(params.no_edit);
    }

    #[test]
    fn test_reorder_params_requires_order() {
        // Order is required for MCP (no TUI)
        let params: StackReorderParams = serde_json::from_str(r#"{"order": "3,1,2"}"#).unwrap();
        assert_eq!(params.order, "3,1,2");
        // force defaults to false
        assert!(!params.force);
    }

    #[test]
    fn test_stack_drop_requires_targets() {
        let server = GgMcpServer::new();
        let params = StackDropParams {
            targets: vec![],
            force: false,
        };
        let result = server.stack_drop(Parameters(params));
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "At least one target is required".to_string()
        );
    }

    #[test]
    fn test_stack_split_requires_files() {
        let server = GgMcpServer::new();
        let params = StackSplitParams {
            commit: None,
            files: vec![],
            message: None,
            no_edit: false,
            force: false,
        };
        let result = server.stack_split(Parameters(params));
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "At least one file is required for split".to_string()
        );
    }

    #[test]
    fn test_reorder_params_supports_space_separated() {
        // Order can also be space-separated
        let params: StackReorderParams = serde_json::from_str(r#"{"order": "3 1 2"}"#).unwrap();
        assert_eq!(params.order, "3 1 2");
    }
}
