# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/mrmans0n/git-gud/compare/v0.1.12...HEAD
[0.1.12]: https://github.com/mrmans0n/git-gud/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/mrmans0n/git-gud/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/mrmans0n/git-gud/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/mrmans0n/git-gud/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/mrmans0n/git-gud/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/mrmans0n/git-gud/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/mrmans0n/git-gud/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/mrmans0n/git-gud/releases/tag/v0.1.5
