# Stack Workflow Roadmap — Implementation Plan

**Goal:** Improve `git-gud`'s stack workflow with six high-leverage features: PR/MR breadcrumbs, a stack-native `gg log`, `gg undo`, `gg restack`, `gg reparent`, and `gg inbox`.
**Architecture:** Ship in four milestones. Start with visibility and review-context features that layer onto existing stack/sync/provider code, then add a shared operation log for recovery, then add structural stack-editing primitives, and finally add a multi-stack triage view.
**Tech Stack:** Rust, existing `gg-core` command modules, `git2`, current GitHub/GitLab provider adapters, existing JSON output helpers, and the MCP server surface.

---

## Milestone 1: Review Context And Visibility

### Task 1: Add PR/MR stack breadcrumbs during `gg sync`

The goal is to make every synced PR/MR clearly indicate where it sits in the stack and link adjacent entries.

**Files:**
- Modify: `crates/gg-core/src/commands/sync.rs`
- Modify: `crates/gg-core/src/template.rs`
- Modify: `crates/gg-core/src/provider.rs`
- Modify: `crates/gg-core/src/gh.rs`
- Modify: `crates/gg-core/src/glab.rs`
- Modify: `crates/gg-core/src/output.rs`
- Modify: `crates/gg-cli/src/main.rs`

### Step 1: Add sync options and config plumbing

Add a CLI flag in `main.rs`:

```rust
/// Update stack breadcrumb blocks in PR/MR descriptions
#[arg(long)]
update_breadcrumbs: bool,
```

Plumb this through `sync::run()` so sync can decide whether to update breadcrumb content on create/update.

### Step 2: Add breadcrumb rendering helpers

Add helper functions in `template.rs` or a new internal helper module:

```rust
pub fn render_stack_breadcrumbs(
    stack_name: &str,
    entries: &[StackEntry],
    current_index: usize,
) -> String
```

The generated block should include:
- Stack name
- Position in stack, for example `2/5`
- Previous/next PR/MR links where available
- Compact list of stack entries

Wrap the block in stable markers so it can be replaced idempotently:

```md
<!-- gg:breadcrumbs:start -->
...
<!-- gg:breadcrumbs:end -->
```

### Step 3: Update provider description flows

Ensure GitHub and GitLab description creation/update paths can:
- append breadcrumbs on initial PR/MR creation
- replace only the generated block on re-sync
- preserve user-authored description content outside the markers

### Step 4: Add JSON output

Extend sync JSON in `output.rs` with breadcrumb status:

```json
{
  "sync": {
    "breadcrumbs": {
      "enabled": true,
      "updated": 3,
      "unchanged": 2
    }
  }
}
```

### Step 5: Write tests

Add tests for:
- first-time breadcrumb rendering
- idempotent re-sync replacement
- reorder/drop/split effects on stack position rendering
- provider description preservation

### Step 6: Update docs and skills

Update:
- `README.md`
- `docs/src/commands/sync.md`
- `docs/src/guides/your-first-stack.md` if it references review flows
- `skills/gg/SKILL.md`
- `skills/gg/reference.md`

---

### Task 2: Add `gg log` / smartlog

The goal is to complement `gg ls` with a stack-native graph view that helps users understand ancestry, current position, and review state at a glance.

**Files:**
- Create: `crates/gg-core/src/commands/log.rs`
- Modify: `crates/gg-core/src/commands/mod.rs`
- Modify: `crates/gg-core/src/output.rs`
- Modify: `crates/gg-cli/src/main.rs`
- Modify: `docs/src/mcp-server.md`

### Step 1: Register the new command

Add to `main.rs`:

```rust
/// Show a stack-native graph view of commits
#[command(name = "log")]
Log {
    /// Show all local stacks
    #[arg(long)]
    all: bool,

    /// Include hidden/superseded commits when available
    #[arg(long)]
    hidden: bool,

    /// Output structured JSON
    #[arg(long)]
    json: bool,
},
```

Wire it to `gg_core::commands::log::run(...)`.

### Step 2: Build graph-oriented stack output

Implement `log.rs` by reusing stack loading and provider status data. The initial version should show:
- base branch
- current stack and current commit marker
- commit order from base to head
- GG-ID
- sync state
- PR/MR state
- CI/approval state where already available

Prefer separating data assembly from text rendering:

```rust
struct LogNode {
    position: usize,
    sha: String,
    gg_id: Option<String>,
    title: String,
    is_current: bool,
    pr: Option<PrSummary>,
}
```

### Step 3: Add JSON and MCP support

Add `log` JSON output in `output.rs`, then expose it via MCP in `docs/src/mcp-server.md` as a new `stack_log` tool.

### Step 4: Write tests

Add tests for:
- current-position marker
- single-stack and multi-stack rendering
- `--all` output shape
- JSON schema

### Step 5: Update docs and skills

Update:
- `README.md`
- Add `docs/src/commands/log.md`
- `docs/src/SUMMARY.md`
- `skills/gg/SKILL.md`
- `skills/gg/reference.md`

---

## Milestone 2: Safety And Recovery

### Task 3: Add `gg undo` backed by an operation log

