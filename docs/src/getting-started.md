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

- Git 2.x+
- GitHub repositories: [`gh`](https://cli.github.com/)
- GitLab repositories: [`glab`](https://gitlab.com/gitlab-org/cli)

Authenticate first:

```bash
# GitHub
gh auth login

# GitLab
glab auth login
```

## Quick start

```bash
# Create or switch to a stack
gg co my-feature

# Make commits with normal git
git add . && git commit -m "Add data model"
git add . && git commit -m "Add API endpoint"

# Inspect stack
gg ls

# Push branches + create/update PRs/MRs
gg sync --draft

# Land approved changes
gg land --all

# Cleanup merged stack
gg clean
```
