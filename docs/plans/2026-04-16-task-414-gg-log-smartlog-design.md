---
task_id: 414
title: gg log smartlog view — design
date: 2026-04-16
project: git-gud
phase: design
prior_art:
  - docs/research/414-gg-log-smartlog-backlog.md          # backlog grooming (authoritative file list)
  - docs/plans/2026-03-23-stack-workflow-roadmap-plan.md  # Milestone 1, Task 2 — original spec
  - docs/research/413-investigate-possible-reasonable-additions-to-gg.md  # §5.3 motivation
---

# Task #414 — `gg log` smartlog view (Design)

## 1. Overview

Add a first-class `gg log` subcommand that prints the **current stack** as a
smartlog-style graph: one row per commit with a graph glyph, short SHA, GG-ID,
title, and PR/MR state — with a clear current-commit marker and a versioned
`--json` mode. Cross-stack DAG rendering is deferred to a future task (the
`--all` flag name is reserved but not implemented in v1).

The data layer is already complete. `Stack::load` + `Stack::refresh_mr_info`
already produce every field this command needs, and `StackEntry::status_display`
already formats the PR/MR status string. The new code is:

1. A renderer (`crates/gg-core/src/commands/log.rs`)
2. A JSON wrapper added to `crates/gg-core/src/output.rs`
3. A clap subcommand in `crates/gg-cli/src/main.rs`
4. An MCP tool in `crates/gg-mcp/src/tools.rs`
5. Docs + skill updates

## 2. Scope

### In scope (v1)

| Surface                | v1 behaviour                                         |
|------------------------|------------------------------------------------------|
| `gg log`               | Render current stack, base → head, with graph glyphs |
| `gg log --json`        | Emit `LogResponse { version, log: LogJson }`         |
| `gg log -r / --refresh`| Force refresh PR/MR state before rendering           |
| Empty-stack message    | Low-noise hint, same shape as `gg ls`                |
| Docs                   | `docs/src/commands/log.md`, `SUMMARY.md`             |
| Skill                  | `skills/gg/SKILL.md`, `skills/gg/reference.md`       |
| MCP tool               | `stack_log` wrapping JSON output                     |

### Out of scope (v1)

- Cross-stack DAG / smartlog-across-all-stacks — reserve `--all` name, do not
  ship a degenerate "list each stack in sequence" placeholder.
- `--hidden` — no hidden-commit concept exists yet (no op-log).
- Revset-based filtering (`-r <revset>`) — separate future feature.
- Template / format DSL — explicitly rejected upstream.
- Extracting a shared `print_stack` helper between `ls` and `log`.

## 3. Architecture decisions

### 3.1 Renderer forked from `ls`, not unified

**Decision.** `log.rs` owns its own text renderer. `ls::show_stack` stays as-is.

**Why.** `gg ls` is a status-table view keyed on `[N]` position prefixes;
smartlog is a graph view where the leading column is a glyph that later
generalises to a DAG. Unifying them today would either (a) bloat `show_stack`
with a layout mode flag, or (b) introduce a premature abstraction the two
callers would fight over. The data layer is already shared — that's where
coupling is cheap. Rendering is where it's expensive. Revisit after restack
ships and a third caller exists.

### 3.2 Renderer takes `&Stack` directly (no intermediate model)

**Decision.** `render_text(&Stack, &Repository)` and `render_json(&Stack)` take
the loaded `Stack` straight through. No `LogModel` / `LogNode` shim.

**Why.** `StackEntry` already has every field the renderer needs. The roadmap
plan proposed a `LogNode` struct, but in practice it would rename the same 12
fields. A shim only earns its keep when the renderer needs derived state
(e.g. DAG edges, layout geometry) that doesn't exist on `StackEntry`. v1 has
none of that. If v2 cross-stack DAG rendering needs a `LogGraph { nodes,
edges }` structure, introduce it then — with a real second use case to shape it.

### 3.3 JSON reuses `StackEntryJson`, wrapped in a new `LogResponse`

**Decision.** Add `LogResponse { version, log: LogJson }` and
`LogJson { stack, base, current_position, entries: Vec<StackEntryJson> }` in
`output.rs`. Do **not** mint a new per-entry shape.