The goal is to make history-rewriting commands reversible through a first-class `gg` workflow rather than raw reflog knowledge.

**Files:**
- Create: `crates/gg-core/src/commands/undo.rs`
- Create: `crates/gg-core/src/operations.rs`
- Modify: `crates/gg-core/src/commands/mod.rs`
- Modify: `crates/gg-core/src/output.rs`
- Modify: `crates/gg-cli/src/main.rs`
- Modify: mutating command modules to record operations

Likely mutating command touch points:
- `crates/gg-core/src/commands/split.rs`
- `crates/gg-core/src/commands/drop_cmd.rs`
- `crates/gg-core/src/commands/reorder.rs`
- `crates/gg-core/src/commands/rebase.rs`
- `crates/gg-core/src/commands/squash.rs`
- `crates/gg-core/src/commands/absorb.rs`
- `crates/gg-core/src/commands/checkout.rs`
- `crates/gg-core/src/commands/sync.rs` for local ref changes only

### Step 1: Define operation log storage

Store operation records under `.git/gg/operations/`.

Define a serializable record:

```rust
#[derive(Debug, Serialize, Deserialize)]
struct OperationRecord {
    id: String,
    kind: String,
    created_at: String,
    stack_name: Option<String>,
    head_before: String,
    head_after: Option<String>,
    refs_before: Vec<RefSnapshot>,
    refs_after: Vec<RefSnapshot>,
    touched_remote: bool,
}
```

### Step 2: Add record/replay helpers

Implement helpers in `operations.rs`:
- create a pending record before mutation
- finalize it after success
- list recent operations
- replay the reverse of supported operations

The first supported undo target should be local ref/HEAD restoration. Remote rollback can be explicitly out of scope for v1, but it must be surfaced clearly in output.

### Step 3: Add the `undo` command

Add to `main.rs`:

```rust
/// Undo a recent gg operation
#[command(name = "undo")]
Undo {
    /// List undoable operations
    #[arg(long)]
    list: bool,

    /// Specific operation ID to undo
    operation_id: Option<String>,

    /// Output structured JSON
    #[arg(long)]
    json: bool,
},
```

### Step 4: Instrument mutating commands

Wrap mutating commands so they:
- capture pre-operation repo state
- persist a pending record
- finalize on success
- leave enough information for post-failure diagnosis

### Step 5: Add JSON and MCP support

Add output for:
- undo success
- unsupported undo
- list of recorded operations

Expose MCP tools:
- `stack_undo`
- `stack_undo_list`

### Step 6: Write tests

Add integration tests for undoing:
- reorder
- drop
- split
- rebase

Also test:
- listing operations
- unsupported remote rollback messaging
- interrupted/pending record handling

### Step 7: Update docs and skills

Update:
- `README.md`
- Add `docs/src/commands/undo.md`
- `docs/src/faq.md`
- `skills/gg/SKILL.md`
- `skills/gg/reference.md`

---

## Milestone 3: Structural Stack Editing

### Task 4: Add `gg restack`

The goal is to repair stack parent/child structure after manual Git operations or upstream rebases without making the user think in raw Git terms.

**Files:**
- Create: `crates/gg-core/src/commands/restack.rs`
- Modify: `crates/gg-core/src/commands/mod.rs`
- Modify: `crates/gg-core/src/output.rs`
- Modify: `crates/gg-cli/src/main.rs`
- Modify: `docs/src/mcp-server.md`

Likely shared helpers:
- `crates/gg-core/src/stack.rs`
- `crates/gg-core/src/git.rs`
- `crates/gg-core/src/commands/reconcile.rs`

### Step 1: Add the command surface

Add to `main.rs`:

```rust
/// Repair stack ancestry after manual history changes
#[command(name = "restack")]
Restack {
    /// Show planned changes without mutating history
    #[arg(long)]
    dry_run: bool,

    /// Repair only from this target upward
    #[arg(long)]
    from: Option<String>,

    /// Output structured JSON
    #[arg(long)]
    json: bool,
},
```

### Step 2: Build a shared stack rewrite planner

Before implementing execution, add an internal planning type that can be reused later by `reparent`:

```rust
struct StackRewritePlan {
    operations: Vec<RewriteStep>,
}
```

The plan should capture:
- the intended stack order
- the current ancestry mismatch
- what rebases or ref updates must occur

### Step 3: Implement dry-run planning

Implement `--dry-run` first. It should report:
- commits that are already correct
- commits that would be reattached
- whether remote branch/PR lineage will need updating on next sync

### Step 4: Implement execution

Execute the plan with the existing rebase patterns used by `reorder` and `drop`, while preserving GG-ID mappings.

### Step 5: Add JSON and MCP support

Add a `restack` JSON payload in `output.rs` and document a new MCP tool:
- `stack_restack`

### Step 6: Write tests

Add integration tests for:
- no-op restack
- manual amend causing drift
- manual cherry-pick/rewrite causing descendant mismatch
- partial `--from` repair
- dry-run JSON

### Step 7: Update docs and skills

