# CLAUDE.md

This file provides guidance for Claude Code and other AI assistants working on this repository.

## Project Overview

**git-gud (gg)** is a stacked-diffs CLI tool for GitHub and GitLab, inspired by Gerrit, Phabricator/Arcanist, and Graphite. It enables developers to break large changes into small, reviewable commits where each commit becomes its own PR/MR with proper dependency chains.

**Status**: Active development (v0.4.x).

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
    ├── land.rs      # Merge approved PRs/MRs
    ├── lint.rs      # Run lint commands per commit
    ├── ls.rs        # List stacks and commits
    ├── nav.rs       # Navigation (first/last/next/prev/mv)
    ├── rebase.rs    # Rebase onto base branch
    ├── reorder.rs   # Interactive reorder commits
    ├── setup.rs     # Config setup wizard
    ├── squash.rs    # Squash changes into current commit
    └── sync.rs      # Push branches and create/update PRs/MRs

tests/
└── integration_tests.rs  # Integration tests with temp repos

docs/
└── src/                     # mdBook documentation source

skills/
└── gg/                      # Unified agent skill (GitHub + GitLab)
    ├── SKILL.md             # Core instructions + agent rules
    ├── reference.md         # Command reference + JSON schemas
    └── examples/            # Workflow walkthroughs
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
- Contains: base branch, username, lint commands, stack configs, PR/MR mappings

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

## Pull Request Workflow

**Before merging any PR, you MUST:**

1. **Wait for CI to be fully green** - All checks must pass (format, clippy, test)
2. **Wait for Claude review feedback** - The `claude-review` check must complete
3. **Address any feedback** - If Claude leaves comments/suggestions, address them before merging
4. **Only then merge** - Use `gh pr merge <number> --squash --delete-branch`

This applies to all PRs, including those created by subagents.

## Documentation & Agent Skills

**All user-facing changes MUST update documentation and skills:**

### Documentation (`docs/`)

- New features, commands, or flags → update the relevant mdBook page in `docs/src/`
- Changed behavior or defaults → update both the command reference and any guides that mention it
- Removed features → remove from docs and add migration note if needed

Documentation lives in `docs/` (mdBook). Build locally with `mdbook build docs` or `mdbook serve docs`.

The docs should read as a guide, not a parameter dump. Explain *what you can do* and *why*, with real-world examples. The command reference pages can be more exhaustive, but even those should have practical examples.

### Agent skill (`skills/gg/`)

There is a single unified agent skill at `skills/gg/` that covers both GitHub and GitLab workflows. It is also published as a Claude Code plugin (manifest at `.claude-plugin/plugin.json`).

When making changes that affect features, commands, flags, or JSON output:

- Update `skills/gg/SKILL.md` — core workflow instructions and agent operating rules
- Update `skills/gg/reference.md` — command reference and JSON schemas
- Update examples in `skills/gg/examples/` if the workflow changes
- GitLab-specific features go in the dedicated "GitLab-specific" section of SKILL.md

The skill is provider-agnostic by default. GitLab-specific behavior (merge trains, `--auto-merge`, `glab` commands) is documented in dedicated sections rather than in a separate skill.

### README.md

Keep the README in sync with major feature changes, especially the feature list and usage examples.

## Code Style

- Follow standard Rust conventions
- Use `thiserror` for error definitions
- Prefer `git2` library over subprocess git calls when possible
- Use `glab` CLI for GitLab API operations (authentication handled externally)
- Keep functions focused and small
- Add doc comments for public APIs
