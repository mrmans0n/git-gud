# Mid-stack commit insertion

**Issue:** [#348](https://github.com/mrmans0n/git-gud/issues/348) — `gg ls` loses commits made in the middle of the stack
**Date:** 2026-06-09
**Status:** Approved (design)

## Problem

A user navigates into the middle of a stack and commits:

```
gg co -b main testing
git commit --allow-empty -m one
git commit --allow-empty -m two
gg mv 1                          # detached HEAD at "one"
git commit --allow-empty -m inserted
gg ls
```

`gg ls` shows only `one` and `two`, with HEAD apparently on `two`. The `inserted`
commit appears lost.

### Why it happens

- A stack's commits are computed by walking the **parent chain from the stack
  branch ref** (`<user>/<stack>`), stopping at the base. There is no stored commit
  list (`git.rs:415`, `git.rs:437`).
- `gg mv 1` checks out a **detached HEAD** at commit `one` and saves a nav context
  to `.git/gg/current_stack` as `branch | position | original_oid`
  (`nav.rs:245`, `stack.rs:436`).
- `git commit -m inserted` lands the new commit on top of `one` at detached HEAD.
  The branch ref `testing` still points at `two`, so `inserted` is **not reachable
  from the tip**. `gg ls` walks `two → one → base` and never sees it.

The commit is **not actually lost** — it is at HEAD and in the reflog. Crucially,
`nav.rs::check_and_rebase_if_modified` (`nav.rs:285`) already handles this exact
shape: had the user run `gg next` / `gg last`, it would have run
`git rebase --onto inserted one testing` and produced the desired
`one → inserted → two`. Those commands recover it because they call the helper;
`gg ls` does not.

So this is not a missing mechanism. It is two gaps:

1. **`gg ls` / bare `gg` is read-only** and never triggers integration, so it shows
   a misleading "the commit vanished" state when the data is safe.
2. When nav *does* integrate, it **moves HEAD to the stack head**, whereas the
   issue wants HEAD to **stay on the just-inserted commit**.

## Goals

- `gg ls` / bare `gg` must never present an inserted commit as lost.
- There must be a single, discoverable command that folds an inserted commit into
  the stack and leaves HEAD on it.
- Reuse the existing rebase/conflict machinery; add no new conflict handling.

## Non-goals

- Detecting inserts made via a **manual** `git checkout <sha>` (no nav context).
  v1 only handles inserts after a `gg`-driven navigation. Structural detection is a
  possible follow-up.
- Making `gg ls` (or any read command) mutate history.
- Assigning GG-IDs to the inserted commit during integration (left to `sync` /
  `reconcile`, as for any new commit).

## Design

### 1. Detection (shared helper)

Reuse the existing nav context — no new state. A commit was inserted at a detached
mid-stack HEAD when **all** of:

1. A nav context exists in `.git/gg/current_stack`.
2. HEAD is detached and `HEAD_oid != original_oid`.
3. HEAD's commit is a **descendant** of `original_oid` (committed *on top of* the
   navigated commit — distinct from amending it in place).
4. The branch tip is not yet reachable from HEAD (the upper commits have not been
   moved over).

This naturally covers **multiple** inserted commits (a whole chain on top of
`original_oid`), since detection only requires that HEAD descends from
`original_oid`.

The helper is extracted so both `ls` (report-only) and `restack` (integrate) use
the same definition.

### 2. `gg ls` reporting (read-only)

When detection fires, `ls` / bare `gg` still walks the branch tip as today, but
appends a clear callout. The orphan is *shown*, visually separated from the
integrated stack:

```
testing (2 commits, 0 synced)

  [1] e1de9de one       not pushed  (id: -)
  [2] b9ba3d2 two       not pushed  (id: -)

  ⚠ Un-integrated commit at HEAD (detached):
      3fb873d inserted  — sits on top of [1] one
    Run `gg restack` to fold it into the stack (it will become [2]).
```

