---
task_id: 413
title: Investigate possible reasonable additions to gg
date: 2026-04-16
project: git-gud
---

# gg × jj × git-spice: Feature Adoption Analysis

**Subject repo:** `/Volumes/Ambrosio/repos/git-gud` (branch `main`)
**Sources:** jj-vcs.github.io/jj/latest, github.com/jj-vcs/jj, abhinav.github.io/git-spice, github.com/abhinav/git-spice, gg source tree + existing roadmap at `docs/plans/2026-03-23-stack-workflow-roadmap-plan.md`

---

## 1. Executive Summary

`gg` is already a mature stacked-diff tool built on three strong foundations: (a) **stable commit identity** via `GG-ID` trailers, (b) **linear-history discipline** enforced end-to-end, and (c) a **provider abstraction** that treats GitHub and GitLab as peers. Many of the most-cited jj and git-spice features require "shadow state" to work in a git wrapper — and gg **already pays that cost**, so adopting more of them is cheaper than for tools starting from scratch.

The existing roadmap (`docs/plans/2026-03-23-stack-workflow-roadmap-plan.md`) independently targets five features that this comparison also validates as high-value: `gg undo` + op log, `gg log` smartlog, `gg restack`, `gg reparent`, and `gg inbox`. That's a strong endorsement of the current direction.

This doc focuses on **five genuinely new additions** not in the roadmap, ordered by value-for-effort:

1. **Immutability boundary** (from jj) — block accidental edits of landed/pushed commits. **Trivial.**
2. **Revset-lite query language** (from jj) — unified `-r <expr>` for all commands that take targets. **Moderate.**
3. **Rebase-plan continuation** (from git-spice) — resumable multi-branch restacks across conflicts. **Moderate.**
4. **`gg fix` / run-formatter-across-stack** (from jj) — formatter runs against a revset with auto-descendant rebase. **Moderate.**
5. **Parallelize / sibling split** (from jj) — niche, but a natural extension of `gg split`. **Low.**

Plus four **refinements** to roadmap items informed by how jj and git-spice ship these:

- Model `gg undo` on **jj's op log**, not git's reflog — record every mutation, not just HEAD movement.
- Consider **`refs/gg/data`** (git-spice style) as a storage option alongside `.git/gg/operations/` to survive `git clone --mirror`.
- Extend `gg log` with a **DAG view across all local stacks** (jj smartlog style), not just the current stack.
- Give `gg restack`/`gg reparent` a **shared rewrite-plan type** (already called out in roadmap rule 3) that mirrors git-spice's `rebase-continue` state so interrupted ops can resume.

Explicitly **not recommended**: first-class conflicts (essentially impossible in a git wrapper), working-copy-as-commit (would break gg's model), bookmark-style non-advancing branches (conflicts with gg's one-branch-per-commit design), or a full template/format DSL (low ROI vs. adding `--format` flags per command).

---

## 2. gg's Current State (Baseline)

### 2.1 Design invariants (what *must* be preserved)

- **Linear history only** — merge commits are a hard error. Any adopted feature must respect this.
- **GG-IDs are stable across rewrites** — they live in commit trailers; every new command must preserve them.
- **One branch per commit in a stack** — `<user>/<stack>--<gg-id>` naming is how dependent PRs are created. Adopting jj's "bookmarks" model would break this.
- **Provider-agnostic** — every surface must work for both GitHub (`gh`) and GitLab (`glab`).
- **No remote rollback** — already stated in the roadmap; anything related to "undo" must respect this.

### 2.2 Current command surface (for reference)

Stack mgmt: `co`, `ls`, `clean`
Editing: `sc`/`amend`/`squash`, `split`, `reorder`, `absorb`, `drop`
Navigation: `first`, `last`, `prev`, `next`, `mv`
Sync/land: `sync`, `land`, `rebase`, `reconcile`, `continue`, `abort`
Utilities: `setup`, `lint`, `run`, `completions`

### 2.3 Already-planned (per roadmap 2026-03-23)

