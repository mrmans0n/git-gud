# `gg split -i` — Hunk-level Splitting Design

**Date:** 2026-03-13
**Author:** Ambrosio (AI) + Nacho López
**Status:** Approved
**Depends on:** `gg split` (file-level, merged in PR #207)

## Summary

Extend `gg split` with hunk-level granularity. Users can select individual diff hunks (not just whole files) to include in the new commit. Uses a `git add -p` style sequential prompt.

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| UI style | `git add -p` sequential prompt | Familiar, no new deps, TUI later |
| Activation | `-i` flag + auto for single-file commits | Explicit control + resolves edge case |
| Actions | y/n/q/a/d/s | Standard set, `a`/`d` save time, `e` deferred |

## Activation Rules

1. `gg split -i` — Forces hunk mode for all files in the commit
2. `gg split -i file1.rs file2.rs` — Hunk mode only for specified files
3. `gg split` on a single-file commit — Auto-enters hunk mode (instead of current error)
4. `gg split` on a multi-file commit — File mode (unchanged from PR #207)

## Interactive Hunk Prompt

### Display Format

```
--- a/src/auth.rs
+++ b/src/auth.rs
@@ -10,6 +10,12 @@ fn authenticate(user: &str) -> bool {
+    // Validate token
+    if token.is_empty() {
+        return false;
+    }
+    let valid = check_token(token);
+    valid

Include this hunk? (y)es / (n)o / (a)ll file / (d)one file / (s)plit / (q)uit / (?)help
```

### Actions

| Key | Action | Description |
|-----|--------|-------------|
| `y` | Yes | Include this hunk in the new commit |
| `n` | No | Skip this hunk (stays in remainder) |
| `a` | All | Include all remaining hunks from this file |
| `d` | Done file | Skip all remaining hunks from this file |
| `s` | Split | Split this hunk into smaller hunks |
| `q` | Quit | Stop; all remaining hunks stay in remainder |
| `?` | Help | Show this help |

### Hunk Splitting (`s`)

When a hunk contains multiple logical changes separated by unchanged lines, `s` breaks it into smaller hunks and presents each one individually. If the hunk cannot be split further (contiguous changes), print "This hunk cannot be split further" and re-prompt.
