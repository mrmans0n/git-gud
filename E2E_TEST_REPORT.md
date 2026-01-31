# E2E Testing Report - git-gud with GitHub

## Test Date
2026-01-31

## Environment
- Platform: macOS (arm64)
- Repository: mrmans0n/git-gud-playground (GitHub)
- GitHub CLI: gh 2.x (authenticated as mrmans0n)

## Bugs Found & Fixed

### 1. GitLab → GitHub Migration
**Issue**: Original code was hardcoded for GitLab (glab CLI)
**Fix**: Created new `gh.rs` module with GitHub CLI support, updated all commands to use GitHub PR APIs instead of GitLab MR APIs
**Files changed**:
- New: `src/gh.rs`
- Modified: `src/main.rs`, `src/error.rs`, `src/stack.rs`
- Modified: `src/commands/checkout.rs`, `sync.rs`, `land.rs`, `ls.rs`, `clean.rs`, `setup.rs`

### 2. Branch Naming Conflict
**Issue**: Git doesn't allow refs like `foo` and `foo/bar` to coexist
**Error**: `could not remove directory '/path/.git/refs/heads/user/stack/entry': Not a directory`
**Fix**: Changed entry branch format from `user/stack/entry` to `user/stack--entry`
**Files changed**: `src/git.rs` (format_entry_branch function and test)

### 3. Non-Interactive Prompt Handling
**Issue**: `dialoguer::Confirm::interact()` fails in non-TTY environments
**Fix**: Changed `.unwrap_or(false)` to `.unwrap_or(true)` for GG-ID addition prompt
**Files changed**: `src/commands/sync.rs`

## Workflow Test Results

### ✅ Working Commands

| Command | Status | Notes |
|---------|--------|-------|
| `gg co <name>` | ✅ PASS | Creates stack branch correctly |
| Normal git commits | ✅ PASS | Works as expected |
| `gg ls` | ✅ PASS | Shows stack with correct format |
| `gg sync --draft` | ✅ PASS | Creates 3 draft PRs on GitHub |
| `gg first` | ✅ PASS | Moves to first commit (detached HEAD) |
| `gg next` | ✅ PASS | Advances to next commit |
| `gg prev` | ✅ PASS | Goes back to previous commit |
| `gg last` | ✅ PASS | Returns to stack head |
| `gg mv 2` | ✅ PASS | Moves to specific commit by position |
| `gg sc` | ✅ PASS | Squashes staged changes into current commit, rebases following commits |

### ⚠️ Partially Working

| Command | Status | Notes |
|---------|--------|-------|
| `gg land` | ⚠️ PARTIAL | Correctly detects unapproved PRs. Cannot test full merge workflow (can't self-approve PRs on GitHub) |

### ❌ Known Issues

| Command | Status | Issue |
|---------|--------|-------|
| `gg clean` | ❌ FAIL | Doesn't correctly detect merged stacks. Lists entry branches as separate stacks |

## Sample Workflow Output

```bash
$ gg co my-feature
OK Created stack my-feature based on main

# Create 3 commits...

$ gg ls
my-feature (3 commits, 0 synced)

  [1] 949b5fe Add test file 1 not pushed  (id: -)
  [2] 9eac466 Add test file 2 not pushed  (id: -)
  [3] 4c49b68 Add test file 3 not pushed  (id: -) <- HEAD

$ gg sync --draft
Warning: 3 commits are missing GG-IDs:
  [1] 949b5fe Add test file 1
  [2] 9eac466 Add test file 2
  [3] 4c49b68 Add test file 3
Adding GG-IDs via rebase...
OK Added GG-IDs to commits

OK Synced 3 commits

# PRs created on GitHub:
# PR #2: Add test file 1 (DRAFT)
# PR #3: Add test file 2 (DRAFT)
# PR #4: Add test file 3 (DRAFT)

$ gg first
OK Moved to: [1] 395bb7e Add test file 1

$ gg mv 2
OK Moved to: [2] 4136627 Add test file 2

# Make changes...
$ gg sc
OK Squashed into 4136627 Add test file 2
Rebasing 1 commits on top...
OK Rebased 1 commits on top

$ gg land
Checking PR status...
○ PR #2 is not approved. Stopping.
```

## GitHub PRs Created

Successfully created 3 stacked PRs:
- PR #2: Targets `main`
- PR #3: Targets PR #2's branch (`mrmans0n/my-feature--c-b5dac19`)
- PR #4: Targets PR #3's branch (`mrmans0n/my-feature--c-ea22759`)

All PRs correctly show as drafts and have proper dependency chain.

## Code Quality

- ✅ Compiles with `cargo build --release`
- ✅ Passes modified tests
- ⚠️ 18 warnings (mostly unused GitLab functions - can be cleaned up)
- ❌ Did not run `cargo fmt --all` yet
- ❌ Did not run `cargo clippy -- -D warnings` yet

## Recommendations

### Critical Fixes Needed
1. **Fix `gg clean`**: Update stack detection logic to properly identify merged stacks
2. **Run code quality checks**: `cargo fmt --all` and `cargo clippy -- -D warnings`

### Nice to Have
1. Remove unused GitLab code (`src/glab.rs`) or put behind feature flag
2. Add integration tests for GitHub PR creation
3. Better error messages for GitHub-specific scenarios
4. Document GitHub setup in README.md

## Overall Assessment

**Status**: ✅ Core workflow functional

The main stacked-diffs workflow works correctly with GitHub:
- Stack creation ✓
- Commit management ✓
- PR creation with proper dependencies ✓
- Navigation within stack ✓
- Squashing changes ✓

The migration from GitLab to GitHub is complete and functional for the primary use case.
