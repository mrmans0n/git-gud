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

## GG-IDs

Each stack commit carries a stable trailer, for example:

```text
GG-ID: c-abc1234
```

Why GG-IDs matter:

- They keep commit-to-PR/MR mappings stable across rebases
- They let git-gud identify entries by a durable ID (not just SHA)
- They make reconcile and navigation safer after history edits

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
