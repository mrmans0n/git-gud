# git-gud (gg)

A stacked-diffs CLI tool for GitHub and GitLab, inspired by Gerrit, Phabricator/Arcanist, and Graphite.

> [!CAUTION]
> This tool has been vibe coded for myself and hasn't been battle tested yet. Do not use! You might bork your git repository!

## What are Stacked Diffs?

Stacked diffs allow you to break large changes into small, reviewable commits that build on each other. Each commit becomes its own Pull Request (GitHub) or Merge Request (GitLab), with proper dependency chains. This enables:

- **Faster reviews** - Small, focused changes are easier to review
- **Parallel work** - Start the next feature while waiting for review
- **Clean history** - Each commit is a logical unit of change

## Installation

### From source

```bash
cargo install --path .
```

### From crates.io (coming soon)

```bash
cargo install git-gud
```

## Prerequisites

- Git 2.x+
- For **GitHub** repositories: [gh](https://cli.github.com/) - GitHub CLI
- For **GitLab** repositories: [glab](https://gitlab.com/gitlab-org/cli) - GitLab CLI

git-gud automatically detects your remote provider and uses the appropriate CLI tool.

Authenticate with your provider before using git-gud:

```bash
# For GitHub
gh auth login

# For GitLab
glab auth login
```

## Quick Start

```bash
# Create a new stack
gg co my-feature

# Make changes and commit (normal git workflow)
git add . && git commit -m "Add data model"
git add . && git commit -m "Add API endpoint"
git add . && git commit -m "Add UI component"

# View your stack
gg ls

# Sync with remote (creates PRs/MRs)
gg sync --draft

# Navigate within the stack
gg first          # Go to first commit
gg next           # Go to next commit
gg prev           # Go to previous commit
gg last           # Return to stack head

# After review feedback, modify a commit
gg mv 1           # Move to commit 1
# make changes...
gg sc             # Squash changes into current commit

# Land approved PRs/MRs
gg land --all

# Clean up merged stacks
gg clean
```

## Commands

### Stack Management

| Command | Description |
|---------|-------------|
| `gg co <name>` | Create a new stack, switch to existing, or checkout from remote |
| `gg ls` | List current stack commits with PR/MR status |
| `gg ls --all` | List all stacks in the repository |
| `gg ls --remote` | List remote stacks not checked out locally |
| `gg clean` | Remove merged stacks and their remote branches |

### Syncing

| Command | Description |
|---------|-------------|
| `gg sync` | Push all commits and create/update PRs/MRs |
| `gg sync --draft` | Create new PRs/MRs as drafts |
| `gg sync --force` | Force push even if remote diverged |

### Navigation

| Command | Description |
|---------|-------------|
| `gg first` | Move to the first commit in the stack |
| `gg last` | Move to the last commit (stack head) |
| `gg prev` | Move to the previous commit |
| `gg next` | Move to the next commit |
| `gg mv <target>` | Move to a specific commit (by position, GG-ID, or SHA) |

### Editing

| Command | Description |
|---------|-------------|
| `gg sc` | Squash staged changes into current commit |
| `gg sc --all` | Squash all changes (staged + unstaged) |
| `gg reorder` | Reorder commits interactively |
| `gg absorb` | Auto-distribute changes to appropriate commits |

### Landing

| Command | Description |
|---------|-------------|
| `gg land` | Merge the first approved PR/MR (squash by default) |
| `gg land --all` | Merge all approved PRs/MRs in sequence |
| `gg land --no-squash` | Merge using merge commit instead of squash |
| `gg rebase` | Rebase stack onto updated base branch |

### Utilities

| Command | Description |
|---------|-------------|
| `gg setup` | Generate or update `.git/gg/config.json` interactively |
| `gg lint` | Run lint commands on each commit |
| `gg continue` | Continue after resolving conflicts |
| `gg abort` | Abort current operation |
| `gg completions <shell>` | Generate shell completions |

## Configuration

Configuration is stored in `.git/gg/config.json`. Run `gg setup` to generate it interactively:

```json
{
  "defaults": {
    "base": "main",
    "branch_username": "your-username",
    "lint": [
      "cargo fmt --check",
      "cargo clippy -- -D warnings"
    ]
  },
  "stacks": {
    "my-feature": {
      "base": "main",
      "mrs": {
        "c-abc1234": 123,
        "c-def5678": 124
      }
    }
  }
}
```

### Configuration Options

| Option | Description | Default |
|--------|-------------|---------|
| `defaults.base` | Default base branch | Auto-detect (main/master/trunk) |
| `defaults.branch_username` | Username for branch naming | Auto-detect via `gh`/`glab` |
| `defaults.lint` | Lint commands to run per commit | `[]` |

## How It Works

### Branch Naming

- **Stack branch**: `<username>/<stack-name>` (e.g., `nacho/my-feature`)
- **Per-commit branches**: `<username>/<stack-name>--<entry-id>` (e.g., `nacho/my-feature--c-abc1234`)

### GG-ID Trailers

Each commit gets a stable `GG-ID` trailer that persists across rebases:

```
Add user authentication

Implement JWT-based auth with refresh tokens.

GG-ID: c-abc1234
```

This ID is used to track which PR/MR corresponds to which commit, even after reordering or amending.

### PR/MR Dependencies

PRs/MRs are created with proper target branches:
- First commit targets the base branch (e.g., `main`)
- Subsequent commits target the previous commit's branch

This creates a chain of dependent PRs/MRs that can be reviewed and merged in order.

## Example Workflow

```bash
# 1. Start a new feature
$ gg co user-auth
OK Created stack "user-auth" based on main

# 2. Develop incrementally
$ git add . && git commit -m "Add user model"
$ git add . && git commit -m "Add auth endpoints"
$ git add . && git commit -m "Add login UI"

# 3. Check your stack
$ gg ls
user-auth (3 commits, 0 synced)
  [1] abc1234 Add user model      (id: c-f9a1e2b) (not pushed)
  [2] def5678 Add auth endpoints  (id: c-7c1b9d0) (not pushed)
  [3] ghi9012 Add login UI        (id: c-98ab321) (not pushed) <- HEAD

# 4. Push to remote
$ gg sync --draft
OK Pushed nacho/user-auth--c-f9a1e2b -> MR !101 (draft)
   https://gitlab.com/user/repo/-/merge_requests/101
OK Pushed nacho/user-auth--c-7c1b9d0 -> MR !102 (draft)
   https://gitlab.com/user/repo/-/merge_requests/102
OK Pushed nacho/user-auth--c-98ab321 -> MR !103 (draft)
   https://gitlab.com/user/repo/-/merge_requests/103

# 5. Address review feedback on commit 1
$ gg mv 1
OK Moved to: [1] abc1234 Add user model

$ # make changes...
$ gg sc
OK Squashed into abc1234
OK Rebased 2 commits on top

# 6. Re-sync
$ gg sync
OK Force-pushed nacho/user-auth--c-f9a1e2b
OK Force-pushed nacho/user-auth--c-7c1b9d0
OK Force-pushed nacho/user-auth--c-98ab321

# 7. Land when approved
$ gg land --all
OK Merged MR !101 into main
OK Merged MR !102 into main
OK Merged MR !103 into main

# 8. Clean up
$ gg clean
OK Deleted stack "user-auth" (all merged)
```

## Working with Remote Stacks

You can continue working on stacks from another machine:

```bash
# List stacks that exist on remote but not locally
$ gg ls --remote
Remote stacks:
  ○ user-auth (3 commits) [#101, #102, #103]
  ○ api-refactor (2 commits)

# Check out a remote stack
$ gg co user-auth
→ Fetching remote stack user-auth...
OK Checked out remote stack user-auth

# Continue working normally
$ gg ls
$ gg sync
```

## Shell Completions

Generate completions for your shell:

```bash
# Bash
gg completions bash > ~/.local/share/bash-completion/completions/gg

# Zsh
gg completions zsh > ~/.zfunc/_gg

# Fish
gg completions fish > ~/.config/fish/completions/gg.fish
```

## Troubleshooting

### "gh CLI not installed" / "glab is not installed"

Install the appropriate CLI for your provider:

```bash
# GitHub CLI (macOS)
brew install gh

# GitLab CLI (macOS)
brew install glab

# Other platforms
# GitHub: https://cli.github.com/
# GitLab: https://gitlab.com/gitlab-org/cli#installation
```

### "Not authenticated with GitHub/GitLab"

Run the appropriate auth command:
```bash
gh auth login    # For GitHub
glab auth login  # For GitLab
```

### "Not on a stack branch"

You're on a branch that doesn't follow the `<user>/<stack>` naming convention. Use `gg co <name>` to create or switch to a stack.

### "Merge commits are not supported"

Stacks must have linear history. Rebase your branch to remove merge commits:
```bash
git rebase main
```

## Contributing

Contributions are welcome! Please see the [design document](docs/git-gud-design.md) for architecture details.

## License

MIT License - see [LICENSE](LICENSE) for details.
