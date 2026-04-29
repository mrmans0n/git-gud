# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.2] - 2026-04-29

### Added
- `gg unstack` command to split a stack into multiple independent stacks (#309).
- Worktree support for `gg unstack` via `--wt/--worktree` (#313).

### Changed
- Provider default detection now follows the remote host more reliably (#303).
- `gg inbox` now uses provider-specific PR/MR labels (#306).
- Updated dependencies: `skim` to v4.6.1 and `clap_complete` to v4.6.3 (#304, #307).
- Skills setup docs are now agent-neutral (#308).

### Fixed
- `gg unstack` now hardens name resolution and rebase cleanup paths (#310).
- `gg sync` now recreates PRs when the source branch changes after unstack (#314).
- `gg land --clean` now preserves verified merge state during auto-cleanup so remote branches are deleted after successful landing (#318).

## [0.9.1] - 2026-04-24

### Changed
- `gg sync` now decouples title updates from description updates, avoiding unnecessary title churn during sync (#302).

### Fixed
- `gg sync` / `gg land` now retarget downstream GitLab MRs after a parent merge (#298).
- `gg rebase` now allows stacks with squash-merged commits (#296).

## [0.9.0] - 2026-04-20

### Added
- `gg undo` command: reverses the local ref/HEAD effects of the most recent
  mutating `gg` command. `gg undo --list` shows the recent operation log
  (newest-first, with id, kind, status, args, and undoability); `gg undo
  <operation_id>` targets a specific record. A second `gg undo` redoes the
  first (undo of an undo). Operations that touched a remote (`sync`, `land`)
  are recorded but refused for local replay — `gg` prints a provider-specific
  revert hint instead of silently rewriting published history. The working
  tree is never modified.
- Per-repo operation log at `<commondir>/gg/operations/*.json` feeds the new
  command. The log is a bounded ring buffer of 100 records; `Pending`
  records are never pruned so interrupted operations stay visible.
- `gg undo --json` emits a stable, additive response schema
  (`UndoResponse` / `UndoListResponse`) — see `docs/src/commands/undo.md`.
- MCP tools `stack_undo` and `stack_undo_list` shell out to the CLI for
  agentic undo / log-inspection workflows.
- Immutability guard for history-rewriting commands: `gg sc`, `gg drop`,
  `gg reorder`/`gg arrange`, `gg split`, `gg absorb`, and `gg rebase` now
  refuse by default to rewrite commits whose PR/MR is already merged or which
  are already reachable from `origin/<base>`. A new `-f, --force` flag (alias
  `--ignore-immutable`) bypasses the check for each of these commands.
- `gg log` smartlog view plus the `stack_log` MCP tool (#279).
- `gg sync --no-verify` to skip pre-push hooks when explicitly requested (#283).
- `gg restack` ancestry repair command (#289).
- `gg inbox` command for multi-stack actionable triage (#292).

### Changed
- **Behavior change — new lock acquisition.** `gg reorder`, `gg absorb`,
  `gg reconcile`, and `gg run --amend` now acquire the existing operation
  lock for the duration of their work. Concurrent invocations will serialize
  instead of racing, which closes a latent window where two of these
  commands running in parallel could corrupt ref state. User-visible
  effect: a second concurrent invocation will print "Another gg operation
  is currently running" and exit, instead of proceeding unsafely.
- `gg drop --force` now *also* overrides the immutability check, in addition
  to skipping the existing confirmation prompt. Scripts that previously
  relied on `--force` to silently rewrite merged commits will continue to
  succeed; interactive users get a stronger safety net before they opt in.
- Updated dependencies including `uuid`, `tokio`, and `rmcp` (#278, #284, #287).
- TUI polish including lazygit-style bottom bar key hints (#295).

### Fixed
- `gg rebase` now allows stacks with squash-merged commits to refresh cleanly,
  including cases where merged commits drop only after fetching the latest base
  branch (#294, #296).
- `gg sync` / `gg land` now retarget downstream GitLab MRs after a parent merge (#298).
- `gg undo --list` aligns the ARGS column correctly (#290).
- Removed stale research/docs cruft from the repo.

## [0.8.3] - 2026-04-16

### Added
- Stack navigation comments on GitHub and GitLab pull requests and merge requests via `gg sync` (#276)

### Changed
- Updated `clap` to v4.6.1 (#277)

## [0.8.2] - 2026-04-15

### Added
- `gg sync` now preserves manual pull request body edits while still managing the generated sections it owns (#272)

### Changed
- Updated dependencies: `tokio` to v1.51.1 and v1.52.0, `clap_complete` to v4.6.2, and `skim` to v4.6.0 (#267, #268, #269, #273)
- README refresh and cleanup

## [0.8.0] - 2026-04-13

### Added
- `gg run` command to execute commands across stack commits, with read-only, amend, discard, parallel, and JSON output modes (#258)
- `--admin` flag for `gg land` to allow PR approval bypass flows where supported (#262)

### Changed
- `gg split` now defaults to the interactive hunk-selection TUI, removing the need for a separate `-i` mode (#265)

### Fixed
- `gg split` no longer fails when the editor is left unchanged and the commit message would otherwise become empty (#263)

### Dependencies
- Updated `rmcp` to v1.4.0 (#256)

## [0.7.4] - 2026-04-08

### Fixed
- `gg clean` now falls back to a detached `HEAD` strategy when branch checkout fails because the branch is checked out in a linked worktree (#255)

### Changed
- Updated `skim` to v4.5.1 (#253, #254)

## [0.7.3] - 2026-04-05

### Fixed
- `gg sync --draft` no longer converts existing GitHub pull requests to draft unexpectedly

## [0.7.2] - 2026-03-26

### Added
- GG-Parent trailers and normalized stack metadata for stack-aware workflows (#237)

### Improved
- Stack workflow roadmap planning updates (#233, #235)

## [0.7.1] - 2026-03-20

### Fixed
- `gg sync --lint` now aborts and restores repository state on lint failure instead of continuing with sync side effects (#232)

## [0.7.0] - 2026-03-19

### Added
- `gg drop` command — remove one or more commits from the stack by position, SHA, or GG-ID (#223)
- `gg drop` alias: `gg abandon` (inspired by jj) (#223)
- `gg arrange` alias for `gg reorder` (#222)
- Drop support in `gg reorder` TUI — press `d` to mark commits for dropping (#222)
- MCP tools: `stack_drop`, `stack_split`, `stack_reorder` — expose drop, split, and reorder via MCP server (#225)

## [0.6.4] - 2026-03-18

### Fixed
- Resolve `.git/` paths via `commondir()` for worktree support — lint scripts in `.git/gg/` now work in linked worktrees (#221)
- Allow remote branch deletion in `gg clean` when provider check fails but ancestor check passes (#220)

## [0.6.3] - 2026-03-17

### Improved
- `gg sync` auto-rebase UX: when `auto_rebase` is enabled, the warning now says "Auto-rebasing..." instead of the confusing "Run 'gg rebase' first to update." (#219)

## [0.6.2] - 2026-03-14

### Added
- Global config file support (`~/.config/gg/config.json`) — personal defaults that apply across all repositories (#218)
- `gg setup --all` flag — full setup mode with all options organized into groups (General, Sync, Land, Lint, Worktrees, GitLab) (#218)
- `sync_draft` config option — create new PRs/MRs as drafts by default (#218)
- `sync_update_descriptions` config option — control whether PR/MR descriptions are updated on re-sync (#218)

### Changed
- `gg setup` (without `--all`) now only asks essential settings: provider, base branch, and username (#218)

## [0.6.1] - 2026-03-13

### Added
- `gg reorder` interactive TUI — drag-and-drop commit reordering with arrow keys, full ratatui interface with color-coded commit list, `--no-tui` fallback for sequential prompts (#215)

### Fixed
- `TerminalGuard` cleanup: raw mode and alternate screen are now reliably restored on panic, Ctrl+C, or early return in all TUI commands (#217)
- `--no-tui` integration test no longer requires a real terminal (#217)

## [0.6.0] - 2026-03-13

### Added
- `gg split` command — split any commit in the stack into two, by file or by hunk (#207)
- Interactive hunk selection (`gg split -i`) with a full ratatui TUI: two-panel layout (files + colored diff), hunk checkboxes, keyboard navigation (#209, #210)
- Inline commit message editing in the split TUI — both the new commit and the remainder commit messages are edited inline, no external editor needed (#213, #214)
- `--no-tui` flag for `gg split -i` to fall back to sequential prompts (#211)

### Fixed
- Split TUI: Ctrl+C now aborts properly in raw mode (#211)
- Split TUI: diff panel scrolling works correctly when hunks exceed visible area (#211)
- Split TUI: path truncation no longer panics on narrow terminals or non-ASCII file paths (#211)

### Changed
- Updated dependencies: `console` to v0.16.3 (#208)

## [0.5.6] - 2026-03-12

### Added
- Git operation lock detection: `gg` now checks for `.git/index.lock` before starting operations — if `git` is running concurrently, `gg` waits (up to 10s) instead of risking repository corruption (#204)
- `gg setup` now prompts for all configurable fields (base branch, username, lint commands) (#196)

### Fixed
- `gg sync --lint` no longer crashes after rebase drops landed commits — fixed out-of-range index in lint position calculation (#200)
- Network errors during auth check no longer show misleading "Not authenticated" message — `gg` now prints a warning and continues instead of blocking (#203)
- Sync lint regression test is now auth-independent and exercises the actual lint-after-rebase code path (#205)

### Changed
- Updated dependencies: `rmcp` to v1.2.0 (#197)

## [0.5.5] - 2026-03-11

### Fixed
- `gg land --wait --all` now shows CI failure details when stopping mid-stack — displays failed job names and stages instead of just "Landed N MR(s)" (#195)

### Changed
- Updated dependencies: `skim` to v4 (#192)

## [0.5.4] - 2026-03-09

### Fixed
- `gg land --wait` now shows error messages instead of silently exiting — errors were swallowed in non-JSON mode (#190)
- `gg land --wait` retries transient API failures (up to 5 consecutive) instead of aborting the entire wait on a single network hiccup (#190)
- `gg land --wait --json` no longer emits duplicate JSON objects on error (#190)

### Changed
- Updated dependencies: `rmcp` to v1.1.1 (#191)

## [0.5.3] - 2026-03-09

### Fixed
- `gg sync` now correctly detects when a branch needs rebasing even if local main is up-to-date — uses merge-base between HEAD and origin/main instead of comparing local main vs origin/main (#189)

### Changed
- Updated dependencies: `skim` to v3.7.0, `uuid` to v1.22.0, `rmcp` to v1 (#183, #184, #185, #186, #187, #188)

## [0.5.2] - 2026-02-24

### Fixed
- `gg land --wait` now responds to Ctrl+C promptly instead of waiting up to 10 seconds — replaced blocking sleep with interruptible 250ms-chunk sleep (#181)
- Second Ctrl+C during `--wait` now force-exits immediately via `abort()` (#181)
- Use `git describe` to find previous tag for changelog generation (#179)
- CI: fetch full history and tags for changelog generation

### Added
- Marketplace metadata for Claude plugin (#180)

## [0.4.2] - 2026-02-20

### Fixed
- `gg amend` / `gg rebase` no longer leave detached HEAD inside git worktrees — added `ensure_branch_attached` helper that safely re-attaches HEAD after rebase (#162)
- `gg land` now shows a message when a PR/MR is already merged or closed, instead of silently skipping (#163)
- `gg clean` now correctly detects squash-merged stacks as merged — removed overly strict commit reachability check that failed with squash/rebase merge strategies (#164)
- `gg ls --remote` now separates active and landed stacks — merged stacks shown at the bottom with ✓ marker (#164)
- `gg land --wait` no longer exits immediately after adding MR to merge train — added grace period for GitLab to register the MR in the train queue (#165)

### Changed
- Updated dependencies: `anyhow` to v1.0.102, `clap` to v4.5.60 (#160, #161)

## [0.4.1] - 2026-02-19

### Fixed
- `gg amend` no longer falsely reports "Unstaged changes detected" in git worktrees — replaced libgit2 status checks with `git diff` subprocess (#158)
- `gg amend` no longer leaves detached HEAD after amending a non-tip commit in a stack (#159)

## [0.4.0] - 2026-02-19

### Added
- `--json` flag for `gg ls` — structured JSON output for single stack, all stacks, and remote stacks (#150)
- `--json` flag for `gg sync` — per-entry results with action/error fields and warnings (#152)
- `--json` flag for `gg land` — partial results, merge train support, error tracking (#154)
- `--json` flag for `gg clean` — requires `--all` in JSON mode for safety (#155)
- `--json` flag for `gg lint` — per-commit, per-command pass/fail results (#155)
- `gg ls --json` automatically refreshes PR/MR state from provider API (best-effort) (#156)

### Fixed
- Dynamic PR/MR labels in all user-facing messages — uses provider detection instead of hardcoded strings (#153)
- `gg lint` now runs commands from repo root directory, fixing relative path issues (#157)
- Lint failures are now surfaced as warnings in `gg sync --json` output (#155)
- Push failures in JSON mode no longer discard accumulated entry results (#152)
- Rebase output no longer contaminates JSON stdout (#152)

## [0.3.3] - 2026-02-18

### Added
- `unstaged_action = "add"` option to automatically stage all unstaged changes and continue during amend (#148)
- Integration test for `unstaged_action = "add"` behavior (#149)

## [0.3.2] - 2026-02-18

### Added
- `gg amend` warns about unstaged changes and offers to auto-stash them (#144)
- `unstaged_action` config option (`ask`, `stash`, `continue`, `abort`) to set default behavior for unstaged changes during amend (#145)
- `gg setup` now writes `unstaged_action` explicitly in config

## [0.3.1] - 2026-02-17

### Added
- `gg sync` now detects when stack base is behind `origin/<base>`, warns, and suggests rebasing first (#143)
- `gg sync --no-rebase-check` to bypass behind-base check for a single sync (#143)
- `sync_auto_rebase` (`sync.auto_rebase`) config to auto-run rebase during sync when threshold is met (#143)
- `sync_behind_threshold` (`sync.behind_threshold`) config to tune/disable behind-base checks (`0` disables) (#143)
- `gg ls` shows a `↓N` indicator when stack base is behind `origin/<base>` (#143)
- mdBook documentation with GitHub Pages deploy and PR previews (#139)

### Fixed
- Use hidden progress bar when stderr is not a TTY (#141)

### Dependencies
- Updated clap to 4.5.59 (#142)
- Updated indicatif to 0.18.4 (#140)

## [0.3.0] - 2026-02-14

### Added
- `--no-limit` / `-n` flag for `gg absorb`: search all commits in the stack instead of the default 10 (#138)
- `--squash` / `-s` flag for `gg absorb`: squash fixup commits directly instead of creating fixup! commits (#138)

### Changed
- Upgraded git-absorb dependency to 0.9 (#137)

### Improved
- Comprehensive integration tests for absorb (basic, worktree, edge cases) (#137, #138)

### Dependencies
- Updated uuid to 1.21.0 (#135)
- Fixed Homebrew formula capitalization and lint

## [0.2.1] - 2026-02-13

### Fixed
- `gg absorb` now works correctly in linked worktrees by setting `GIT_DIR`/`GIT_WORK_TREE` for the git-absorb library (#134)
- `gg rebase` no longer fails when the base branch is checked out in another worktree — uses `git update-ref` instead of checkout (#134)

## [0.2.0] - 2026-02-13

### Added
- Managed worktree support: `gg co --wt`/`--worktree` creates a git worktree for the stack (#133)
- Configurable worktree base path via `worktree_base_path` in config (#133)
- `[wt]` indicator in `gg ls` for stacks with associated worktrees (#133)
- `gg clean` detects and removes worktrees for merged stacks with confirmation (#133)

### Fixed
- Shared state across git worktrees using `commondir` (#131)
- Handle branches checked out in worktrees during clean (#132)
- Fall back to git-based merge detection when provider check fails (#133)
- Strip redundant version and article from Homebrew formula

## [0.1.16] - 2026-02-11

### Fixed
- Reload stack state after rebase in merge train flow (#129)
- Sort merge train results ascending to fix position calculation (#128)
- Preserve draft state when syncing existing GitLab MRs (#125)

## [0.1.15] - 2026-02-10

### Fixed
- Preserve lint changes when adding GG-IDs during sync (#123)

## [0.1.14] - 2026-02-09

### Fixed
- Prevent stack cleanup when unsynced commits exist (#121)

## [0.1.13] - 2026-02-07

### Fixed
- `gg lint` rebase conflicts now stay on stack branch instead of detached HEAD (#120)
- `gg continue` properly updates stack branch after resolving lint conflicts (#120)

### Improved
- Better UX when rebase conflict occurs during lint (#118)

## [0.1.12] - 2026-02-07

### Changed
- Remove Windows from cargo-dist targets (skim-tuikit dependency uses Unix-only APIs)

### Added
- Automated releases with cargo-dist
- Homebrew formula auto-publishing to `mrmans0n/homebrew-tap`

## [0.1.11] - 2026-02-07

### Added
- Setup cargo-dist for automated releases

## [0.1.10] - 2026-02-06

### Fixed
- Remove redundant `Draft:` prefix for GitLab MRs
- Fix rebase/continue UX issues
- Fix `gg land --wait --all` to poll merge train until merged

### Added
- Spinner UI with elapsed time for `gg land --wait`

## [0.1.9] - 2026-02-05

### Fixed
- Rebase remaining commits after lint amends a commit
- Rebase remaining branches after landing stacked PRs

### Added
- Integration tests for stacked PR rebase after squash merge

## [0.1.8] - 2026-02-04

### Added
- `--lint` flag to sync command
- `--until` flag to sync and land commands

### Fixed
- Run lint only once in sync command
- Restore branch after `lint --until` makes changes

### Improved
- Better error formatting for push failures
- Show "Up to date" when sync has no changes to push

## [0.1.7] - 2026-02-04

### Added
- `wp` alias for `clean` command

### Fixed
- Restore stack branch after lint squash
- Import PR mappings when checking out remote stacks

## [0.1.6] - 2026-02-03

### Fixed
- Replace recursion with loop in sync to prevent non-reentrant lock error
- Prevent double-lock in land command

## [0.1.5] - 2026-02-03

### Added
- Initial public release with core stacked diffs functionality

[Unreleased]: https://github.com/mrmans0n/git-gud/compare/v0.9.2...HEAD
[0.9.2]: https://github.com/mrmans0n/git-gud/compare/v0.9.1...v0.9.2
[0.9.1]: https://github.com/mrmans0n/git-gud/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/mrmans0n/git-gud/compare/v0.8.3...v0.9.0
[0.8.3]: https://github.com/mrmans0n/git-gud/compare/v0.8.2...v0.8.3
[0.8.2]: https://github.com/mrmans0n/git-gud/compare/v0.8.0...v0.8.2
[0.8.0]: https://github.com/mrmans0n/git-gud/compare/v0.7.4...v0.8.0
[0.7.4]: https://github.com/mrmans0n/git-gud/compare/v0.7.3...v0.7.4
[0.7.3]: https://github.com/mrmans0n/git-gud/compare/v0.7.2...v0.7.3
[0.7.2]: https://github.com/mrmans0n/git-gud/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/mrmans0n/git-gud/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/mrmans0n/git-gud/compare/v0.6.4...v0.7.0
[0.6.4]: https://github.com/mrmans0n/git-gud/compare/v0.6.3...v0.6.4
[0.6.3]: https://github.com/mrmans0n/git-gud/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/mrmans0n/git-gud/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/mrmans0n/git-gud/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/mrmans0n/git-gud/compare/v0.5.6...v0.6.0
[0.5.6]: https://github.com/mrmans0n/git-gud/compare/v0.5.5...v0.5.6
[0.5.5]: https://github.com/mrmans0n/git-gud/compare/v0.5.4...v0.5.5
[0.5.4]: https://github.com/mrmans0n/git-gud/compare/v0.5.3...v0.5.4
[0.5.3]: https://github.com/mrmans0n/git-gud/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/mrmans0n/git-gud/compare/v0.4.2...v0.5.2
[0.4.2]: https://github.com/mrmans0n/git-gud/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/mrmans0n/git-gud/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/mrmans0n/git-gud/compare/v0.3.3...v0.4.0
[0.3.3]: https://github.com/mrmans0n/git-gud/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/mrmans0n/git-gud/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/mrmans0n/git-gud/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/mrmans0n/git-gud/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/mrmans0n/git-gud/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/mrmans0n/git-gud/compare/v0.1.16...v0.2.0
[0.1.16]: https://github.com/mrmans0n/git-gud/compare/v0.1.15...v0.1.16
[0.1.15]: https://github.com/mrmans0n/git-gud/compare/v0.1.14...v0.1.15
[0.1.14]: https://github.com/mrmans0n/git-gud/compare/v0.1.13...v0.1.14
[0.1.13]: https://github.com/mrmans0n/git-gud/compare/v0.1.12...v0.1.13
[0.1.12]: https://github.com/mrmans0n/git-gud/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/mrmans0n/git-gud/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/mrmans0n/git-gud/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/mrmans0n/git-gud/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/mrmans0n/git-gud/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/mrmans0n/git-gud/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/mrmans0n/git-gud/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/mrmans0n/git-gud/releases/tag/v0.1.5
