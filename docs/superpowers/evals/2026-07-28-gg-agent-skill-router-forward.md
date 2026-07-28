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
- Tracked evaluator provenance (fresh context identifiers, exact dispatch
  templates and prompts, activation candidate descriptions, and concise exact
  results): [forward evaluator evidence](2026-07-28-gg-agent-skill-router-forward-evidence.md).
  The ignored SDD appendix is only a supplementary copy of full final messages.
- Final-review reruns started from reviewed head
  `584f36048d00688b2ab88d2ae8be71eb9e686d45` plus the uncommitted correction
  wave. Their exact prompts, fresh context identifiers, and complete final
  results are in the
  [final-review evaluator evidence](2026-07-28-gg-agent-skill-router-final-review-evidence.md).

## Results

| Scenario | Baseline result | Final routed result | Evidence |
| --- | --- | --- | --- |
| Inspection only | Fail (router) | Pass | Read `SKILL.md` then only `references/setup-and-inspection.md`; proposed read-only JSON inspection. |
| Multi-commit edit | Fail (router) | Pass | Read `SKILL.md` then only `references/editing-stacks.md`; selected staged-only `gg absorb -s` with no sync. |
| Behind-base sync | Fail (router and workflow) | Pass after final-review rerun | Baseline proposed nonexistent `gg rebase --json`. The initial routed evaluator corrected that to ordinary `gg rebase` but still treated the sync summary as if it contained review, CI, behind-base, and target-branch state and used `"unchanged"` instead of `"up_to_date"`. The fresh rerun uses ordinary `gg rebase`, consumes the summary only for publication fields, reports `"up_to_date"`, and refreshes with `gg ls --refresh --json`. |
| Immutable rewrite | Fail (router) | Pass | Read editing then recovery after the reported `ImmutableTargets` error; surfaced merged PR #41 and required approval. |
| Remote-touching undo | Fail (router) | Pass | Read `SKILL.md` then only `references/recovery.md`; surfaced `gh pr close 52` and stopped for approval. |
| Ambiguous landing authority | Fail (router) | Pass | Read `SKILL.md` then only `references/landing-and-cleanup.md`; asked for immediate landing confirmation. |
| GitLab merge train | Fail (router) | Pass | Read `SKILL.md` then only `references/landing-and-cleanup.md`; described polling as pending, not complete. |
| Negative activation | Pass | Pass | Read no skill file and introduced neither gg nor stacked diffs. |

Auditable final-review excerpt for scenario 3: “Consume the final JSONL
`summary` only for publication results,” then use refreshed JSON for current
review, CI, approval, and behind-base gates. Earlier excerpts remain useful for
routing and authority evidence, but the initial scenario-3 workflow result is
superseded rather than presented as fully correct.

## Final-review focused cases

| Case | Result | Evidence |
| --- | --- | --- |
| Effective configured admin | Pass | Ordinary landing confirmation did not authorize effective `land_admin=true`; the evaluator stopped for separate admin-bypass approval. |
| Corrected sync parsing | Pass | Used ordinary `gg rebase`, exact `"up_to_date"`, publication-only summary consumption, and provider-backed refresh. |
| Setup/worktree fallback | Pass after focused rerun | Used concrete version/config/provider/repository/worktree/stack/HEAD inspection, then the printed path for an explicit `cd`; final wording truthfully remained proposed-only. |

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
| Configured admin could bypass the authority gate | `skills/gg/references/landing-and-cleanup.md` | Inspect effective `land_admin` immediately before landing and require separate explicit bypass approval. | Stopped with ordinary landing authorized but admin bypass unauthorized. |
| Sync summary fields/action terminology were overstated | `skills/gg/references/syncing-and-reviews.md` | Limit summary consumption to publication fields, use `"up_to_date"`, and refresh provider state afterward. | Correctly separated publication evidence from review, CI, approval, behind-base, mergeability, and target-branch state. |
| Setup omitted concrete inspection and parent-shell worktree fallback | `skills/gg/references/setup-and-inspection.md` | Add exact version/config/repository/worktree/stack/HEAD inspection and `cd <printed-worktree-path>` fallback. The first rerun exposed a reporting loophole, so the owning report contract now also forbids completion claims before execution and observed post-`cd` state. | The focused rerun selected the concrete fallback and ended: “Proposed only: no inspection, setup, checkout, or directory change was executed.” |

The structural contract was updated in `crates/gg-cli/tests/skill_contract.rs`
for the refined frontmatter. Its expected string was changed first and the
test failed against the old description, then passed after the skill change.
The frontmatter correction itself required no workflow reference change; the
three corrections above changed their owning workflow references.

## Outcome

After the final-review correction, all eight workflow scenarios pass the
fallback rubric for activation, reference selection, authority, structured
output, verification, and truthful status. The initial scenario-3 answer is
retained as a failure, not rewritten into a pass. All required activation
categories meet their expected result, including the corrected ambiguous
rerun. The unauthenticated Claude CLI remains a limitation: it did not enter an
evaluator turn, so it cannot supply independent tool-read streams in this
environment.

## Validation

Initial forward validation exited 0:

```text
rtk cargo fmt --all
rtk cargo test -p gg-cli --test skill_contract
rtk uvx --from 'git+https://github.com/agentskills/agentskills.git#subdirectory=skills-ref' skills-ref validate skills/gg
rtk git diff --check
```

`skill_contract`: 1 passed. `skills-ref`: `Valid skill: skills/gg`.