- **No mutation.** `ls` stays a pure read.
- The orphan is displayed with its SHA and the position it sits on, so the user
  sees the work is safe.
- `--json` output gains an `unintegrated_commits` array (`oid`, `subject`,
  `sits_on_position`) alongside the normal `commits`, so agents/scripts see it too.

Scoped to `ls` / bare `gg` for v1. The detection helper can be reused to surface the
same callout from other read paths (e.g. `gg log`) later.

### 3. `gg restack` integration

`gg restack` gains an integration step that runs **before** its existing GG-Parent
reattachment logic:

1. **Detect** inserted commit(s) via the Section 1 helper. If none, `restack`
   behaves exactly as today.
2. **Integrate** via existing machinery:
   `git rebase --onto <HEAD_oid> <original_oid> <branch>`. The upper commits
   (`two`, …) replay on top of the inserted chain. The inserted commit's own oid is
   unchanged (it is the new base of the rebase), so we retain it.
3. **Stay in place:** re-checkout the inserted commit as detached HEAD and rewrite
   the nav context to its new position (`saved_position + <#inserted>`). HEAD ends
   on `inserted`.
4. **GG-IDs:** the inserted commit has none. Leave it blank — matches the issue's
   expected output (`id: -`); IDs are assigned lazily on `sync` / `reconcile`. The
   one required fix: restack's guard that *all* commits must already have GG-IDs
   (`restack.rs:70`) must not reject a freshly-integrated orphan. Integration runs
   first; GG-Parent reattachment then proceeds over the now-linear stack,
   tolerating the new commit's missing ID.
5. **Amend-in-place too:** restack also picks up an *amended* mid-stack commit (HEAD
   replaces `original_oid` rather than descending from it), integrating it and
   staying in place — making `restack` the single "integrate my detached-HEAD
   changes, keep my position" entry point.
6. **Conflicts:** if the `rebase --onto` conflicts, reuse the existing path — print
   the `gg continue` / `gg abort` guidance and return `RebaseConflict`. No new
   conflict machinery.

Net effect of `gg mv 1; git commit -m inserted; gg restack`: `one → inserted → two`,
HEAD on `inserted` — the issue's exact expected output.

## Affected code

- `crates/gg-core/src/stack.rs` / `nav.rs` — extract the detection helper from the
  logic currently inside `check_and_rebase_if_modified` (`nav.rs:285`).
- `crates/gg-core/src/commands/ls.rs` — read-only detection + callout + JSON field.
- `crates/gg-core/src/commands/restack.rs` — integration step before reattachment;
  relax the all-IDs-present guard (`restack.rs:70`); stay-on-commit + nav-context
  rewrite.

## Testing

Integration tests (temp repos, `run_gg` / `run_git`) covering:

- `gg mv 1; git commit; gg ls` → reports the un-integrated commit; stack still shows
  2 commits; exit success; no mutation.
- `gg mv 1; git commit; gg restack` → stack is `one → inserted → two`; HEAD detached
  on `inserted`; `gg ls` afterward shows 3 commits with HEAD on `[2]`.
- Multiple inserted commits at the middle → all integrated in order.
- Amend-in-place at detached middle HEAD + `gg restack` → amended commit integrated,
  HEAD stays on it.
- `gg restack` with no orphan → unchanged behavior (regression guard).
- Conflict during integration → `RebaseConflict`, `gg continue` / `gg abort`
  guidance shown.
- `gg ls --json` → `unintegrated_commits` array populated/empty as appropriate.

## Docs & skill updates

- `docs/src` — document the insert-in-the-middle workflow (`gg mv N; git commit;
  gg restack`) and the `ls` callout.
- `skills/gg/SKILL.md` and `skills/gg/reference.md` — note the workflow, the `ls`
  callout, the `unintegrated_commits` JSON field, and that `restack` integrates
  detached-HEAD commits while staying in place.
