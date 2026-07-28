# gg Agent Skill Router Design

## Summary

Reshape the unified `gg` Agent Skill into a compact, CLI-first operational
router. Keep the safety and execution rules required for every gg task in
`SKILL.md`, then load one focused workflow reference based on the user's goal.

The skill is for agents operating git-gud, not for teaching humans or
reproducing the command manual. Human explanations remain in mdBook, and the
installed CLI remains the source of truth for available flags.

## Problem

The current skill entrypoint has accumulated four responsibilities:

1. Skill activation and shared safety policy.
2. End-to-end workflow guidance.
3. Exhaustive command and feature documentation.
4. Native-client and MCP integration documentation.

`skills/gg/SKILL.md` is 435 lines, while the monolithic `reference.md` is 1,111
lines. Both repeat setup, command, and behavioral details. Repository guidance
also requires user-facing changes to update both files, which encourages
append-only growth and documentation drift.

This shape makes agents load irrelevant context, obscures the decisions they
must make, and makes runtime CLI behavior compete with copied flag
documentation.

## Goals

- Optimize for agents safely executing gg workflows.
- Activate when the user explicitly requests gg or stacked diffs, or when the
  repository is already managed as a gg stack.
- Keep the CLI as the canonical execution surface.
- Load only the workflow guidance relevant to the current intent.
- Keep shared authority and safety rules active for every workflow.
- Make procedures easy to scan and hard to misinterpret.
- Reduce duplicated command and schema documentation.
- Make future feature changes update the skill only when agent behavior
  changes.
- Validate the rewrite through realistic agent behavior, not prose review
  alone.

## Non-goals

- Do not turn unrelated multi-commit tasks into stacked-diff workflows.
- Do not make the skill a self-contained human manual.
- Do not reproduce every CLI flag or complete JSON schema.
- Do not make MCP the primary execution surface.
- Do not split gg into independently triggered leaf skills.
- Do not add wrapper scripts unless testing reveals a repeated operation that
  the gg CLI cannot perform reliably itself.

## Activation Contract

Use one discoverable skill named `gg`. Its frontmatter description should
contain only triggering conditions, for example:

```yaml
---
name: gg
description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
---
```

The skill activates for explicit gg or stacked-diff requests and for work in an
existing gg-managed stack. It does not activate merely because a task could be
divided into several commits.

## Architecture

Keep a single skill so every workflow inherits the same authority and safety
rules. Turn its entrypoint into an intent router:

```text
skills/gg/
├── SKILL.md
└── references/
    ├── setup-and-inspection.md
    ├── editing-stacks.md
    ├── syncing-and-reviews.md
    ├── landing-and-cleanup.md
    ├── recovery.md
    └── native-clients.md
```

All references are one hop from `SKILL.md`. Do not route to more independently
triggered skills: doing so would fragment a stateful workflow, create
overlapping activation descriptions, and require safety policy to be duplicated
or conditionally loaded.

### Entrypoint

Target roughly 120–180 lines for `SKILL.md`. It contains only information
needed on every activated task:

1. Core operating principle.
2. Authority matrix and non-negotiable safety boundaries.
3. Initial repository and stack inspection.
4. Intent-routing table.
5. Shared execution and verification loop.
6. Final status-reporting contract.

The entrypoint should read in execution order:

```text
# Operating gg
1. Establish authority
2. Inspect repository state
3. Route by intent
4. Execute the selected workflow
5. Verify and report
```

### Intent Routing

Route by user goal rather than by individual command:

| User intent | Reference |
|---|---|
| Initialize, enter, or inspect a stack | `references/setup-and-inspection.md` |
| Amend, absorb, split, reorder, unstack, drop, rebase, or restack | `references/editing-stacks.md` |
| Publish, update, or monitor PRs or MRs | `references/syncing-and-reviews.md` |
| Land or clean stacks | `references/landing-and-cleanup.md` |
| Resolve conflicts, undo, or recover interrupted work | `references/recovery.md` |
| Integrate a native client or MCP surface | `references/native-clients.md` |