**Why.** `StackEntryJson` is already the canonical per-entry wire format shared
with `gg ls --json`. MCP consumers and downstream tooling already know this
shape. A second per-entry schema would double the contract surface for zero
gain. The top-level wrapper differs (`log` vs `stack`) so the two responses
remain distinguishable and the schema version bumps cleanly in lockstep.

### 3.4 Glyph palette: Unicode box-drawing, family-consistent with `ls`

**Decision.** Use:

- **`●`** for the current commit (cyan + bold)
- **`○`** for every other commit
- **`│`** as vertical connector between rows

Rendered bottom→top (stack head printed **last**, like a git log that the eye
reads as "most recent on top" — but ordered base→head in the source data so
the connector logic stays simple; see §5.2 for the ordering convention).

**Why.** The backlog groom recommended "match `gg ls`'s character set for
consistency". `gg ls`'s `list_all_stacks` (the multi-stack tree view) already
uses Unicode box-drawing (`├──`, `└──`). Staying in the same Unicode family
keeps gg's visual identity coherent. We intentionally pick glyphs that are
**different** from both other views: `show_stack` prefixes with `[N]`,
`list_all_stacks` uses `├──`/`└──`, so `●`/`○`/`│` signals "this is the graph
view". The v2 cross-stack DAG can introduce `┬`, `┴`, `├`, `┤` for branches
without clashing.

Portability note: gg already uses `✓`, `✗`, `●`, `○`, `🚂` in other output.
There is no ASCII-only fallback. We will not introduce one for `log` unless a
user reports a real terminal that can't render these.

### 3.5 `--all` deferred, flag name reserved

**Decision.** Do **not** add `--all` in v1. Cross-stack rendering ships as a
separate task alongside `gg inbox` (Milestone 4).

