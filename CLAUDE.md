# CLAUDE.md

This file provides guidance for Claude Code and other AI assistants working on this repository.

## Project Overview

**git-gud (gg)** is a stacked-diffs CLI tool for GitLab, inspired by Gerrit, Phabricator/Arcanist, and Graphite. It enables developers to break large changes into small, reviewable commits where each commit becomes its own Merge Request (MR) with proper dependency chains.

**Status**: Early-stage (v0.1.0) - not battle-tested. Exercise caution.

## Code Quality Requirements

**All committed code MUST meet these requirements:**

1. **Tests are required** - All new functionality must have corresponding tests. Do not commit code without tests.

2. **Code must be formatted** - Run `cargo fmt --all` before finishing any work. CI will reject unformatted code.

3. **No clippy warnings** - Run `cargo clippy --all-targets --all-features -- -D warnings` and fix all warnings before committing.

### Pre-commit Checklist

```bash
cargo fmt --all                                              # Format code
cargo clippy --all-targets --all-features -- -D warnings     # Check for warnings
cargo test --all-features                                    # Run all tests
```

## Project Structure

```
src/
├── main.rs          # CLI entry point with clap subcommands
├── config.rs        # Config file management (.git/gg/config.json)
├── error.rs         # Error types using thiserror
├── git.rs           # Git operations via git2-rs
├── glab.rs          # GitLab CLI (glab) integration
├── stack.rs         # Stack data model and operations
└── commands/        # Individual command implementations
    ├── absorb.rs    # Auto-distribute changes to commits
    ├── checkout.rs  # Create/switch stacks (gg co)
    ├── clean.rs     # Remove merged stacks
    ├── completions.rs # Shell completions
    ├── land.rs      # Merge approved MRs
    ├── lint.rs      # Run lint commands per commit
    ├── ls.rs        # List stacks and commits
    ├── nav.rs       # Navigation (first/last/next/prev/mv)
    ├── rebase.rs    # Rebase onto base branch
    ├── reorder.rs   # Interactive reorder commits
    ├── setup.rs     # Config setup wizard
    ├── squash.rs    # Squash changes into current commit
    └── sync.rs      # Push branches and create/update MRs

tests/
└── integration_tests.rs  # Integration tests with temp repos

docs/
├── git-gud-design.md        # Architecture document
└── github-support-design.md # Future GitHub support planning
```

## Key Concepts

### GG-ID System
- Stable identifier format: `c-` + 7 UUID chars (e.g., `c-abc1234`)
- Stored as trailer in commit messages: `GG-ID: c-abc1234`
- Persists through rebases/reorders
- Used to track commit-to-MR mappings

### Branch Naming Convention
- Stack branch: `<username>/<stack-name>` (e.g., `nacho/my-feature`)
- Per-commit branches: `<username>/<stack-name>/<gg-id>` (e.g., `nacho/my-feature/c-abc1234`)

### Configuration
- Stored in `.git/gg/config.json` (per-repository)
- Contains: base branch, username, lint commands, stack configs, MR mappings

## Testing Patterns

### Integration Tests
Tests use temporary git repositories created with the `tempfile` crate. See `tests/integration_tests.rs` for examples.

Key test helpers:
- `create_test_repo()` - Creates isolated temp repo with git config
- `run_gg()` - Executes gg commands and captures output
- `run_git()` - Executes git commands

### Unit Tests
Unit tests are co-located in source files (e.g., `config.rs`, `git.rs`). Use `#[cfg(test)]` modules.

### Writing New Tests
```rust
#[test]
fn test_new_feature() {
    let repo = create_test_repo();

    // Setup: create commits, branches, etc.
    run_git(&repo, &["commit", "--allow-empty", "-m", "test commit"]);

    // Execute
    let result = run_gg(&repo, &["your-command"]);

    // Assert
    assert!(result.status.success());
    assert!(result.stdout.contains("expected output"));
}
```

## Dependencies

Key crates:
- `clap` - CLI argument parsing
- `git2` - libgit2 bindings for git operations
- `serde`/`serde_json` - Configuration serialization
- `tokio` - Async runtime for subprocess handling
- `indicatif`/`console`/`dialoguer` - Terminal UI
- `thiserror`/`anyhow` - Error handling

## Common Commands

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo test                     # Run all tests
cargo test <test_name>         # Run specific test
cargo run -- <args>            # Run gg with arguments
cargo install --path .         # Install locally
```

## Error Handling

Use the `GgError` enum in `src/error.rs` for domain-specific errors. All errors should have user-friendly messages with actionable guidance.

## Code Style

- Follow standard Rust conventions
- Use `thiserror` for error definitions
- Prefer `git2` library over subprocess git calls when possible
- Use `glab` CLI for GitLab API operations (authentication handled externally)
- Keep functions focused and small
- Add doc comments for public APIs
