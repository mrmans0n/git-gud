//! Error types for git-gud

use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)] // Some variants reserved for future use
pub enum GgError {
    #[error("Network error: {0}")]
    NetworkError(String),

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

    #[error("Push failed for branch {branch}")]
    PushFailed {
        branch: String,
        hook_error: Option<String>,
        git_error: Option<String>,
    },

    #[error("Command '{0}' failed: {1}")]
    Command(String, String),

    #[error("Rebase conflict. Resolve conflicts and run `gg continue`, or `gg abort` to cancel.")]
    RebaseConflict,

    #[error("No rebase in progress")]
    NoRebaseInProgress,

    #[error(
        "cannot rewrite immutable commits (pass --force / --ignore-immutable to override):\n{0}"
    )]
    ImmutableTargets(String),

    #[error("Git error: {0}")]
    Git(#[from] git2::Error),

    #[error("Config error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("No operation record found with id '{0}'")]
    OperationRecordNotFound(String),

    #[error("Operation '{id}' is not undoable: {reason}")]
    OperationNotUndoable { id: String, reason: String },

    #[error("Cannot undo: ref '{ref_name}' has moved since the operation (expected {expected}, actually {actual}). Run `gg undo --list` to see a safe candidate.")]
    StaleUndo {
        ref_name: String,
        expected: String,
        actual: String,
    },

    #[error("Cannot locally undo '{kind}': it touched a remote.\n{hint}")]
    RemoteUndoUnsupported { kind: String, hint: String },

    #[error("{0}")]
    Other(String),

    #[error("A git operation is currently in progress.\n{0}\nIf no other process is running, remove the stale lock:\n  rm {1}")]
    GitOperationInProgress(String, String),
}

pub type Result<T> = std::result::Result<T, GgError>;

/// Check if an error message indicates a network problem rather than an auth failure.
///
/// This is used to distinguish between actual authentication failures (e.g., token expired,
/// not logged in) and transient network issues (e.g., DNS failures, connection timeouts).
pub fn is_network_error(output: &str) -> bool {
    let lower = output.to_lowercase();
    let network_patterns = [
        "could not resolve host",
        "connection refused",
        "connection timed out",
        "connection reset",
        "network is unreachable",
        "no route to host",
        "tls handshake timeout",
        // SSL patterns - specific enough to avoid false positives
        "ssl connection",
        "ssl handshake",
        "ssl certificate",
        "ssl error",
        "ssl_connect",
        // Timeout patterns - specific variants
        "timed out",
        "request timeout",
        // DNS patterns - specific enough to avoid false positives
        "dns resolution",
        "dns lookup",
        "dns error",
        "getaddrinfo",
        "econnrefused",
        "econnreset",
        "etimedout",
        "enetunreach",
        "ehostunreach",
        "name or service not known",
        "temporary failure in name resolution",
        "failed to connect",
        "unable to access",
    ];
    network_patterns
        .iter()
        .any(|pattern| lower.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_network_error_detects_dns_errors() {
        assert!(is_network_error("Could not resolve host: github.com"));
        assert!(is_network_error("DNS resolution failed"));
        assert!(is_network_error("DNS lookup failed for api.github.com"));
        assert!(is_network_error("DNS error: NXDOMAIN"));
        assert!(is_network_error(
            "getaddrinfo failed: Name or service not known"
        ));
        assert!(is_network_error("temporary failure in name resolution"));
    }

    #[test]
    fn test_is_network_error_detects_connection_errors() {
        assert!(is_network_error("connection timed out"));
        assert!(is_network_error("Connection refused (os error 111)"));
        assert!(is_network_error("Connection reset by peer"));
        assert!(is_network_error("failed to connect to api.github.com"));
    }

    #[test]
    fn test_is_network_error_detects_network_unreachable() {
        assert!(is_network_error("network is unreachable"));
        assert!(is_network_error("no route to host"));
        assert!(is_network_error("ENETUNREACH"));
        assert!(is_network_error("EHOSTUNREACH"));
    }

    #[test]
    fn test_is_network_error_detects_tls_errors() {
        assert!(is_network_error("TLS handshake timeout"));
        assert!(is_network_error("SSL connection failed"));
        assert!(is_network_error("SSL handshake failed"));
        assert!(is_network_error("SSL certificate verify failed"));
        assert!(is_network_error("SSL error: certificate expired"));
        assert!(is_network_error("curl: (35) SSL_connect returned"));
    }

    #[test]
    fn test_is_network_error_detects_timeout_errors() {
        assert!(is_network_error("connection timed out"));
        assert!(is_network_error("request timeout"));
        assert!(is_network_error("Operation timed out"));
        assert!(is_network_error("ETIMEDOUT"));
    }

    #[test]
    fn test_is_network_error_detects_errno_codes() {
        assert!(is_network_error("ECONNREFUSED"));
        assert!(is_network_error("ECONNRESET"));
    }

    #[test]
    fn test_is_network_error_detects_git_access_errors() {
        assert!(is_network_error(
            "unable to access 'https://github.com/repo.git/'"
        ));
    }

    #[test]
    fn test_is_network_error_ignores_auth_errors() {
        assert!(!is_network_error("not logged in"));
        assert!(!is_network_error("authentication failed"));
        assert!(!is_network_error("token expired"));
        assert!(!is_network_error("bad credentials"));
        assert!(!is_network_error(
            "You are not logged into any GitHub hosts"
        ));
    }

    #[test]
    fn test_is_network_error_ignores_empty_string() {
        assert!(!is_network_error(""));
    }

    #[test]
    fn test_is_network_error_case_insensitive() {
        assert!(is_network_error("COULD NOT RESOLVE HOST"));
        assert!(is_network_error("Connection Timed Out"));
        assert!(is_network_error("DNS RESOLUTION FAILED"));
    }
}