**Why.** The flag carries strong semantic weight ("smartlog across all stacks,
with a DAG"). Shipping a placeholder now that just concatenates per-stack logs
would (a) set user expectations we don't meet, and (b) make the real v2 change
a breaking UX shift. Reserving the name is cheaper than retiring a bad one.

### 3.6 Refresh semantics mirror `gg ls`

**Decision.** Add `-r / --refresh`. Auto-refresh when `--json` is set. When
neither is set, opportunistically refresh if a provider is detected (best
effort, silent on failure). This reuses `should_refresh_mr_info(refresh, json)`
from `ls.rs` verbatim.

**Why.** Consistency beats cleverness. Anyone who learned `gg ls -r` gets `gg
log -r` for free. JSON consumers get authoritative state without an extra
flag. Pulling the helper from `ls.rs` into a shared spot (e.g.
`gg_core::commands::refresh`) is tempting but premature — two callers is not a
pattern; keep it duplicated for now and deduplicate when a third caller
arrives.

### 3.7 MCP `stack_log` ships in the same PR

**Decision.** Add `stack_log` to `crates/gg-mcp/src/tools.rs` alongside the
existing `stack_list` / `stack_status` / etc. tools. Document in
`docs/src/mcp-server.md`.

**Why.** The marginal cost is small: the MCP tool is a thin wrapper around
`commands::log::render_json`. Shipping together keeps the MCP surface
consistent — every stack inspection command gets an MCP counterpart — and
avoids a follow-up PR whose only purpose is copy-pasting 15 lines of tool
plumbing.

## 4. Components & responsibilities

### 4.1 `crates/gg-core/src/commands/log.rs` (new)

```rust
pub fn run(json: bool, refresh: bool) -> Result<()> {
    // 1. Open repo, load config, load Stack (mirror ls::run's opening).
    // 2. Refresh if should_refresh_mr_info(refresh, json).
    // 3. Dispatch: if json { print_json(&render_json(&stack)) } else { print!(render_text(&stack, &repo)) }.
}

fn render_text(stack: &Stack, repo: &Repository) -> String { ... }
fn render_json(stack: &Stack) -> LogResponse { ... }
```

**Responsibilities.**
- `run` is the thin orchestrator — it handles I/O (repo open, provider
  detect, print) so `render_*` stays pure and trivially unit-testable.
- `render_text` owns all glyph/styling choices and the empty-stack hint.
- `render_json` owns `LogResponse` assembly. Field population mirrors the
  `is_current` logic already in `ls::show_stack` lines 521–523 (reuse the
  same fallback: `stack.current_position.unwrap_or(len - 1)`; an entry is
  current when its position matches that, or when `current_position` is
  `None` and it's the head entry).

**What `render_text` prints, row-by-row** (for a three-entry stack, entry 2
is current, base = `main`):

```
my-feature (3 commits, 2 synced)

  ●  abc1234  Fix cache TTL bug          open       ✓  (id: c-1a2b3c4) <- HEAD
  │                                      !42
  │
  ○  def5678  Add cache layer            merged     ✓  (id: c-5d6e7f8)
  │                                      !41
  │
  ○  9012345  Extract storage interface  open       ●  (id: c-9012345)
                                         !40 [train pos 2]
```

- **Header line** and trailing blank line: identical shape to `ls::show_stack`
  so the two views share their "frame".
- **Glyph column** sits at two-column width (glyph + one space padding) so a
  future DAG column can slot in without reflowing.
- **Connectors (`│`)** print on their own line between entries, in dim style.
  No connector after the head entry. No connector after an entry whose PR
  line isn't printed — but the connector row appears *after* the PR sub-line,
  not after the commit row, so the visual "column" stays continuous.
- **Per-commit row** columns: glyph, short SHA (yellow, bold when current),
  title (bold when current), status (styled per `mr_state` exactly as in
  `show_stack`), CI marker, trailing `(id: <gg_id>)` in dim style, optional
  ` <- HEAD` for the current commit in cyan bold.
- **PR sub-line** (when `mr_number.is_some()`): indented under the glyph
  column, styled blue, optionally `[train pos N]` / `[train]` suffix.
  Identical to `ls::show_stack` lines 659–671.
- **Empty stack**: print the header line, a blank line, then `"  No commits
  yet. Use \`git commit\` to add changes."` in dim style. Same text as
  `ls::show_stack` so the empty-state phrasing stays one string.
- **Rebase-in-progress warning**: if `git::is_rebase_in_progress(&repo)`,
  print the same warning block `ls::show_stack` prints (lines 571–580).

### 4.2 `crates/gg-core/src/output.rs` (extend)

Add after the existing `StackEntryJson`:

```rust
#[derive(Serialize)]
pub struct LogResponse {
    pub version: u32,
    pub log: LogJson,
}

#[derive(Serialize)]
pub struct LogJson {
    pub stack: String,
    pub base: String,
    pub current_position: Option<usize>, // 1-indexed, None if at head / detached
    pub entries: Vec<StackEntryJson>,    // reuse existing shape
}
```

No changes to `StackEntryJson`, `OUTPUT_VERSION`, or `print_json`.

### 4.3 `crates/gg-core/src/commands/mod.rs` (extend)

Add `pub mod log;` alongside the existing modules (keep alphabetical).

### 4.4 `crates/gg-cli/src/main.rs` (extend)

Add a `Log` variant to `Commands`, modelled on the existing `List` variant
(line 42):

```rust
/// Show the current stack as a smartlog graph
#[command(name = "log")]
Log {
    /// Refresh PR/MR status from remote
    #[arg(short, long)]
    refresh: bool,

    /// Output structured JSON
    #[arg(long)]
    json: bool,
},
```

And the dispatch arm (mirroring line 372):

```rust
Some(Commands::Log { refresh, json }) => (
    gg_core::commands::log::run(json, refresh),
    json,
),
```

### 4.5 `crates/gg-mcp/src/tools.rs` (extend)

Add a `stack_log` tool right after `stack_list`:

- **Name**: `stack_log`
- **Description**: "Show the current stack as a smartlog — graph view with
  glyphs, SHAs, PR/MR state, and a current-commit marker. Returns the same
  data shape as `gg log --json`."
- **Input schema**: `{ refresh: bool (default false) }` — no `json` arg; MCP
  always returns JSON.
- **Handler**: build a `Stack`, refresh if asked, return
  `render_json(&stack)` serialized.

Pattern matches the existing `stack_list` handler exactly.

### 4.6 Docs

| File                                                   | Change                                                                           |
|--------------------------------------------------------|----------------------------------------------------------------------------------|
| `docs/src/commands/log.md` (new)                       | Mirror `docs/src/commands/ls.md` shape: synopsis, description, flags, JSON shape |
| `docs/src/SUMMARY.md`                                  | Insert `- [gg log](commands/log.md)` under the Commands section                  |
| `docs/src/mcp-server.md`                               | Add `stack_log` row to the tool table                                            |
| `README.md`                                            | **Only if** the feature list explicitly enumerates commands — check before edit  |

### 4.7 Skill

| File                          | Change                                                                   |
|-------------------------------|--------------------------------------------------------------------------|
| `skills/gg/SKILL.md`          | One-line mention of `gg log` in the commands overview                    |
| `skills/gg/reference.md`      | Flag table (`--json`, `-r/--refresh`) + JSON schema entry for `LogResponse` |

## 5. Data models & interfaces

### 5.1 `LogResponse` wire schema (JSON)

```json
{
  "version": 1,
  "log": {
    "stack": "my-feature",
    "base": "main",
    "current_position": 2,
    "entries": [
      {
        "position": 1,
        "sha": "9012345",
        "title": "Extract storage interface",
        "gg_id": "c-9012345",
        "gg_parent": null,
        "pr_number": 40,
        "pr_state": "open",
        "approved": false,
        "ci_status": "running",
        "is_current": false,
        "in_merge_train": true,
        "merge_train_position": 2
      },
      { "position": 2, "...": "...", "is_current": true },
      { "position": 3, "...": "..." }
    ]
  }
}
```

**Guarantees**:
- `version` tracks `output.rs::OUTPUT_VERSION` and changes only on breaking
  schema edits.
- `entries` is ordered base → head (position 1 is oldest, last is stack head).
- `entries[*]` is byte-for-byte the same shape `gg ls --json` already emits.
- Empty stack → `entries: []`, `current_position: null`. Still valid.

### 5.2 Rendering / ordering convention

Internal `Stack.entries` is ordered **base → head** (position 1 = oldest).
The text renderer **prints in the same order** (oldest at top, head at
bottom). This differs from `git log` / Sapling (which show newest first) but
matches `gg ls::show_stack` and `gg`'s existing mental model: "the stack
grows downward; `gg log` shows you the whole stack in the order you built it."

Callout: the backlog groom said "bottom→top". We interpret that as "iterate
base-to-head when emitting connectors" — i.e. the `●`/`│` rendering loop
walks positions 1..N in source order and `println!`s in that order. There is
no reversal step. JSON consumers that want newest-first can reverse on their
end; we keep the wire order canonical.

### 5.3 `is_current` computation

Unchanged from `ls::show_stack` lines 521–523:

```rust
let current_pos = stack.current_position.unwrap_or(stack.len().saturating_sub(1));
let is_current = entry.position == current_pos + 1
    || (stack.current_position.is_none() && entry.position == stack.len());
```

Same expression in both renderers. Extracting a helper is not worth it for a
two-line expression with one call site per renderer.

## 6. Testing strategy

### 6.1 Unit tests (co-located in `log.rs`)

All four cases construct a `Stack` in memory — no git2, no provider.

1. **Empty stack**. Build a `Stack` with `entries: vec![]`. Assert
   `render_text` contains `"No commits yet"` and `render_json(...).log.entries`
   is empty with `current_position: None`.
2. **Normal 3-entry stack, current at position 2**. Assert the rendered
   string contains `"<- HEAD"` on the line whose short SHA matches
   `entries[1].short_sha`. Assert `render_json` sets `is_current: true` on
   exactly that entry.
3. **Merged entry**. Build one entry with `mr_state = Some(PrState::Merged)`.
   Assert `render_text` contains `"merged"` somewhere (skip colour assertion
   — `console::style` is not test-stable). Mirrors
   `ls::tests::test_classification_rules_with_pr_states` (line 736).
4. **JSON schema smoke**. `serde_json::to_value(&render_json(&stack))`,
   assert the top-level keys are `version` and `log`; `log` has `stack`,
   `base`, `current_position`, `entries`; `entries[0]` has every field from
   `StackEntryJson`.

### 6.2 Integration tests (`crates/gg-cli/tests/integration_tests.rs`)

1. **`gg log` success**. Use `create_test_repo` (line 17) + `run_gg` (line
   60) to create a stack with two commits, run `gg log`, assert exit code 0
   and stdout contains both commit titles.
2. **`gg log --json` parses**. Run `gg log --json`, parse stdout with
   `serde_json::from_str::<serde_json::Value>`. Assert
   `value["version"].as_u64() == Some(1)` and `value["log"]["entries"]` is an
   array with `len() == 2`.

### 6.3 MCP tests

If `crates/gg-mcp` has an existing test scaffolding for `stack_list`, add an
equivalent test for `stack_log`. If not (current state is fine without it),
defer — MCP coverage is an existing gap, not a new one.

### 6.4 CI gates (blocking)

Per the project's CLAUDE.md conventions:

```
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

All three must pass before handoff.

## 7. Risks & trade-offs

| Risk                                                           | Mitigation                                                                                        |
|---------------------------------------------------------------|---------------------------------------------------------------------------------------------------|
| Renderer duplication with `ls::show_stack`                    | Accept it in v1. Revisit after restack adds a third stack-view caller.                           |
| Glyphs don't render in some terminals                         | Same character set as existing `list_all_stacks`; no new portability risk. Do not add ASCII fallback unless users report. |
| JSON shape becomes a contract the moment it ships             | Wire format reuses `StackEntryJson` verbatim; `LogResponse` wrapper owns the versioning.         |
| `--all` users expect cross-stack output                       | Flag is unimplemented in v1; clap rejects it with a standard unknown-flag error. Clean message, no half-built path. |
| `is_current` logic drift between `ls` and `log`               | Both renderers use the same three-line formula; unit-tested in both. If a third caller appears, extract then. |
| Merge-train annotation appears only on GitLab                 | Already the case in `ls`; the JSON fields (`in_merge_train`, `merge_train_position`) exist on every entry and are `false` / `None` on GitHub. No new risk. |
| MCP tool name collision (`stack_log` vs future `log`)         | MCP naming is stack-prefixed by convention (`stack_list`, `stack_status`, ...). `stack_log` fits. |

## 8. Open questions (resolved)

| Question                                        | Resolution                                                   |
|-------------------------------------------------|--------------------------------------------------------------|
| `--all` in v1?                                  | **No.** Reserve the flag name; implement in a future task.   |
| Glyph palette?                                  | **`●` / `○` / `│`.** Unicode box-drawing, family with `ls`.  |
| Refresh parity with `ls`?                       | **Yes.** `-r/--refresh` + auto-refresh on `--json`.          |
| MCP `stack_log` same PR?                        | **Yes.** Marginal cost, keeps MCP surface complete.          |
| `&Stack` vs derived `LogModel`?                 | **`&Stack` directly.** No shim.                              |
| Extract shared `print_stack`?                   | **No.** Not until a third caller exists.                     |
| Row order in text output?                       | **Base→head (oldest at top).** Matches `ls`; JSON consumers reverse if they want newest-first. |

No questions are left open for the implementing agent.

## 9. Definition of done

- `gg log` compiles, is registered as a clap subcommand, and prints a readable
  smartlog for any stack.
- `gg log --json` emits `LogResponse` with `version: 1` and
  `log.entries: Vec<StackEntryJson>`.
- `gg log -r` force-refreshes PR/MR state; `gg log --json` auto-refreshes.
- Four unit tests pass (empty, current marker, merged, JSON shape).
- Two CLI integration tests pass (success + JSON parse).
- `stack_log` MCP tool is registered and returns `LogResponse`.
- `docs/src/commands/log.md` exists and is linked from `SUMMARY.md`.
- `skills/gg/SKILL.md` + `reference.md` mention the command with flags and
  schema.
- `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test --all-features` all
  pass.

Handoff to the executor agent: follow the implementation order in
`docs/research/414-gg-log-smartlog-backlog.md` §"Implementation order". Every
open question is resolved above; no new triage needed.