| Feature | Milestone |
|---|---|
| PR/MR stack breadcrumbs in descriptions | 1 |
| `gg log` smartlog view | 1 |
| `gg undo` + operation log | 2 |
| `gg restack` (repair after manual git ops) | 3 |
| `gg reparent` (intentional subtree moves) | 3 |
| `gg inbox` (cross-stack actionable list) | 4 |

---

## 3. Evaluation Framework

Each feature is scored on four axes:

- **User value**: *Low / Medium / High / Critical* — how much pain does it remove?
- **Fit with gg model**: *Clash / Neutral / Aligned* — does it respect the invariants in §2.1?
- **Implementation cost**: *Trivial / Moderate / Hard / Requires shadow state / Impossible* — rough order of magnitude.
- **Existing prior art**: tools with a working implementation to learn from.

Final rank combines value × fit, with cost as a tiebreaker.

---

## 4. New Recommendations — Ordered by Importance

### 4.1 **P0 — Immutability boundary** *(from jj)*

**What.** Configurable revset describing commits that must not be rewritten (typically `trunk() | merged-PRs() | pushed-bookmarks()`). Any mutation whose target falls inside that set is rejected unless `--force` / `--ignore-immutable` is passed.

**Why for gg.** Today a user can `gg sc` or `gg reorder` on a commit whose PR is already merged; gg will happily rewrite local history and then `gg sync` will try to push the rewrite. The immutability boundary is the cheapest, highest-ROI footgun-removal in the jj playbook. gg already knows which commits are merged (via `StackEntry::mr_state == Merged`) and which are behind `origin/<base>`, so the data is already loaded.

**How.**
- Add `[immutable]` section to `.git/gg/config.json`:
  ```json
  { "immutable": { "merged_prs": true, "base_ancestors": true, "extra_revset": null } }
  ```
- In `crates/gg-core/src/stack.rs`, add `fn is_immutable(entry: &StackEntry) -> bool` that checks: `mr_state == Merged`, or commit is an ancestor of `origin/<base>`, or matches extra_revset.
- Wrap all mutating commands (`sc`, `split`, `reorder`, `drop`, `absorb`, `rebase`) with a guard that calls this and refuses unless `--force`.
- Error message should name the offending commit and suggest the flag:
  ```
  error: commit c-abc1234 is immutable (merged as !123). Use --force to override.
  ```

**Cost.** **Trivial** — the data is already computed during `gg ls`; only need the guard + flag plumbing. ~200 LoC + tests.

**Value / fit.** **Critical / Aligned.** No model friction, huge ergonomic win, no net-new concepts for users.

**Prior art.** jj (`immutable_heads()`), git-branchless (phases concept from Mercurial).

---

### 4.2 **P1 — Revset-lite query language** *(from jj)*

**What.** A tiny DSL for selecting commits that every target-taking command accepts: `gg mv <expr>`, `gg drop <expr>`, `gg run -r <expr> -- cmd`, `gg lint -r <expr>`, `gg split --commit <expr>`. Today each command invents its own selection syntax (position, GG-ID, SHA prefix). Unifying them would be a massive UX simplification.

**Scope (lean).** A 20-function subset is enough to cover 95% of real use:
- Atoms: `@`, `@-`, `@--`, `<position>`, `<gg-id>`, `<sha>`, `trunk`, `base`, `head`, `origin/<branch>`
- Operators: `&`, `|`, `~`, `::` (ancestors), `..` (range)
- Functions: `current()`, `mutable()`, `immutable()`, `merged()`, `draft()`, `unsynced()`, `touches("path/glob")`, `author("nacho")`, `since("origin/main")`

Example uses this unlocks:
```
gg drop 'mutable() & touches("src/experimental/*")'
gg run -r 'immutable() ~ merged()' -- cargo clippy
gg sync --until 'draft()'
gg lint -r '::@-'            # everything up to current - 1
```

**Why for gg.**
- Eliminates 20+ ad-hoc flags across commands (`--until`, `--commit`, `--from`, positional targets).
- Makes the MCP surface dramatically more composable — AI agents can ask for "all unmerged commits that touch the auth module" without a bespoke tool per query shape.
- Because gg's stacks are linear and small, the implementation is **far simpler** than jj's (no DAG walk, no reachability set math at scale).

