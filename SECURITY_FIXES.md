# Security Fixes - Critical Issues

This document summarizes the critical security fixes implemented to address issues identified in the safety audit.

## Fixed Issues

### 1. ✅ Force-push retry bypasses --force-with-lease safety (CRITICAL)

**File**: `src/git.rs` - `push_branch()` function

**Problem**: When `--force-with-lease` failed due to stale info, the code automatically retried with `--force`, completely bypassing the safety check and potentially overwriting other people's work.

**Fix**: 
- Removed automatic `--force` retry
- Added user confirmation prompt using `dialoguer::Confirm` in interactive mode
- In non-interactive mode, fails with a helpful error message instructing the user to fetch and review changes
- Only proceeds with `--force` if user explicitly confirms

**Safety Impact**: Prevents accidental data loss from force-pushing over someone else's changes.

---

### 2. ✅ Config file race conditions (CRITICAL)

**File**: `src/config.rs`

**Problem**: Multiple concurrent `gg` processes could read-modify-write the config file simultaneously, causing last-write-wins corruption and lost MR mappings.

**Fix**:
- Added `fs2` crate dependency for file locking
- Implemented `acquire_lock()` method with timeout-based retry logic
- Added shared locks for reading (multiple readers allowed)
- Added exclusive locks for writing (single writer)
- Uses atomic write pattern (write to temp file, then rename)
- 5-second timeout with helpful error messages

**Safety Impact**: Prevents config corruption when multiple terminals run gg commands simultaneously.

---

### 3. ✅ Recursive sync during GG-ID addition (CRITICAL)

**File**: `src/commands/sync.rs`

**Problem**: When adding GG-IDs triggers a rebase, the code would recursively call `run()` without checking if the rebase completed successfully. If the rebase had conflicts, this could lead to undefined state.

**Fix**:
- Added `is_rebase_in_progress()` check before recursive call
- If rebase is still in progress, returns clear error message instructing user to:
  - Resolve conflicts with `git rebase --continue` or `gg continue`
  - Run `gg sync` again after rebase completes
- Only recurses if rebase completed successfully

**Safety Impact**: Prevents cascading errors and undefined state when rebase conflicts occur during GG-ID addition.

---

### 4. ✅ Clean command deletes remote branches without full merge verification (HIGH)

**File**: `src/commands/clean.rs`

**Problem**: The clean command could delete remote branches without fully verifying that all commits are reachable from the base branch, especially in edge cases.

**Fix**:
- Added `verify_commits_reachable()` function that walks the commit graph
- Verifies all commits in the stack are reachable from the base branch
- Uses `git2::Revwalk` to check commit ancestry
- Conservative approach: if verification fails, considers stack not merged
- Additional safety layer on top of PR/MR status checks

**Safety Impact**: Prevents deletion of branches with unmerged commits, even if PR status API is incorrect or stale.

---

### 5. ✅ Operation-level locking for concurrency (HIGH)

**Files**: `src/git.rs`, `src/commands/sync.rs`, `src/commands/rebase.rs`, `src/commands/land.rs`, `src/commands/clean.rs`

**Problem**: No mechanism prevented concurrent gg operations in different terminals, leading to race conditions, force-push conflicts, and config corruption.

**Fix**:
- Added `acquire_operation_lock()` function in `src/git.rs`
- Creates exclusive lock file at `.git/gg/operation.lock`
- All mutating commands (sync, rebase, land, clean) acquire lock at start
- Uses `fs2::FileExt::try_lock_exclusive()` for platform-independent locking
- 10-second timeout with informative error messages
- Lock automatically released when `OperationLock` is dropped
- Writes operation info (operation name, PID) to lock file for debugging

**Safety Impact**: Prevents all concurrent operation conflicts by ensuring only one gg command runs at a time per repository.

---

### 6. ✅ Auto-stash functionality (HIGH)

**File**: `src/commands/rebase.rs`

**Problem**: Rebase required a clean working directory but provided no auto-stash option. Users had to manually stash/unstash, risking forgotten stashed changes.

**Fix**:
- Removed hard requirement for clean working directory
- Automatically stashes uncommitted changes before rebase with message "gg-rebase-autostash"
- Restores stash after successful rebase
- On rebase conflict, preserves stash and informs user changes will be restored after resolution
- On other errors, attempts to restore stash immediately
- Clear user messaging at each step

**Safety Impact**: Prevents lost uncommitted work and reduces cognitive load on users.

---

## Testing

All fixes have been validated:
- ✅ Code compiles successfully
- ✅ `cargo fmt --all` - Code is properly formatted
- ✅ `cargo clippy --all-targets --all-features -- -D warnings` - No clippy warnings
- ✅ `cargo test --all-features` - All 165 tests pass (110 unit tests + 55 integration tests)

## Dependencies Added

- `fs2 = "0.4"` - Cross-platform file locking for config and operation locks

## Code Style

All changes follow the project's coding conventions:
- English comments and messages
- Uses existing patterns from codebase (e.g., `dialoguer::Confirm` for prompts)
- Consistent error messaging with actionable guidance
- Follows Rust idioms and best practices

## Recommendations

These fixes address the most critical safety issues. Additional improvements for future consideration:
1. Add integration tests for concurrent operations
2. Implement transaction log for land operations (rollback on failure)
3. Add `gg doctor` command for diagnosing issues
4. Implement backup/undo system for destructive operations
5. Increase GG-ID length from 7 to 10 characters to reduce collision probability
