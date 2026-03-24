# Core Concepts

## Stacked diffs in one sentence

A stack is a series of commits where each commit is reviewed as its own PR/MR, and each PR/MR depends on the previous one.

## The git-gud model: one commit = one PR/MR

In git-gud, each commit is an "entry" in the stack:

- Entry 1 targets your base branch (for example, `main`)
- Entry 2 targets entry 1's branch
- Entry 3 targets entry 2's branch
- ...and so on

That gives reviewers small units, while preserving execution order.

## GG metadata trailers

Each stack commit carries stable trailers, for example:

```text
GG-ID: c-abc1234
GG-Parent: c-1234567
```

- `GG-ID` identifies the commit itself.
- `GG-Parent` points to the previous stack entry's `GG-ID`.
- The first stack entry has no `GG-Parent`.

Why this matters:

- Commit-to-PR/MR mappings stay stable across rebases
- Stack topology is recoverable from commit-local metadata
- `gg sync` and `gg reconcile` can auto-heal metadata drift after history edits

## Branch naming convention

git-gud uses predictable branch names:

- **Stack branch**: `<username>/<stack-name>`
- **Entry branch**: `<username>/<stack-name>--<gg-id>`

Example:

- `nacho/user-auth`
- `nacho/user-auth--c-abc1234`

This convention is what makes remote discovery (`gg ls --remote`) and reconciliation possible.

## PR/MR dependency chains

Dependency chaining is automatic during `gg sync`:

- First PR/MR targets `main` (or your configured base)
- Next PR/MR targets previous entry branch
- This continues until stack head

Result: reviewers can review from bottom to top, and `gg land` can merge safely in order.
