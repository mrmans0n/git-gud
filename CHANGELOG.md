# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/mrmans0n/git-gud/compare/v0.2.0...HEAD
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