**How.**
- New crate module `crates/gg-core/src/revset.rs` containing a hand-written recursive-descent parser and evaluator against `Vec<StackEntry>`.
- The evaluator works over the stack slice in memory; it doesn't need to call `git rev-list` because stack entries are already loaded.
- Gate behind a feature flag initially; roll out by adding `-r` alongside existing positional args (deprecation path).

**Cost.** **Moderate** — ~800-1200 LoC for parser + evaluator + tests. Fully self-contained; no git2/libgit2 dependency changes.

**Value / fit.** **High / Aligned.** No conflict with any invariant; strictly additive.

**Prior art.** Mercurial revsets, jj, git-branchless (partial), Sapling.

---

### 4.3 **P1 — Rebase-plan continuation** *(from git-spice)*

**What.** git-spice's `refs/spice/data` stores a `rebase-continue` entry: the full queue of pending per-branch rebases plus any post-hooks ("then submit"). When a user hits a conflict mid-restack, resolves it, and runs `gs rebase continue`, git-spice reads that plan and picks up where it left off. gg's current `gg continue` is thinner — it only re-runs the current operation's next step.

**Why for gg.** When `gg land --all`, `gg rebase`, or the future `gg restack`/`gg reparent` hit a conflict, the *plan* of remaining work is ephemeral — lost on terminal close or a `cargo run` re-invocation. A persistent plan makes multi-step operations crash-safe and makes `gg abort` semantically cleaner.

**How.** This can **piggyback on the planned operation log**:
- The `OperationRecord` already envisioned in roadmap Task 3 can include a `pending_plan: Option<Plan>` field.
- `Plan` is a `Vec<PlannedStep>` enum: `{ Rebase, UpdateRef, Sync, Lint, ... }`.
- `gg continue` consults the latest operation's pending plan; `gg abort` clears it.
- Shared with the rewrite planner in roadmap Task 4 (restack) — same type serves both purposes.

**Cost.** **Moderate** — estimated at +300 LoC on top of the already-planned operation log. The saved complexity in `restack`/`reparent` may net out even cheaper.

**Value / fit.** **High / Aligned.** Makes roadmap items more robust without changing their scope.

**Prior art.** git-spice (`refs/spice/data/rebase-continue`), jj's op log (though jj solves it by making rebases never fail).

---

### 4.4 **P2 — `gg fix` / run-formatter-across-stack** *(from jj)*

**What.** `jj fix` runs a configured formatter (rustfmt, prettier, black) on every mutable commit in a revset and rebases descendants onto the reformatted trees. Net effect: "fix formatting drift across my entire stack" in one command.

**Current gg state.** `gg run --amend -- <cmd>` already exists and walks the stack applying changes. The gap is:
- no **parallel execution** (only read-only mode has `--jobs`)
- no **atomic rollback** if a middle commit fails
- no **config integration** (formatter is ad-hoc on each invocation)

**Why.** Rust users rely on `cargo fmt`; JS teams on `prettier`. Running these across a stack today means either `gg run --amend -- cargo fmt` (works, but slow, no rollback) or raw `git rebase -x`. A first-class `gg fix` with a config block is a small wrapper around `gg run` that earns a dedicated name:

```json
{
  "fix": {
    "tools": [
      { "command": ["cargo", "fmt", "--all"], "include": "**/*.rs" },
      { "command": ["prettier", "--write", "{}"], "include": "**/*.{ts,js}" }
    ]
  }
}
```

```
gg fix                       # run all tools on entire stack
gg fix -r '@--..@'          # last two commits only (pairs with revsets)
gg fix --tools cargo-fmt     # just one tool
```

**How.** `crates/gg-core/src/commands/fix.rs` — a ~150-line wrapper over the existing `run_command_on_commits` logic in `run.rs`, plus config loading.

**Cost.** **Moderate** — mostly config + orchestration; the hard part (walking stack + amending) already exists.

**Value / fit.** **Medium / Aligned.** Saves real time for users who hit formatter drift across review iterations.

**Prior art.** `jj fix`; GitHub's `pre-commit run --all-files` across a branch (but not across commits).

---

### 4.5 **P3 — Parallelize / sibling split** *(from jj)*

