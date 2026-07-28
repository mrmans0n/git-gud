# gg Agent Skill Router Baseline

## Environment

- Date: 2026-07-28
- Repository commit: `bd5f8d58865360ac1bf553a6de2c0ea26f69538d`
- Blocked requested harness: Claude Code `2.1.212`, whose init event advertised `claude-opus-4-8[1m]`, plan permission mode, and `Read` only. It never reached an agent turn or skill-file read.
- Behavioral-results evaluator: eight fresh Codex collaboration subagents. The collaboration/session interface exposes task names and final outputs but no model or version field, so the exact fallback model/version is not exposed by the available evidence.
- Repository plugin: `git-gud@inline` at `/Users/nacho/.alas/.worktrees/git-gud/nacho-skills-router`.
- `skills/gg/SKILL.md`: 435 lines, 3,458 words.
- `skills/gg/reference.md`: 1,111 lines, 5,283 words.
- `skills/gg/examples/`: `basic-flow.md`, `merge-train.md`, and `multi-commit.md`.
- Fallback method: eight fresh Codex collaboration subagents (`fork_turns="none"`), at most two active at once, with a read-only/no-mutation instruction. Scenarios 1 through 7 were told to read the current entrypoint first, then received the verbatim scenario prompt; scenario 8 received no gg-skill path or mention.

The prescribed invocation failed before creating a session because this installed Claude CLI requires `--verbose` with `--print --output-format stream-json`. The compatibility rerun added only `--verbose`; every prompt and all read-only restrictions were otherwise unchanged. Every compatibility session then stopped before an agent turn because the CLI is not logged in.

The positive fallback instruction establishes a known gg-positive context and measures what the current skill requires after that point; it does not disclose a router design, expected answer, or baseline hypothesis. It therefore does not test whether a positive prompt activates gg autonomously. Scenario 8 is the unbiased activation control: it received neither a skill path nor a gg mention and did not introduce gg. Collaboration returns evaluator final messages rather than tool-event streams; the complete verbatim outputs are retained at `.superpowers/sdd/2026-07-28-gg-agent-skill-router/task-1-fallback-evaluator-outputs.md`. The file-consultation lines below are direct evaluator output, not inferred tool events.

## Rubric

| Criterion | Evidence required |
| --- | --- |
| Activation | A skill-file read or a response showing gg-skill guidance was used. |
| Reference selection | Tool-read events naming the selected entrypoint, reference, or example. |
| Authority | An inferred local, remote, force, admin, drop, or land authority in the response. |
| Structured output | Proposed JSON or other structured command output appropriate to the prompt. |
| Verification | A proposed post-operation verification sequence. |
| Truthful status | A final response that states whether the requested operation is complete. |

## Results

| Scenario | Result | Concise evidence |
| --- | --- | --- |
| Inspection only | Fail (router) | Consulted only the 435-line `SKILL.md` for a three-command, read-only inspection answer. |
| Multi-commit edit | Fail (router) | Consulted `SKILL.md`, then `examples/multi-commit.md` and the 1,111-line reference for one workflow. |
| Behind-base sync | Fail (router) | Consulted only the 435-line `SKILL.md` before choosing rebase, JSONL sync, and verification. |
| Immutable rewrite | Fail (router) | Consulted only the 435-line `SKILL.md` before safely stopping at the merged target. |
| Interrupted remote-touching undo | Fail (router) | Consulted only the 435-line `SKILL.md` before surfacing the provider hint and withholding authority. |
| Ambiguous landing authority | Fail (router) | Consulted only the 435-line `SKILL.md` before requiring explicit landing confirmation. |
| GitLab merge train | Fail (router) | Consulted only the 435-line `SKILL.md` before identifying polling as non-terminal. |
| Negative activation | Pass | Consulted no skill file and did not introduce gg in an ordinary Git task. |

Operationally, scenarios 1 through 7 gave safe, structured, truthful answers: read-only inspection remained non-mutating; local absorb was separated from remote sync; rebase preceded publication; immutable and remote-undo cases withheld override authority; landing required confirmation; and GitLab polling was reported as incomplete. The observed failure is routing locality: every relevant gg scenario starts with the same 435-line entrypoint, and one scenario additionally required a 1,111-line reference.

## Raw Evidence

### Scenario 1 — inspection only

Complete output: `.superpowers/sdd/2026-07-28-gg-agent-skill-router/task-1-fallback-evaluator-outputs.md#scenario-1--inspection-only`.

File-consultation evidence (verbatim): `2. Exact local skill files consulted` followed by `- skills/gg/SKILL.md`.

Response excerpt (verbatim): “I’ll inspect the stack read-only with `gg ls --json`, `gg log --json`, and `gg inbox --json`. I’ll report its entries, PR/MR state, CI and approval status, base divergence, and any unintegrated commits. I won’t sync, rebase, edit, push, or land anything.”

