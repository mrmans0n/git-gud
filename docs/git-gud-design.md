# git-gud design

gg (git gud) is the entry point for a stacked-diffs CLI tool, intended for GitLab and its CLI `[glab](https://gitlab.com/gitlab-org/cli)`.

About stacked diffs: https://newsletter.pragmaticengineer.com/p/stacked-diffs

## Motivation

I want a Gerrit- or Phabricator/Arcanist-like tool specialized in stacked diffs for GitLab repositories. `glab stack` exists, but I want to improve the UX and API to fit my workflow.

## Prior art

There are other projects that aim to do the same, for different providers: git-stack, gh-stack, graphite (gt), git-ps-rs, and others.

I have a series of shell commands in my dotfiles repo, all the functions that start with "gg", as a first version of this. It's not polished and doesn't work great, hence the need to expand on it.

This should have its own repo in my GitHub account.

## Goals & Non-Goals

### Goals

- Provide a superior UX to `glab stack` while maintaining GitLab MR compatibility
- Allow normal `git commit` usage without special commands to add changes
- Intuitive navigation within stacks (first/last/prev/next)
- Bidirectional sync with GitLab (push MRs, pull status)
- Automation of tedious tasks: lint per commit, auto-rebase, absorb
- Show rich stack status (approved, draft, merged, CI status)

### Non-Goals

- GitHub/Bitbucket support (GitLab only initially)
- Replace `glab` in the MVP; use it for auth and GitLab actions
- Web UI or GUI; pure CLI tool
- Non-linear stacks support (dependency trees)

## User Stories

- **US1: Create a new stack for a feature**
- **US2: Add incremental commits to the stack**
- **US3: Respond to code review feedback**
- **US4: Merge the stack when approved**

## Architecture & Data Model

### Stack definition

The stack is the branch. Commits between merge-base and HEAD are the stack entries, and the history must be linear (no merge commits). SHAs change on rebase/amend, so each commit should carry a stable entry id (for example, a `GG-ID: <id>` trailer). Remote branch names and MR mapping use the entry id; order is derived from git history. When creating or updating MRs, strip the `GG-ID` trailer from the commit message so it never appears in MR titles or descriptions.

### State Management (Minimalist Approach)

Derive everything possible from git. Only persist what cannot be derived. Stack commits come from `git log base..HEAD`. Branches follow a naming convention. The only external state is the entry-id->MR mapping.

### Configuration

If it needs extra configuration or repo-local data, use `.git/gg/config.json`. Example:

```json
{
  "defaults": {
    "base": "main",
    "branch_username": "nacho",
    "lint": [
      "cargo fmt",
      "cargo clippy -- -D warnings"
    ]
  },
  "stacks": {
    "my-feature": {
      "base": "main",
      "mrs": {
        "c-3f9a1e2": 1234,
        "c-7c1b9d0": 1235
      }
    }
  }
}
```

Defaults can be overridden per stack. If `branch_username` is not set, fall back to `glab whoami`. If `base` is not set, default to the repo's primary branch: `main`, then `master`, then `trunk`.

If a commit is missing a `GG-ID`, `gg sync` should prompt to add one (amend the commit) or abort. This keeps entry ids stable across rebases and reorders.

If local config is missing, `gg sync` should rebuild the entry-id->MR mapping by scanning remote branches that match `<user>/<stack>/*` and reading the entry id from the branch name or commit trailer.

### Branch Naming Convention

Branches follow the pattern: `<username>/<stack-name>/<entry-id>`

- Local stack branch: `nacho/my-feature` (contains all commits)
- Remote branches for MRs: `nacho/my-feature/c-3f9a1e2`, `nacho/my-feature/c-7c1b9d0`, etc.
- Each remote branch points to exactly one commit in the stack, identified by its entry id
- MR target order is derived from current commit order, not from the branch name

## Technology Recommendation

### Rust

Rust is the recommended choice.

### Justification

