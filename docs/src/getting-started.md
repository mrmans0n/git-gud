# Getting Started

## Installation

### Homebrew (macOS/Linux)

```bash
brew install mrmans0n/tap/gg-stack
```

### crates.io

```bash
cargo install gg-stack
```

### From source

```bash
cargo install --path .
```

## Prerequisites

Before using git-gud, make sure you have:

- Git 2.x+
- [GitHub CLI (`gh`)](https://cli.github.com/) for GitHub repositories
- [GitLab CLI (`glab`)](https://gitlab.com/gitlab-org/cli) for GitLab repositories

## Authentication

Authenticate with your provider CLI first:

```bash
# GitHub
gh auth login

# GitLab
glab auth login
```

If authentication is missing, `gg sync` and `gg land` cannot create or merge PRs/MRs.

## Quick start: first stack in 2 minutes

```bash
# 1) Create a stack
gg co my-feature

# 2) Commit in small slices
git add . && git commit -m "Add data model"
git add . && git commit -m "Add API endpoint"
git add . && git commit -m "Add UI"

# 3) Inspect current stack
gg ls

# 4) Push branches and create PRs/MRs
gg sync --draft

# 5) Navigate to edit an earlier commit
gg mv 1
# ...make changes...
gg sc

# 6) Re-sync after changes
gg sync

# 7) Land approved changes
gg land --all

# 8) Clean merged stack
gg clean
```

For a full walkthrough with expected outputs and decision points, see [Your First Stack](./guides/your-first-stack.md).