Load one primary workflow reference at a time. If the requested outcome spans
multiple phases, load the next reference only when entering that phase. Load
`recovery.md` additionally only when an error or interrupted state requires
recovery.

## Authority and Safety

Put the shared authority matrix near the beginning of `SKILL.md`:

| Action | Agent authority |
|---|---|
| Read repository and stack state | Run immediately |
| Make requested local stack edits | Allowed when implied by the task |
| Push or create/update PRs or MRs with `gg sync` | Only when publishing or syncing is requested |
| Drop commits or bypass immutability | Surface affected targets and obtain explicit approval |
| Land | Obtain explicit confirmation immediately before execution |
| Use `--force`, `--ignore-immutable`, or `--admin` | Never infer; require explicit approval |

Every workflow also inherits these rules:

- Inspect `git status --short` before mutation.
- Stage only reviewed, intended files; never use `git add -A` blindly.
- Prefer worktrees for newly created stacks.
- Use structured output for decisions.
- Re-inspect state after mutation.
- Stop rather than improvise when ownership, state, or authority is ambiguous.

## Shared Execution Loop

Use the following loop for all routed workflows:

1. Establish the requested outcome and whether it authorizes local or remote
   mutation.
2. Run `git status --short`.
3. Inspect the relevant stack using structured output.
4. Stop on unrelated dirty state or ambiguous ownership.
5. Load the primary workflow reference selected by intent.
6. Execute the smallest operation that satisfies the request.
7. Re-inspect repository, stack, and remote state affected by the operation.
8. Report completed actions, remote effects, and remaining gates truthfully.

Use JSON when the agent needs a final structured response. Use JSONL for
long-running sync operations so progress can be monitored while retaining a
final summary event.

## Reference Responsibilities

Every reference uses the same internal shape:

1. Preconditions.
2. Procedure.
3. Stop conditions.
4. Verification.
5. Reporting requirements.

### `setup-and-inspection.md`

- Detect the installed gg command and version when compatibility matters.
- Detect repository configuration, provider, current stack, dirty state, and
  worktree context.
- Create or enter stacks.
- Explain shell-integration directory changes and the manual fallback.
- Define how to proceed when the repository is not initialized for gg.

### `editing-stacks.md`

- Cover amend, staged-only squash, absorb, split, reorder, insert, unstack,
  drop, rebase, and restack workflows.
- Centralize dirty-tree behavior, detached positions, immutability, and
  post-rewrite verification.
- Keep terminal split interactive.
- Route structured native-client split behavior to `native-clients.md`.
- Surface immutable targets before requesting approval for an override.

### `syncing-and-reviews.md`

- Determine whether rebase or lint is required before sync.
- Run and monitor `gg sync --jsonl`, then consume its final event.
- Interpret draft behavior, branch-prefix warnings, recreated PRs or MRs, CI,
  approval, and provider-specific states.
- Do not publish unless the user's request authorizes publishing.
- Verify remote state at the end rather than treating a successful push as the
  entire outcome.

### `landing-and-cleanup.md`

- Define the readiness predicate from current approval, CI, and merge state.
- Ask for explicit confirmation immediately before landing.
- Cover the explicitly approved GitHub admin bypass.
- Interpret GitLab merge-train terminal and non-terminal states.
- Verify the remote merge result before cleaning local state.

### `recovery.md`

- Cover conflict continuation, interrupted operations, `gg undo`, stale
  records, remote rollback refusal, and ancestry repair.
- Preserve and surface provider-specific rollback hints.
- Define explicit stop conditions so agents do not invent destructive recovery
  procedures.

### `native-clients.md`

- Load only for MCP or native-client integration work.
- Cover client operation IDs, structured Describe/Apply protocols, staged-only
  mutations, and tool-to-CLI correspondence.
- Keep schema detail here only when it is required to implement a native
  integration.

## Prose Contract

Write the skill as an executable decision contract:

