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

## Immutable commits

Some commits should not be casually rewritten. History-rewriting commands —
`gg squash`, `gg drop`, `gg reorder`, `gg split`, `gg absorb`, and `gg rebase` —
refuse to touch the following by default:

- **Merged PR/MR commits.** If an entry's PR/MR state is `Merged`, rewriting it
  locally produces a duplicate of something already upstream. This is the only
  rule that catches **squash-merged** PRs, because their merge commit on
  `origin/<base>` has a brand-new SHA that doesn't share ancestry with your
  local commit.
- **Base-ancestor commits.** Any commit already reachable from `origin/<base>`
  via plain merge or rebase falls in the same bucket. When no `origin/<base>`
  ref exists, gg falls back to the local base branch.

Running one of those commands on an immutable target prints a clear error like:

```text
error: cannot rewrite immutable commits (pass --force / --ignore-immutable to override):
  #2  abc1234  Fix typo in parser  (merged as !123)
  #3  def5678  Bump dependency     (already in origin/main)
```

If you genuinely want to rewrite history anyway, pass `--force` (or
`--ignore-immutable` if you prefer the longer, self-describing name). Every
rewrite command accepts both spellings. The guard still emits a warning so
scripts see that they are bypassing a safety check.

### Keeping PR state fresh

Each rewrite command runs a best-effort PR-state refresh against the
configured provider just before the immutability check, so the merged-PR rule
fires even when nothing in the session has touched provider state yet. The
refresh is silent when offline / no auth is configured, in which case the
base-ancestor rule remains the only protection. Note that base-ancestor does
**not** catch squash-merges (those produce a new SHA on `origin/<base>`), so
working offline against a repo that uses squash-merge is the one case where
you can still rewrite a "merged" commit without `--force`. A working provider
closes that gap automatically.
