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

## GG-IDs and GG-Parent trailers

Each stack commit carries stable trailers that persist across rebases:

```text
GG-Parent: c-1234567
GG-ID: c-abc1234
```

**GG-ID** identifies the commit itself — it keeps commit-to-PR/MR mappings stable across rebases, lets git-gud identify entries by a durable ID (not just SHA), and makes reconcile and navigation safer after history edits.

**GG-Parent** records the GG-ID of the previous entry in the stack. The first entry has no `GG-Parent`. This trailer encodes the stack's topology directly in commit metadata, so higher-level tools (CLI, MCP, agents) can reconstruct the dependency chain from commits alone — without relying on PR description breadcrumbs or remote state.

Both trailers are managed automatically. `gg sync`, `gg reconcile`, `gg reorder`, `gg drop`, and `gg split` all normalize the trailer chain after any stack changes.

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
