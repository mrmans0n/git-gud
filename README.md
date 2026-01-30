# git-gud (gg)

A stacked-diffs CLI tool for GitLab, inspired by Gerrit, Phabricator/Arcanist, and Graphite.

## What are Stacked Diffs?

Stacked diffs allow you to break large changes into small, reviewable commits that build on each other. Each commit becomes its own Merge Request, with proper dependency chains. This enables:

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

- [glab](https://gitlab.com/gitlab-org/cli) - GitLab CLI (used for authentication and MR operations)
- Git 2.x+

Authenticate with GitLab before using git-gud:

```bash
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

# Sync with GitLab (creates MRs)
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

# Land approved MRs
gg land --all

# Clean up merged stacks
gg clean
```

## Commands

### Stack Management

| Command | Description |
|---------|-------------|
| `gg co <name>` | Create a new stack or switch to existing one |
| `gg ls` | List current stack commits with MR status |
| `gg ls --all` | List all stacks in the repository |
| `gg clean` | Remove merged stacks and their remote branches |

### Syncing with GitLab

| Command | Description |
|---------|-------------|
| `gg sync` | Push all commits and create/update MRs |
| `gg sync --draft` | Create new MRs as drafts |
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
| `gg land` | Merge the first approved MR |
| `gg land --all` | Merge all approved MRs in sequence |
| `gg rebase` | Rebase stack onto updated base branch |

### Utilities

| Command | Description |
|---------|-------------|
| `gg lint` | Run lint commands on each commit |
| `gg continue` | Continue after resolving conflicts |
| `gg abort` | Abort current operation |
| `gg completions <shell>` | Generate shell completions |

## Configuration

Configuration is stored in `.git/gg/config.json`:

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
| `defaults.branch_username` | Username for branch naming | `glab whoami` |
| `defaults.lint` | Lint commands to run per commit | `[]` |

## How It Works

### Branch Naming

- **Stack branch**: `<username>/<stack-name>` (e.g., `nacho/my-feature`)
- **Per-commit branches**: `<username>/<stack-name>/<entry-id>` (e.g., `nacho/my-feature/c-abc1234`)

### GG-ID Trailers

Each commit gets a stable `GG-ID` trailer that persists across rebases:

```
Add user authentication

Implement JWT-based auth with refresh tokens.

GG-ID: c-abc1234
```

This ID is used to track which MR corresponds to which commit, even after reordering or amending.

### MR Dependencies

MRs are created with proper target branches:
- First commit targets the base branch (e.g., `main`)
- Subsequent commits target the previous commit's branch

This creates a chain of dependent MRs that can be reviewed and merged in order.

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

# 4. Push to GitLab
$ gg sync --draft
OK Pushed nacho/user-auth/c-f9a1e2b -> MR !101 (draft)
OK Pushed nacho/user-auth/c-7c1b9d0 -> MR !102 (draft)
OK Pushed nacho/user-auth/c-98ab321 -> MR !103 (draft)

# 5. Address review feedback on commit 1
$ gg mv 1
OK Moved to: [1] abc1234 Add user model

$ # make changes...
$ gg sc
OK Squashed into abc1234
OK Rebased 2 commits on top

# 6. Re-sync
$ gg sync
OK Force-pushed nacho/user-auth/c-f9a1e2b
OK Force-pushed nacho/user-auth/c-7c1b9d0
OK Force-pushed nacho/user-auth/c-98ab321

# 7. Land when approved
$ gg land --all
OK Merged MR !101 into main
OK Merged MR !102 into main
OK Merged MR !103 into main

# 8. Clean up
$ gg clean
OK Deleted stack "user-auth" (all merged)
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

### "glab is not installed"

Install the GitLab CLI:
```bash
# macOS
brew install glab

# Other platforms
# See https://gitlab.com/gitlab-org/cli#installation
```

### "Not authenticated with GitLab"

Run `glab auth login` to authenticate.

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
