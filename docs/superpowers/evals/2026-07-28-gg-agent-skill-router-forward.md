# gg Agent Skill Router Forward Evaluation

## Environment

- Date: 2026-07-28
- Evaluation base commit: `2edf60981754ecfeb4075c175985f73c92a0a716`
- `skills/gg/SKILL.md`: 58 lines, 388 words.
- Requested Claude evaluator: Claude Code 2.1.212; its compatibility init
  advertised `claude-opus-4-8[1m]`. The prescribed command failed because
  stream JSON requires `--verbose`; adding only that flag then stopped before
  an evaluator turn with `Not logged in · Please run /login`.
- Behavioral fallback: eight fresh collaboration evaluators (`fork_turns="none"`),
  with at most two active at once, requested as `gpt-5.6-terra` at medium
  reasoning. The interface does not expose an evaluator model/version in the
  returned evidence. They were read-only and received neither the baseline
  report nor expected answers.
- Raw evaluator output: `.superpowers/sdd/2026-07-28-gg-agent-skill-router/task-4-evaluator-outputs.md`.

## Results

| Scenario | Baseline result | Routed result | Evidence |
| --- | --- | --- | --- |
| Inspection only | Fail (router) | Pass | Read `SKILL.md` then only `references/setup-and-inspection.md`; proposed read-only JSON inspection. |
| Multi-commit edit | Fail (router) | Pass | Read `SKILL.md` then only `references/editing-stacks.md`; selected staged-only `gg absorb -s` with no sync. |
| Behind-base sync | Fail (router) | Pass | Read `SKILL.md` then only `references/syncing-and-reviews.md`; rebases before `gg sync --jsonl` and waits for final summary. |
| Immutable rewrite | Fail (router) | Pass | Read editing then recovery after the reported `ImmutableTargets` error; surfaced merged PR #41 and required approval. |
| Remote-touching undo | Fail (router) | Pass | Read `SKILL.md` then only `references/recovery.md`; surfaced `gh pr close 52` and stopped for approval. |
| Ambiguous landing authority | Fail (router) | Pass | Read `SKILL.md` then only `references/landing-and-cleanup.md`; asked for immediate landing confirmation. |
| GitLab merge train | Fail (router) | Pass | Read `SKILL.md` then only `references/landing-and-cleanup.md`; described polling as pending, not complete. |
| Negative activation | Pass | Pass | Read no skill file and introduced neither gg nor stacked diffs. |

Auditable excerpts: scenario 3 said “run an ordinary rebase before publishing,
then monitor `gg sync` as JSONL”; scenario 5 said “Surface the provider hint
`gh pr close 52` and stop”; and scenario 7 said “Landing is pending, not
complete.” The complete verbatim messages are retained at the raw-output path.

## Activation Samples

Twenty fresh frontmatter-only samples were evaluated: five explicit-positive,
five existing-context, five negative, and five ambiguous non-Git prompts.

| Category | Result |
| --- | --- |
| Explicit positive | 5/5 activate |
| Existing gg-managed context | 5/5 activate |
| Ordinary Git / unrelated PR | 5/5 do not activate |
| Ambiguous non-Git “stack” | Initial 4/5 do not activate; the failed sample reran as do-not-activate, for final 5/5 |

The failure was the prompt “Stack these changes for the board meeting,” whose
initial evaluator answer was: “activate — ‘Stack these changes’ directly
requests a stacked-diffs workflow.”

## Corrections

| Failure | Owning file | Correction | Fresh rerun |
| --- | --- | --- | --- |
| Ambiguous non-Git wording activated | `skills/gg/SKILL.md` | Require an explicit request for gg/git-gud or the exact terms `stacked diffs`, `stacked PRs`, or `stacked MRs`; retain the explicit gg-managed-repository trigger. | “do-not-activate — ‘Stack these changes’ is ambiguous and does not explicitly indicate git-gud or a gg-managed repository.” |

The structural contract was updated in `crates/gg-cli/tests/skill_contract.rs`
for the refined frontmatter. Its expected string was changed first and the
test failed against the old description, then passed after the skill change.
No workflow reference changed.

## Outcome

All eight workflow rubric items pass under the fallback evaluator: activation,
reference selection, authority, structured output, verification, and truthful
status. All required activation categories now meet their expected result,
including the corrected ambiguous rerun. The unauthenticated Claude CLI
remains a limitation: it did not enter an evaluator turn, so it cannot supply
independent tool-read streams in this environment.

## Validation

All exited 0:

```text
rtk cargo fmt --all
rtk cargo test -p gg-cli --test skill_contract
rtk uvx --from 'git+https://github.com/agentskills/agentskills.git#subdirectory=skills-ref' skills-ref validate skills/gg
rtk git diff --check
```

`skill_contract`: 1 passed. `skills-ref`: `Valid skill: skills/gg`.
