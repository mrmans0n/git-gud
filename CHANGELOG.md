# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/mrmans0n/git-gud/compare/v0.3.2...HEAD
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