Update:
- `README.md`
- Add `docs/src/commands/restack.md`
- `docs/src/faq.md`
- `skills/gg/SKILL.md`
- `skills/gg/reference.md`

---

### Task 5: Add `gg reparent`

The goal is to intentionally move one commit or a contiguous subtree under a different parent entry in the stack.

**Files:**
- Create: `crates/gg-core/src/commands/reparent.rs`
- Modify: `crates/gg-core/src/commands/mod.rs`
- Modify: `crates/gg-core/src/output.rs`
- Modify: `crates/gg-cli/src/main.rs`
- Modify: `crates/gg-core/src/commands/sync.rs` for lineage-sensitive updates
- Modify: `docs/src/mcp-server.md`

### Step 1: Add the command surface

Add to `main.rs`:

```rust
/// Move a commit or subtree under a different parent
#[command(name = "reparent")]
Reparent {
    /// Target commit or subtree root
    target: String,

    /// New parent commit
    #[arg(long)]
    onto: String,

    /// Show planned changes without mutating history
    #[arg(long)]
    dry_run: bool,

    /// Output structured JSON
    #[arg(long)]
    json: bool,
},
```

### Step 2: Reuse the stack rewrite planner

Build `reparent` on top of the same planning abstraction introduced for `restack`:
- resolve target subtree
- reject invalid cycles
- produce the new intended order/parenting
- execute via existing rebase machinery

### Step 3: Handle sync consequences

Ensure that the next `gg sync` updates:
- branch lineage
- PR/MR breadcrumb order
- any provider-side base relationships currently modeled by sync

### Step 4: Add JSON and MCP support

Add a `reparent` result payload and document a new MCP tool:
- `stack_reparent`

### Step 5: Write tests

Add integration tests for:
- single-commit reparent
- subtree reparent
- invalid self-descendant parenting
- sync after reparent

### Step 6: Update docs and skills

Update:
- `README.md`
- Add `docs/src/commands/reparent.md`
- any editing workflow guide that references reorder/drop/split
- `skills/gg/SKILL.md`
- `skills/gg/reference.md`

---

## Milestone 4: Multi-Stack Triage

### Task 6: Add `gg inbox`

The goal is to show users which stacks need attention across the repository, rather than requiring them to inspect each stack manually.

**Files:**
- Create: `crates/gg-core/src/commands/inbox.rs`
- Modify: `crates/gg-core/src/commands/mod.rs`
- Modify: `crates/gg-core/src/output.rs`
- Modify: `crates/gg-core/src/provider.rs`
- Modify: `crates/gg-core/src/gh.rs`
- Modify: `crates/gg-core/src/glab.rs`
- Modify: `crates/gg-cli/src/main.rs`
- Modify: `docs/src/mcp-server.md`

### Step 1: Add the command surface

Add to `main.rs`:

```rust
/// Show actionable stacks and PRs/MRs
#[command(name = "inbox")]
Inbox {
    /// Include all local stacks
    #[arg(long)]
    all: bool,

    /// Output structured JSON
    #[arg(long)]
    json: bool,
},
```

### Step 2: Normalize provider state into action buckets

Implement a provider-neutral view of actionable status, for example:
- ready to land
- blocked on CI
- awaiting review
- changes requested
- draft
- behind base

Keep the first version author-centric and limited to repository-local stacks.

### Step 3: Add rendering and JSON

Produce both:
- human-readable grouped output
- structured JSON for tooling and MCP

### Step 4: Add MCP support

Document a new MCP tool:
- `stack_inbox`

### Step 5: Write tests

Add tests for:
- status bucketing
- mixed GitHub/GitLab normalization behavior
- stacks with no PR/MR
- JSON schema

### Step 6: Update docs and skills

Update:
- `README.md`
- Add `docs/src/commands/inbox.md`
- `docs/src/guides/README.md` if needed
- `skills/gg/SKILL.md`
- `skills/gg/reference.md`

---

## Cross-Cutting Rules

### Rule 1: Preserve GG-ID stability

All stack-rewriting features must preserve GG-ID mapping where possible so PR/MR associations survive reorders, restacks, and reparents.

### Rule 2: Add JSON output for operational commands

All new commands in this roadmap should support `--json` if they:
- mutate history
- summarize actionable repository state
- are likely MCP/agent entry points

### Rule 3: Keep planning and execution separate

For `restack`, `reparent`, and `undo`, design internal plan types before implementing mutation logic so dry-run, JSON output, and safety checks remain coherent.

### Rule 4: Update docs and skills with every user-facing change

Each milestone must update:
- `README.md`
- relevant `docs/src/commands/*.md`
- `docs/src/mcp-server.md` when MCP changes
- `skills/gg/SKILL.md`
- `skills/gg/reference.md`

### Rule 5: Follow the repo quality bar

Before any milestone is considered complete:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

---

## Recommended Delivery Order

1. PR/MR breadcrumbs
2. `gg log`
3. operation log foundation
4. `gg undo`
5. shared stack rewrite planner
6. `gg restack`
7. `gg reparent`
8. `gg inbox`

This order front-loads user-visible wins, then safety, then deeper stack-editing capabilities, and finally repository-wide triage.
