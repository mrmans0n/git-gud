# Stack Verification Audit

This document tracks which commands should require being on a stack and their current implementation status.

## Summary of Changes

✅ **Fixed**: `gg squash` now properly requires being on a stack  
✅ **Fixed**: `gg rebase` now works on any branch when a target is provided (WIP found in repo)  
✅ **Added**: Comprehensive tests for stack verification  

## Commands That REQUIRE Stack Verification

| Command | File | Status | Implementation |
|---------|------|--------|----------------|
| `gg squash` / `gg sc` / `gg amend` | squash.rs | ✅ **FIXED** | Now calls `Stack::load(&repo, &config)?` at line 22 |
| `gg absorb` | absorb.rs | ✅ Correct | Uses `Stack::load(&repo, &gg_config)?` |
| `gg nav` (first/last/next/prev/mv) | nav.rs | ✅ Correct | All calls use `Stack::load()?` |
| `gg sync` | sync.rs | ✅ Correct | Uses `Stack::load(&repo, &config)?` |
| `gg land` | land.rs | ✅ Correct | Uses `Stack::load(&repo, &config)?` |
| `gg reorder` | reorder.rs | ✅ Correct | Uses `Stack::load(&repo, &config)?` |
| `gg lint` | lint.rs | ✅ Correct | Uses `Stack::load(&repo, &config)?` |
| `gg reconcile` | reconcile.rs | ✅ Correct | Uses `Stack::load(&repo, &config)?` |

## Commands That Should NOT Always Require Stack

| Command | File | Status | Behavior |
|---------|------|--------|----------|
| `gg co` (checkout) | checkout.rs | ✅ Correct | Creates/switches stacks |
| `gg ls` | ls.rs | ✅ Correct | Lists stacks (uses `.ok()`) |
| `gg clean` | clean.rs | ✅ Correct | Cleans up stacks |
| `gg setup` | setup.rs | ✅ Correct | Configuration wizard |
| `gg completions` | completions.rs | ✅ Correct | Shell completions |
| `gg rebase` | rebase.rs | ✅ **FIXED** | Requires stack only if no target provided; works on any branch when target is specified |

## Error Handling

The `GgError::NotOnStack` error exists in `src/error.rs` with a helpful message:
```rust
#[error("Not on a stack branch. Use `gg co <stack-name>` to create or switch to a stack.")]
NotOnStack,
```

When `Stack::load()` is called with the `?` operator, it automatically propagates this error.

## Changes Made

### 1. Fixed `src/commands/squash.rs`

**Before** (line 35):
```rust
let stack_result = Stack::load(&repo, &config);
let needs_rebase = if let Ok(ref stack) = stack_result {
    stack.current_position.map(|p| p < stack.len() - 1).unwrap_or(false)
} else {
    false
};
```

**After** (line 22):
```rust
// Verify we're on a stack
let stack = Stack::load(&repo, &config)?;

// ... later ...
let needs_rebase = stack
    .current_position
    .map(|p| p < stack.len() - 1)
    .unwrap_or(false);
```

This ensures `gg squash` fails with a clear error when not on a stack.

### 2. Enhanced `src/commands/rebase.rs` (WIP found in repo)

**Before**:
```rust
let stack = Stack::load(&repo, &config)?;
let target_branch = target.unwrap_or_else(|| stack.base.clone());
```

**After**:
```rust
// If no target provided, we need to be on a stack to get the base branch
let target_branch = if let Some(t) = target {
    t
} else {
    // No target provided, must be on a stack
    let stack = Stack::load(&repo, &config)?;
    stack.base.clone()
};
```

This allows `gg rebase <target>` to work on any branch, while `gg rebase` (no target) requires being on a stack.

### 3. Added Tests (WIP found in repo)

Three comprehensive integration tests were added:
- `test_squash_requires_stack` - Verifies squash fails when not on a stack
- `test_nav_requires_stack` - Verifies navigation commands require a stack  
- `test_rebase_without_stack_requires_target` - Verifies rebase behavior with/without target

## Test Results

All tests pass:
```
✓ Unit tests: 112 passed
✓ Integration tests: 66 passed
```

## Pre-commit Checklist

- ✅ `cargo fmt --all` - Code formatted
- ✅ `cargo clippy --all-targets --all-features -- -D warnings` - No warnings
- ✅ `cargo test --all-features` - All tests pass

## Notes

- The fix follows the existing pattern: use `Stack::load(&repo, &config)?` to enforce stack requirement
- Error messages are user-friendly and actionable
- The `gg rebase` enhancement maintains backward compatibility while adding flexibility
- All changes maintain the project's code quality standards
- Found WIP changes in the repo that already addressed rebase.rs and added tests - integrated these changes