**What.** `jj parallelize A B C` turns A → B → C into three siblings of the common parent (useful when three commits are truly independent and reviewers want to parallelize them). `jj split -p` makes the new commit a sibling rather than a child.

**Why for gg.** Niche but real: if you have five commits that each edit an orthogonal file and you want five independent PRs (no stacking), today you'd need `gg co` into five separate stacks and `git cherry-pick`. A `gg detach <revset>` or `gg split --parallel` would express this natively.

**Caveat.** This produces *multiple stacks*, not a stack. It conflicts slightly with gg's implicit "current stack is singular" model. A minimal version would spawn new stacks named `<current>--sibling-1`, `<current>--sibling-2`, etc., and leave the user to rename.

**Cost.** **Moderate** — needs new-stack creation + commit relocation + GG-ID regeneration rules.

**Value / fit.** **Low / Neutral.** Don't prioritize until other items are shipped.

**Prior art.** `jj parallelize`, `jj split -p`.

---

## 5. Validations and Refinements to the Existing Roadmap

### 5.1 `gg undo` — model on jj's op log, not git's reflog

The roadmap (Task 3, Step 1) already proposes `.git/gg/operations/` with pre/post state snapshots. **This is the right direction.** Two specific refinements from jj:

- **Record *every* gg-invoked mutation, not just destructive ones.** jj records `fetch`, `bookmark_move`, even `describe`. The op log's value compounds with coverage; a partial log is a partial safety net.
- **Surface time-travel, not just undo.** jj has `jj --at-op=<id>` that reads the repo as-of an op. gg could add `gg ls --at-op <id>` to show what the stack looked like before a change, without restoring. Very cheap once the op log exists.
- **Concurrency benefit.** jj's op log also serves as a lock-free concurrency layer. gg doesn't need this today, but designing the op log with optimistic append-only semantics keeps the door open.

### 5.2 Storage: consider `refs/gg/data` *alongside* `.git/gg/operations/`

git-spice stores all its state in a single git ref (`refs/spice/data`), whose tree contains JSON blobs. The advantage: state travels with `git clone --mirror`, `git push refs/*`, and is visible via `git log --patch refs/spice/data` for debugging.

`.git/gg/operations/` as planned has different tradeoffs:
- ✅ Simpler (just files)
- ✅ Never pushed by accident
- ❌ Lost on a fresh clone
- ❌ Not introspectable with git tooling

Recommend: keep `.git/gg/operations/` as the default, but consider a `refs/gg/data` mirror mode for users who explicitly opt in (teams that want the op log to survive machine migration). Low priority; document as a future option.

### 5.3 `gg log` — extend to cross-stack DAG view

The roadmap's `gg log` (Task 2) is stack-scoped (and `--all` lists them all). jj's smartlog is more ambitious: it shows a single DAG covering trunk, all mutable heads, and their relationships, collapsing uninteresting ancestors. For gg, this would let a user see: "here are my 3 in-flight stacks, here's which one has conflicts, here's which one's PRs are merged."

This is a natural **future evolution** of `gg log --all`, not a change to Milestone 1. Flag it for Milestone 4 alongside `gg inbox` — they're really two faces of the same feature ("what am I working on across this repo?").

### 5.4 Shared rewrite-plan type (roadmap Rule 3) — make it the op log's `pending_plan`

Rule 3 of the roadmap already says to build a `StackRewritePlan` before implementing `restack`/`reparent`. Suggest folding this into the op log: every operation's `OperationRecord` carries its plan; `gg continue` reads the active op's plan. This makes conflict continuation, dry-run, and undo all share one type — strictly less code than keeping them separate.

### 5.5 Breadcrumbs — steal git-spice's navigation comment

Roadmap Task 1 plans breadcrumbs inside the PR *description*. git-spice also posts a **navigation comment** on each PR, which updates in-place when the stack shifts. Description edits can trigger "force-push" notifications for reviewers who watch descriptions; comments are less noisy. Recommend: evaluate both during Task 1 design. gg's existing `<!-- gg:managed:start/end -->` marker logic already supports a comment-based variant.

---

## 6. Features to Explicitly NOT Adopt

### 6.1 First-class conflicts (jj)

