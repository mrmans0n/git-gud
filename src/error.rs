//! Error types for git-gud

use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)] // Some variants reserved for future use
pub enum GgError {
    #[error("Invalid stack name: {0}")]
    InvalidStackName(String),

    #[error("Invalid branch username: {0}")]
    InvalidBranchUsername(String),

    #[error("Not in a git repository")]
    NotInRepo,

    #[error("Could not find base branch (tried main, master, trunk)")]
    NoBaseBranch,

    #[error("Not on a stack branch. Use `gg co <stack-name>` to create or switch to a stack.")]
    NotOnStack,

    #[error("Stack '{0}' not found")]
    StackNotFound(String),

    #[error("Dirty working directory. Please commit or stash your changes first.")]
    DirtyWorkingDirectory,

    #[error("Merge commits are not supported in stacks. Please rebase to a linear history.")]
    MergeCommitInStack,

    #[error("Commit {0} is missing a GG-ID trailer. Run `gg sync` to add one.")]
    MissingGgId(String),

    #[error("glab is not installed. Please install it from https://gitlab.com/gitlab-org/cli")]
    GlabNotInstalled,

    #[error("Not authenticated with GitLab. Run `glab auth login` first.")]
    GlabNotAuthenticated,

    #[error("glab command failed: {0}")]
    GlabError(String),

    #[error("Invalid PR number: {0}")]
    InvalidPrNumber(String),

    #[error("Command '{0}' failed: {1}")]
    Command(String, String),

    #[error("Rebase conflict. Resolve conflicts and run `gg continue`, or `gg abort` to cancel.")]
    RebaseConflict,

    #[error("No rebase in progress")]
    NoRebaseInProgress,

    #[error("Git error: {0}")]
    Git(#[from] git2::Error),

    #[error("Config error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, GgError>;
