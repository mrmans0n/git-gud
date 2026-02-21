# git-gud (gg)

A stacked-diffs CLI tool for GitHub and GitLab, inspired by Gerrit, Phabricator/Arcanist, and Graphite.

## What are Stacked Diffs?

Stacked diffs allow you to break large changes into small, reviewable commits that build on each other. Each commit becomes its own Pull Request (GitHub) or Merge Request (GitLab), with proper dependency chains. This enables:

- **Faster reviews** - Small, focused changes are easier to review
- **Parallel work** - Start the next feature while waiting for review
- **Clean history** - Each commit is a logical unit of change

You can read more [here](https://newsletter.pragmaticengineer.com/p/stacked-diffs) or [here](https://graphite.com/guides/stacked-diffs).

## Installation

### Homebrew (macOS/Linux)

```bash
brew install mrmans0n/tap/gg-stack
```

### From crates.io

```bash
cargo install gg-stack
```

### From source

```bash
cargo install --path .
```

## Prerequisites

- Git 2.x+
- For **GitHub** repositories: [gh](https://cli.github.com/) - GitHub CLI
- For **GitLab** repositories: [glab](https://gitlab.com/gitlab-org/cli) - GitLab CLI

git-gud automatically detects your remote provider from the URL (`github.com` or `gitlab.com`) and uses the appropriate CLI tool.

> **Self-hosted instances**: For GitHub Enterprise or self-hosted GitLab (e.g., `gitlab.mycompany.com`), run `gg setup` to manually select your provider.

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
gg sc             # Squash changes into current commit (or: gg amend)

# Land approved PRs/MRs
gg land --all

# Clean up merged stacks
gg clean
```

📚 **[Full documentation](https://mrmans0n.github.io/git-gud/)** — guides, command reference, configuration, and more.

## Worktree Support

`gg co` supports managed Git worktrees so you can develop a stack in its own checkout while keeping your original repository working tree untouched.

### Create a stack worktree

```bash
gg co my-feature --wt
# or
gg co my-feature --worktree
```

This creates (or reuses) a managed worktree for the stack and checks it out there.

### Default worktree location

By default, git-gud creates worktrees next to your repository using:

`../<repo-name>.<stack-name>`

For example, if your repo is at `/code/my-repo`, stack `user-auth` is created at:

`/code/my-repo.user-auth`

You can customize the base directory with `defaults.worktree_base_path` in `.git/gg/config.json`.

```json
{
  "defaults": {
    "worktree_base_path": "/tmp/gg-worktrees"
  }
}
```

### Stack visibility and cleanup

- `gg ls` / `gg ls --all` shows `[wt]` for stacks that have an associated worktree.
- `gg clean` detects associated stack worktrees and removes them as part of cleanup (with confirmation unless `--all` is used).

### Typical worktree workflow

```bash
# 1) Create stack in a worktree
gg co user-auth --wt

# 2) Work inside the new worktree
cd ../my-repo.user-auth
git add . && git commit -m "Add user model"
git add . && git commit -m "Add auth endpoints"

# 3) Sync stacked branches / PRs/MRs
gg sync

# 4) Inspect stacks (worktree stacks are marked with [wt])
gg ls --all

# 5) After landing, clean stack + managed worktree
gg clean
```

## Commands

### Stack Management

| Command | Description |
|---------|-------------|
| `gg co <name>` | Create a new stack, switch to existing, or checkout from remote |
| `gg ls` | List current stack commits with PR/MR status (shows `↓N` when base is behind `origin/<base>`) |
| `gg ls --all` | List all stacks in the repository |
| `gg ls --remote` | List remote stacks not checked out locally |
| `gg clean` | Remove merged stacks and their remote branches |

### Syncing

| Command | Description |
|---------|-------------|
| `gg sync` | Push all commits and create/update PRs/MRs |
| `gg sync --draft` | Create new PRs/MRs as drafts |
| `gg sync --force` | Force push even if remote diverged |
| `gg sync --update-descriptions` | Update PR/MR titles and descriptions to match commit messages |
| `gg sync --until <target>` | Sync only up to a specific commit (by position, GG-ID, or SHA) |
| `gg sync --no-rebase-check` | Skip checking whether the stack base is behind `origin/<base>` |

**Draft propagation:** If a commit title starts with `WIP:` or `Draft:` (case-insensitive), that PR/MR and all subsequent ones in the stack are created/kept as drafts automatically (even without `--draft`).

**Base-behind detection in sync:** Before pushing, `gg sync` checks whether your stack base is behind `origin/<base>`. If behind and above threshold, gg warns that PRs/MRs may include unrelated changes and offers to run `gg rebase` first. This check can be disabled per command with `--no-rebase-check`, disabled globally with `sync_behind_threshold: 0` (`sync.behind_threshold`), or automated with `sync_auto_rebase: true` (`sync.auto_rebase`).

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
| `gg sc` / `gg amend` | Squash staged changes into current commit |
| `gg sc --all` | Squash all changes (staged + unstaged) |
| `gg reorder` | Reorder commits interactively |
| `gg absorb` | Auto-distribute changes to appropriate commits |

### Landing

| Command | Description |
|---------|-------------|
| `gg land` | Merge the first approved PR/MR (squash by default) |
| `gg land --all` | Merge all approved PRs/MRs in sequence |
| `gg land --wait` | Wait for CI to pass and approvals before merging |
| `gg land --all --wait` | Wait and merge all PRs/MRs in sequence |
| `gg land --no-squash` | Merge using merge commit instead of squash |
| `gg land --auto-merge` | *(GitLab only)* Queue MR auto-merge ("merge when pipeline succeeds") instead of merging immediately |
| `gg land --until <target>` | Land only up to a specific commit (by position, GG-ID, or SHA) |
| `gg land --clean` | Automatically clean up stack after landing all PRs/MRs |
| `gg land --no-clean` | Disable automatic cleanup (overrides config default) |
| `gg rebase` | Rebase stack onto updated base branch |

**Notes:**
- The `--wait` flag polls for CI status and approvals with a configurable timeout (default: 30 minutes). Configure with `land_wait_timeout_minutes` in `.git/gg/config.json`.
- The `--auto-merge` flag is GitLab-only and requests "merge when pipeline succeeds" instead of an immediate merge. You can enable this behavior by default with `defaults.gitlab.auto_merge_on_land` in `.git/gg/config.json`.
- The `--clean` and `--no-clean` flags control automatic stack cleanup after landing all PRs/MRs. If neither is specified, the behavior is controlled by the `land_auto_clean` config option (default: `false`). Use `--clean` to enable cleanup for a single command, or `--no-clean` to override a `true` config default.

### Utilities

| Command | Description |
|---------|-------------|
| `gg setup` | Generate or update `.git/gg/config.json` interactively |
| `gg lint` | Run lint commands on each commit |
| `gg reconcile` | Reconcile stacks that were pushed without using `gg sync` |
| `gg reconcile --dry-run` | Show what reconcile would do without making changes |
| `gg continue` | Continue after resolving conflicts |
| `gg abort` | Abort current operation |
| `gg completions <shell>` | Generate shell completions |

## Configuration

Configuration is stored in `.git/gg/config.json`. Run `gg setup` to generate it interactively:

```json
{
  "defaults": {
    "provider": "gitlab",
    "base": "main",
    "branch_username": "your-username",
    "lint": [
      "cargo fmt --check",
      "cargo clippy -- -D warnings"
    ],
    "unstaged_action": "ask"
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

All configuration options are in the `defaults` section (with provider-specific options nested under `defaults.gitlab`, etc):

| Option | Type | Description | Default |
|--------|------|-------------|---------|
| `provider` | `string` | Git hosting provider (`"github"` or `"gitlab"`). Required for self-hosted instances. | Auto-detect from URL |
| `base` | `string` | Default base branch for new stacks | Auto-detect (main/master/trunk) |
| `branch_username` | `string` | Username prefix for branch naming | Auto-detect via `gh whoami`/`glab whoami` |
| `lint` | `array` | Lint commands to run on each commit with `gg lint` | `[]` |
| `auto_add_gg_ids` | `boolean` | Automatically add GG-IDs to commits without prompting | `true` |
| `unstaged_action` | `string` | Default behavior for `gg sc`/`gg amend` when unstaged changes exist: `"ask"` (prompt), `"add"` (stage all changes), `"stash"` (auto-stash), `"continue"` (ignore unstaged), `"abort"` (fail) | `"ask"` |
| `land_wait_timeout_minutes` | `number` | Timeout in minutes for `gg land --wait` | `30` |
| `land_auto_clean` | `boolean` | Automatically clean up stack after landing all PRs/MRs | `false` |
| `sync_auto_lint` | `boolean` | Automatically run `gg lint` before `gg sync` | `false` |
| `sync_auto_rebase` (`sync.auto_rebase`) | `boolean` | Automatically run `gg rebase` before `gg sync` when base is behind threshold | `false` |
| `sync_behind_threshold` (`sync.behind_threshold`) | `number` | Warn/rebase in `gg sync` when base is at least this many commits behind `origin/<base>` (`0` disables check) | `1` |
| `worktree_base_path` | `string` | Base directory used by `gg co --wt` / `--worktree` for managed stack worktrees | Parent directory of current repository |
| `gitlab.auto_merge_on_land` | `boolean` | *(GitLab only)* Use "merge when pipeline succeeds" for `gg land` by default | `false` |

Example configuration:

```json
{
  "defaults": {
    "base": "main",
    "branch_username": "nacho",
    "lint": [
      "cargo fmt --check",
      "cargo clippy -- -D warnings"
    ],
    "auto_add_gg_ids": true,
    "unstaged_action": "ask",
    "land_wait_timeout_minutes": 60,
    "land_auto_clean": true,
    "sync_auto_lint": true,
    "sync_auto_rebase": false,
    "sync_behind_threshold": 1,
    "worktree_base_path": "/tmp/gg-worktrees",
    "gitlab": {
      "auto_merge_on_land": true
    }
  }
}
```

### PR/MR Description Templates

You can customize PR/MR descriptions by creating a template file at `.git/gg/pr_template.md`. When this file exists, it will be used for all new PR/MR descriptions created by `gg sync`.

#### Template Placeholders

| Placeholder | Description |
|-------------|-------------|
| `{{title}}` | The PR/MR title (from commit subject) |
| `{{description}}` | The commit body/description (empty if none) |
| `{{stack_name}}` | Name of the current stack |
| `{{commit_sha}}` | Short SHA of the commit |

#### Example Template

Create `.git/gg/pr_template.md`:

```markdown
## Summary

{{description}}

---

**Stack:** `{{stack_name}}`
**Commit:** {{commit_sha}}

## Checklist

- [ ] Tests added/updated
- [ ] Documentation updated
```

If no template file exists, git-gud uses the commit description directly, or a default fallback message if the commit has no body.

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
user-auth (3 commits, 0 synced) ↓2
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
$ gg sc  # or: gg amend
OK Squashed into abc1234
OK Rebased 2 commits on top

# 6. Re-sync
$ gg sync
⚠ Your stack is 2 commits behind origin/main. PRs may show unrelated changes. Run 'gg rebase' first to update.
? Rebase before syncing? [Y/n]
OK Rebased stack onto main
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

Generate completions for your shell, then enable them in your shell config:

```bash
# Bash
mkdir -p ~/.local/share/bash-completion/completions
gg completions bash > ~/.local/share/bash-completion/completions/gg

# Add to ~/.bashrc (if bash-completion isn't already enabled)
# source /usr/share/bash-completion/bash_completion
# or on some distros:
# source /etc/bash_completion

# Zsh
mkdir -p ~/.zfunc
gg completions zsh > ~/.zfunc/_gg

# Add to ~/.zshrc
# fpath=(~/.zfunc $fpath)
# autoload -Uz compinit && compinit

# Fish
mkdir -p ~/.config/fish/completions
gg completions fish > ~/.config/fish/completions/gg.fish
```

## Reconciling Out-of-Sync Stacks

If you (or someone else) pushed commits without using `gg sync`, your stack may be out of sync:
- Commits missing GG-IDs
- PRs/MRs exist but aren't tracked in config

Use `gg reconcile` to fix this:

```bash
# Check what would be reconciled (safe, no changes made)
$ gg reconcile --dry-run
→ Analyzing stack my-feature (3 commits)...

→ 2 commits need GG-IDs:
  • abc1234 Add data model
  • def5678 Add API endpoint

→ 1 existing PRs found to map:
  • nacho/my-feature--c-9a8b7c6 → PR #42

→ Dry run complete. No changes made.

# Actually reconcile (will prompt before making changes)
$ gg reconcile
→ Analyzing stack my-feature (3 commits)...
→ 2 commits need GG-IDs
Add GG-IDs to commits? (requires rebase) [y/n]: y
OK Added GG-IDs to commits
OK Mapped c-9a8b7c6 → PR #42
OK Reconciliation complete!
```

**What reconcile does:**
1. **Adds GG-IDs to commits** that don't have them (via rebase)
2. **Finds existing PRs/MRs** for your entry branches and maps them in config

**When to use:**
- After pushing with `git push` instead of `gg sync`
- When inheriting a stack from another machine that got out of sync
- When PRs were created manually outside of git-gud

## AI Agent Integration

git-gud ships as a [Claude Code plugin](https://code.claude.com/docs/en/plugins) following the open [Agent Skills](https://agentskills.io) standard. AI coding agents — Claude Code, Cursor, Gemini CLI, OpenAI Codex, and others — can use `gg` for stacked-diff workflows.

### Quick setup

```bash
# Install from the Claude Code marketplace
claude plugin marketplace add https://github.com/mrmans0n/git-gud
claude plugin install git-gud

# Or load directly
claude --plugin-dir /path/to/git-gud
```

One unified skill is included:
- **gg** — Stacked diffs with GitHub (`gh` CLI) or GitLab (`glab` CLI, merge trains)

📚 **[Agent Skills Guide](https://mrmans0n.github.io/git-gud/guides/agent-skills.html)** — full setup, usage, and agent operating rules.

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

## License

MIT License - see [LICENSE](LICENSE) for details.