jj stores conflicts as structured data inside commits, which is why its rebases never fail. Replicating this in a git wrapper is essentially impossible: git's object model has no room for conflicted blobs, third-party tools would see broken files, and IDEs would misrender. Even git-branchless doesn't attempt this. **Skip forever.**

### 6.2 Working-copy-as-commit (jj)

jj's "every edit is part of a commit" UX requires auto-snapshot on every command. Porting this to gg would mean `gg` commands auto-commit your working tree — a massive behavior change that conflicts with gg users' mental model of "commit when I mean to." **Skip.**

### 6.3 Bookmarks (jj)

jj renamed branches to "bookmarks" that don't auto-advance with commits. gg's entire stacked-PR model relies on one named branch per commit (`<user>/<stack>--<gg-id>`). Adopting non-advancing bookmarks would break every existing gg workflow. **Skip.**

### 6.4 Change ID separate from GG-ID (jj)

jj has *both* a change ID (stable) and a commit ID (content hash). gg has `GG-ID` + git SHA, which is the same structural pattern. Don't add a third ID. **Already done.**

### 6.5 Fileset DSL (jj)

jj has a whole language for selecting files (`glob:`, `root:`, `file:`, `~`, `&`). Git already has pathspecs (`:(exclude)`, `:(glob)`) and gg doesn't have a surface that needs file selection today (no per-file revset operations). Revisit only if `gg fix --paths` or similar ships. **Defer indefinitely.**

### 6.6 Template DSL (jj)

jj has a Sapling-style template language for `jj log` output. For gg, the payoff is much smaller: users who need structured output have `--json`; human output can be tuned with a handful of `--format=` options. A full DSL is massive surface area for marginal benefit. **Skip.**

### 6.7 Full offline-first auth (git-spice)

git-spice's `gs auth login` supports OAuth device flow, GitHub App, PAT, keyring, credential manager, env vars. gg delegates to `gh`/`glab` which already handle all of this. Don't build a parallel auth system. **Skip.**

### 6.8 `branch fold` / `branch track` (git-spice)

These are meaningful for git-spice because branches are the unit of tracking. In gg, commits are the unit (via GG-IDs) and every commit already gets a branch on sync — there's no separate "start tracking" step. The equivalent gg surface is `gg reconcile`, which already handles "adopt this stack that was pushed without gg." **Already done.**

---

## 7. Architectural Observations

### 7.1 gg has already paid the shadow-state tax

The biggest structural argument against adopting jj features in most git wrappers is: "you'd need a shadow state layer, which is huge." gg already has one:

- `.git/gg/config.json` per-repo state
- `GG-ID` / `GG-Parent` commit trailers
- (planned) `.git/gg/operations/` op log

Every feature in §4 above can hang off this existing infrastructure. The cost curve is flatter for gg than for a tool starting fresh.

### 7.2 Commit trailers are a quiet superpower

gg's decision to put `GG-ID` in commit trailers (not a sidecar file or a custom ref) makes it **uniquely robust to raw git usage**. A user can `git rebase -i`, `git cherry-pick`, `git commit --amend` all day long and GG-IDs follow the commits. git-spice's `refs/spice/data` by contrast goes stale the moment a user reaches for raw git.

Any new feature should preserve this property: **state that describes a specific commit should live on the commit**, not in a sidecar. The op log is an exception (it describes events, not commits) and belongs in a sidecar.

### 7.3 The MCP surface amplifies the case for revsets

gg-mcp currently exposes 19 tools, many of which take ad-hoc selection params (position, GG-ID, SHA). An LLM using MCP has to learn each tool's selection idiom. A single `revset` parameter on every target-taking tool would collapse that surface dramatically and align with how agents think about sets ("all unmerged commits touching X"). This is an under-appreciated reason to prioritize §4.2.

### 7.4 Don't grow another forge

The provider abstraction in `provider.rs` already handles GitHub and GitLab well. Adding Bitbucket (git-spice has it) or Gitea would require the same discipline but offers diminishing returns. Skip unless there's explicit user demand.

---

## 8. Consolidated Priority List

### Tier 0 — validated in roadmap, ship as planned