1. **Mature CLI ecosystem**: clap (args), indicatif (progress), dialoguer (prompts), skim (fzf-like)
2. **git2-rs**: native bindings to libgit2, much more robust than shell
3. **Simple distribution**: single binary without runtime, installable via cargo or GitHub releases
4. **git-absorb integration**: written in Rust, can be used as a library
5. **Prior art**: git-branchless and other stacking tools are in Rust

### Key Dependencies

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
git2 = "0.18"
reqwest = { version = "0.11", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
skim = "0.10"  # fzf-like fuzzy finder
indicatif = "0.17"  # progress bars
console = "0.15"  # terminal styling
dialoguer = "0.11"  # interactive prompts
```

### Alternatives Considered

- **Shell scripts**: fragile, hard to test, poor error handling. Good for prototyping.
- **Kotlin**: requires JVM, slow startup. Good if reusing Android/backend code.

## GitLab Integration

For the MVP, use `glab` as a subprocess.

- Pros: reuses existing auth, less code, lower maintenance
- Cons: external dependency, output parsing

Prefer non-interactive usage and machine-readable output. Use `--json` where possible and treat non-zero exit codes as hard failures with clear messaging.

Key glab operations:
- `glab auth login`
- `glab whoami`
- `glab mr create`
- `glab mr view`
- `glab mr merge`

Direct GitLab API integration is a future option if more control is needed.

## Command API

### Entrypoint

`gg`

If run standalone, it should be analogous to running `gg ls` and show all stacks that currently exist, plus all available subcommands.

### Checkout / switch

`gg co` or `gg sw`

Create a new stack or switch to an existing stack, passed as the first parameter. If no parameter is passed, it could use fzf to show the list of currently existing stacks in the system.

Example: `gg co new_stack_name_1`

New stacks default to the repo's primary branch (`main`, then `master`, then `trunk`) unless overridden by config. Branch naming follows the convention described above.

### Committing new changes

We should be able to do this with a normal `git commit`, if possible. In glab, glab stack save is needed, but we should try to avoid this.

### Push changes remotely

`gg diff` or `gg sync`

This should make sure the current stack is synced with its respective GitLab MRs. Any commit without its respective MR should be created remotely at this point, with the proper branching dependencies set.

It could also take a `--draft` parameter, which would mean any MR that doesn't already exist should be created as a draft.

When creating or updating MRs, strip the `GG-ID` trailer from commit messages so it never appears in MR titles or descriptions.

### Move branches

`gg mv` or `gg move` + parameter (index, entry id, or commit SHA) to signal where to move. Index is the current display order only; entry id is stable across reorders.

`gg first` to go to the first element of the stack
`gg last` to go to the last element of the stack
`gg prev` to go to the previous element in the stack
`gg next` to go to the next element in the stack

### Making changes to a commit

To the current commit (whether it's the one at the top or we used `gg mv` to get to it), if we make changes, we should have a `gg sc` or `gg squash` to add and squash all those changes into that specific commit. Commits depending on the current commit should be automatically rebased all the way to the top.

### Reorder branches

`gg reorder` to easily reorder elements in the stack (similar to git rebase -i or glab stack reorder).

Could be TUI based with a nice terminal UI to select branches and move them up or down visually.

### List

`gg ls` should show the list of the current stack and its commits, if on a stack. If not on a stack, it should show the available stacks. Use a colored tree view. Ideally, we should also show the current state on the remote for the specific commits (whether they were pushed, merged, waiting for review, approved, drafts, etc).

### Lint

`gg lint` should go from the first commit to the currently selected commit, applying a given set of lint commands (specified in the config). Require a clean working tree; otherwise abort. If after running `gg lint` in one commit there are changes, they should be automatically `gg squash`ed and then proceed to the next commit, up until the commit selected when the command was initially run.

### Land

`gg land` should be able to start landing on GitLab the MRs that are ready to be landed, starting from the first element of the stack. It could have a parameter (like `gg land --all`) that waits until an MR is merged remotely, then schedules the next one, rinse and repeat until the stack up to the current commit is merged (or stop when the MR isn't approved or hasn't been pushed).

### Clean

`gg clean` should be able to clean up any existing already merged stacks.

### Absorb

`gg absorb` should be able to intelligently interleave the current changes in the stack position they should go to. [https://lib.rs/crates/git-absorb](https://lib.rs/crates/git-absorb) does this; we could either use it internally or do our own take on it. The functionality would be a really nice-to-have.

## End-to-end example

End-to-end example of developing a feature with 3 commits:

```bash
# 1. Create stack from main
$ gg co my-new-feature
OK Created stack "my-new-feature" based on main

# 2. Make first change and commit (normal git)
$ git add . && git commit -m "Add data model"

# 3. Make second change
$ git add . && git commit -m "Add API endpoint"

# 4. Make third change
$ git add . && git commit -m "Add UI component"

# 5. View stack status
$ gg ls
my-new-feature (3 commits, 0 synced)
  [1] abc123 Add data model       (id: c-3f9a1e2) (not pushed)
  [2] def456 Add API endpoint     (id: c-7c1b9d0) (not pushed)
  [3] ghi789 Add UI component     (id: c-98ab321) (not pushed) <- HEAD

# 6. Sync with GitLab (create MRs)
$ gg sync --draft
OK Pushed nacho/my-new-feature/c-3f9a1e2 -> MR !1234 (draft)
OK Pushed nacho/my-new-feature/c-7c1b9d0 -> MR !1235 (draft)
OK Pushed nacho/my-new-feature/c-98ab321 -> MR !1236 (draft)

# 7. After receiving feedback, modify commit 1
$ gg mv 1           # Move to commit 1
$ # make changes...
$ gg sc             # Squash changes into current commit
OK Squashed into abc123
OK Rebased 2 commits on top

# 8. Resync
$ gg sync
OK Force-pushed nacho/my-new-feature/c-3f9a1e2
OK Force-pushed nacho/my-new-feature/c-7c1b9d0
OK Force-pushed nacho/my-new-feature/c-98ab321

# 9. When commit 1 is approved, land it
$ gg land
OK Merged MR !1234 into main
OK Rebased remaining stack on main

# 10. Clean up when everything is merged
$ gg clean
OK Deleted stack "my-new-feature" (all merged)
```

## Error Handling & Edge Cases

- **Rebase conflicts**: pause operation, show clear instructions, allow `gg continue` or `gg abort`
- **MR closed manually**: detect on sync and ask whether to recreate or remove from stack
- **Base branch updated**: offer `gg rebase` to update the entire stack on the new main
- **Dirty working directory**: block operations that require a clean working tree, suggest stash
- **glab not installed**: clear error with installation instructions
- **No GitLab auth**: detect and guide the user to `glab auth login`
- **Merge commits in stack**: fail fast and ask the user to rebase to a linear history

## Open Questions

- [x]  Detect new commits automatically (on sync).
- [x]  Single remote only at the beginning (origin).
- [x]  `gg ls` uses a colored tree view.
- [x]  Ship shell completions from the start.
- [x]  Divergent remote stacks: warn and prompt the user to `git pull` or otherwise reconcile; keep the decision manual.

## Roadmap

### MVP (v0.1-v0.2)

- `gg co`: create/switch stack
- `gg ls`: list stack/stacks
- `gg sync`: push and create MRs
- `gg first/last/prev/next`: basic navigation
- `gg sc`: squash into current commit
- `gg reorder`: reorder stack
- `gg mv`: move to a specific commit

### v0.3 - Landing

- `gg land`: merge approved MRs
- `gg clean`: clean up merged stacks
- `gg rebase`: rebase onto main

### v0.4 - Power Features

- `gg lint`: lint per commit
- `gg absorb`: intelligently absorb changes
- TUI for reorder
- Shell completions