- Use imperative sentences.
- Tie decisions to observable predicates.
- Place a command beside the decision it implements.
- Prefer positive recipes for workflow shape.
- Reserve strong prohibitions for authority, destructive action, and data-loss
  boundaries.
- Avoid product explanations, release history, exhaustive flag catalogs, and
  repeated examples.
- Keep each operational fact in one place.

For example, write:

> If output contains `ImmutableTargets`, report each affected commit and reason,
> then ask whether to retry with `--force`.

Do not replace that contract with a general explanation of commit
immutability.

## Runtime Sources of Truth

Resolve operational truth in this order:

1. Installed `gg <command> --help` for available commands and flags.
2. Actual structured command output for current state.
3. Focused skill guidance for decisions, sequencing, and safety.
4. mdBook for explanatory product documentation.

The skill names only JSON fields that affect agent decisions, such as approval,
CI state, behind-base state, refusal reason, and operation ID. It does not
duplicate complete response schemas.

If the installed binary lacks an expected flag, report the version mismatch.
Do not guess a substitute command.

## Examples and Existing Reference Migration

Remove the monolithic `reference.md` after its operationally necessary content
has moved into the focused references. Move human explanations and exhaustive
command details to mdBook when they are not already present.

Remove standalone skill tutorials unless baseline and forward-testing show that
an agent needs one to execute correctly. If an example remains necessary, keep
one compact golden path in its relevant workflow reference rather than a
separate examples hierarchy.

## Evaluation Strategy

Treat the rewrite as a behavior change and test it with fresh agent contexts.

### Baseline

Run the current skill against these scenarios before editing:

1. Inspect an existing stack.
2. Amend two different stack commits.
3. Sync a behind-base stack.
4. Handle `ImmutableTargets`.
5. Recover an interrupted local operation.
6. Respond to "finish this stack" without inferring landing approval.
7. Monitor a GitLab merge train.
8. Handle an unrelated multi-commit task outside gg without activating
   proactively.

Record the files loaded, decisions made, commands selected, authority behavior,
verification, and final report.

### Forward Test

Run the same scenarios against the router. Score:

- Correct activation.
- Correct reference selection.
- Minimal irrelevant context loaded.
- Safe authority decisions.
- Correct structured-output mode.
- Verification after mutation.
- Truthful final status.

Add separate positive, negative, and ambiguous prompt tests for the frontmatter
description. Use observed baseline and forward-test failures to tighten the
smallest relevant instruction rather than adding speculative prose.

### Structural Validation

- Validate the skill with an Agent Skills validator.
- Verify every router link resolves.
- Verify every reference is reachable directly from `SKILL.md`.
- Check that the core skill contains no duplicated flag catalog or complete
  JSON schema.
- Build the project documentation after installation guidance changes.

## Maintenance Contract

Replace the current rule that every user-facing change updates both
`SKILL.md` and `reference.md` with this ownership policy:

| Change | Required skill update |
|---|---|
| New or renamed CLI flag | Usually none; update Clap help and mdBook |
| New agent decision or safety boundary | Update the router or relevant workflow |
| Changed multi-command workflow | Update one focused workflow reference |
| New JSON field | Update only if agents make decisions from it |
| Native-client protocol change | Update `native-clients.md` |
| Human tutorial or example | Update mdBook, not the skill |

Update `AGENTS.md`, the agent-skills guide, and any repository contribution
guidance to encode this ownership model. This policy is part of the design:
without it, the compact router will grow back into a command manual.

## Success Criteria

- `SKILL.md` is a compact router containing only universal operating guidance.
- Explicit gg requests and existing gg stacks activate the skill; unrelated
  multi-commit work does not.
- A normal task loads only the workflow references required by its current
  phases, one at a time.
- CLI structured output is the canonical execution surface.
- No workflow can bypass the shared landing, force, drop, or admin authority
  gates.
- Exact flag documentation comes from the installed CLI.
- Human explanations remain in mdBook.
- Agent behavior tests pass for routing, safety, execution, verification, and
  status reporting.
- Repository guidance prevents routine CLI additions from bloating the skill.