1. PR/MR breadcrumbs (Milestone 1)
2. `gg log` (Milestone 1, extend to cross-stack view per §5.3)
3. `gg undo` + op log (Milestone 2, model on jj per §5.1, fold rewrite-plan into it per §5.4)
4. `gg restack` (Milestone 3)
5. `gg reparent` (Milestone 3)
6. `gg inbox` (Milestone 4)

### Tier 1 — new, high value, low cost

7. **Immutability boundary** (§4.1) — trivial, enormous ergonomic win. Ship anytime; could precede or parallel Milestone 1.
8. **Revset-lite DSL** (§4.2) — moderate cost, but unlocks compounding UX gains for CLI and MCP. Suggest targeting *before* Milestone 3 so `restack`/`reparent` can use it natively.

### Tier 2 — new, medium value

9. **Rebase-plan continuation** (§4.3) — free rider on the op log; ship with Milestone 2.
10. **`gg fix`** (§4.4) — small wrapper over existing `gg run --amend`; add in a quiet release.

### Tier 3 — defer

11. **Parallelize / sibling split** (§4.5) — niche.
12. Storage-in-git-ref mirror mode (§5.2) — opt-in only, low urgency.

### Not adopted

See §6 for the full Do-Not-Adopt list.

---

## 9. Open Questions

1. **Revset syntax: jj-compatible or bespoke?** jj's syntax is battle-tested but verbose (`::@-`, `immutable_heads()`). A simpler dialect (e.g., `up-to @-`, `not immutable`) would be more approachable but incompatible with jj documentation users already know. Recommend: jj-compatible subset, with aliases in config for ergonomic shortcuts.

2. **Immutability boundary: `--force` vs. `--ignore-immutable`?** jj uses `--ignore-immutable`, which is verbose and self-describing. git uses `--force`, which is terse but overloaded. Recommend `--force` for consistency with git muscle memory, `--ignore-immutable` as a long alias for scriptability.

3. **Should `gg fix` exist as a separate command or as a preset for `gg run --amend`?** If the config block is small (~3 lines), it's almost free to promote to a command. If users want more sophisticated tool selection, it earns its own command. Decide based on prototype config shape.

4. **Op log: retention policy?** jj keeps op log forever. git-branchless has configurable retention. For gg, what's the default — last 100 ops? 30 days? All? Needs a sizing study against real stack workflows.

5. **Is `gg inbox` really distinct from `gg log --all`?** The roadmap treats them as separate features (Milestone 1 vs 4). If `gg log --all` grows the smartlog DAG view (§5.3), it starts to subsume inbox's "which stacks need attention" role. Reconsider the split before Milestone 4 starts.

---

## 10. References

**jujutsu (jj):**
- https://jj-vcs.github.io/jj/latest/
- https://github.com/jj-vcs/jj
- https://jj-vcs.github.io/jj/latest/git-compatibility/ (interop frictions)
- https://jj-vcs.github.io/jj/latest/revsets/ (revset reference)

**git-spice:**
- https://abhinav.github.io/git-spice/
- https://github.com/abhinav/git-spice
- https://abhinav.github.io/git-spice/guide/internals/ (`refs/spice/data` layout)
- https://abhinav.github.io/git-spice/cli/reference/

**gg internal:**
- `/Volumes/Ambrosio/repos/git-gud/docs/plans/2026-03-23-stack-workflow-roadmap-plan.md`
- `/Volumes/Ambrosio/repos/git-gud/CLAUDE.md`
- `/Volumes/Ambrosio/repos/git-gud/README.md`
- `/Volumes/Ambrosio/repos/git-gud/crates/gg-core/src/stack.rs` (GG-ID model)
- `/Volumes/Ambrosio/repos/git-gud/crates/gg-core/src/provider.rs` (provider abstraction)

**Related tools (prior art cited above):**
- git-branchless (https://github.com/arxanas/git-branchless) — op log, smartlog, revsets
- git-absorb (https://github.com/tummychow/git-absorb) — already used by gg
- Sapling (https://sapling-scm.com/) — Meta's VCS; smartlog, templates
- Graphite CLI (https://graphite.dev/) — commercial stacked-PR tool