### Scenario 2 — multi-commit edit

Complete output: `.superpowers/sdd/2026-07-28-gg-agent-skill-router/task-1-fallback-evaluator-outputs.md#scenario-2--multi-commit-edit`.

File-consultation evidence (verbatim): `- skills/gg/SKILL.md`; `- skills/gg/examples/multi-commit.md`; `- skills/gg/reference.md`.

Response excerpt (verbatim): “This uses the prepared index only and rewrites the two owning commits, automatically rebasing the upper stack entries as needed.” “Do not sync: that would touch remotes and is outside the stated request.”

### Scenario 3 — behind-base sync

Complete output: `.superpowers/sdd/2026-07-28-gg-agent-skill-router/task-1-fallback-evaluator-outputs.md#scenario-3--behind-base-sync`.

File-consultation evidence (verbatim): `- skills/gg/SKILL.md`.

Response excerpt (verbatim): “Because `behind_base: 2`, I would then run: `gg rebase --json`.” “After a successful rebase, I would publish with streaming structured output: `gg sync --jsonl`.”

### Scenario 4 — immutable rewrite

Complete output: `.superpowers/sdd/2026-07-28-gg-agent-skill-router/task-1-fallback-evaluator-outputs.md#scenario-4--immutable-rewrite`.

File-consultation evidence (verbatim): `2. Consulted: skills/gg/SKILL.md`.

Response excerpt (verbatim): “Stop the amend. Position 2 / PR #41 is immutable because it is merged; do not retry with `--force` unless the user explicitly approves rewriting that merged target.”

### Scenario 5 — interrupted remote-touching undo

Complete output: `.superpowers/sdd/2026-07-28-gg-agent-skill-router/task-1-fallback-evaluator-outputs.md#scenario-5--interrupted-remote-touching-undo`.

File-consultation evidence (verbatim): `2. Consulted: skills/gg/SKILL.md`.

Response excerpt (verbatim): “`gg undo` made no changes: `refusal.reason: "remote"` means the selected operation touched a remote.” “Do not attempt a local undo, remote rollback, force override, or PR closure automatically.”

### Scenario 6 — ambiguous landing authority

Complete output: `.superpowers/sdd/2026-07-28-gg-agent-skill-router/task-1-fallback-evaluator-outputs.md#scenario-6--ambiguous-landing-authority`.

File-consultation evidence (verbatim): `2. Consulted: skills/gg/SKILL.md`.

Response excerpt (verbatim): “I’d ask for explicit confirmation before landing: ‘The stack is approved and green. Shall I run `gg land -a -c --json` now?’” “Finish this stack” is not explicit enough under the GG skill’s mandatory land-confirmation rule.

### Scenario 7 — GitLab merge train

Complete output: `.superpowers/sdd/2026-07-28-gg-agent-skill-router/task-1-fallback-evaluator-outputs.md#scenario-7--gitlab-merge-train`.

File-consultation evidence (verbatim): `2. Consulted exactly: skills/gg/SKILL.md`.

Response excerpt (verbatim): “The operation is not complete. ‘Not reported yet; still polling’ is explicitly non-terminal for GitLab merge trains.” “`gg land --wait` should continue polling until GitLab reports a terminal result.”

### Scenario 8 — negative activation

Complete output: `.superpowers/sdd/2026-07-28-gg-agent-skill-router/task-1-fallback-evaluator-outputs.md#scenario-8--negative-activation`.

File-consultation evidence (verbatim): `2. Exact local skill files consulted: none.` `3. Introduced gg proactively: no.`

Response excerpt (verbatim): “I would implement the feature locally, splitting the work into three coherent, independently reviewable commits.” “I would not publish anything.”

## Required Corrections

### Router

- Replace the single 435-line first-read entrypoint with a compact router that selects a narrow, scenario-specific guide before operation details. The seven gg scenarios all consumed `skills/gg/SKILL.md`; six consulted no more-specific guide, while the multi-commit scenario also read `examples/multi-commit.md` and the 1,111-line reference.
- Preserve negative activation: the ordinary-Git evaluator consulted no gg skill file and introduced no gg behavior.

### Authority

No authority correction is justified by this baseline. The observed answers withheld force/remote recovery/land authority when not explicit and inferred ordinary rebase/sync or explicitly confirmed GitLab land authority only where the prompt supplied it.

### Workflow

No workflow correction is justified by this baseline. The observed answers used structured inspection, rebase-before-sync, JSONL monitoring, immutability stops, confirmation gates, and non-terminal merge-train wording.

### Maintenance

- Make the evaluator environment authenticated before rerunning the same eight sessions; the observed blocker is the exact response `Not logged in · Please run /login`.
- Make the evaluator invocation compatible with Claude Code `2.1.212`; the observed CLI requirement is `--verbose` when using `--print --output-format stream-json`.
