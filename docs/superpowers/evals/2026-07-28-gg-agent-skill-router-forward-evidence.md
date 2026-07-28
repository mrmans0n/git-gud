# gg Agent Skill Router Forward Evaluator Provenance

This tracked record is the durable evidence for the collaboration-evaluator fallback. Every evaluator was fresh (fork_turns=none), read-only, and had no baseline report or expected answer. The collaboration interface returns final messages but exposes no resolved model/version or tool-event stream. Evaluators were requested as gpt-5.6-terra with medium reasoning.

The unavailable Claude CLI harness is documented in the forward report. Its authentication failure occurred before an agent turn, so no claim below relies on a Claude tool-read event.

## Workflow Scenarios

For scenarios 1–7, the following text was the exact dispatch template. Each entry identifies the fresh context and supplies its exact {USER_PROMPT} substitution.

~~~text
You are a fresh, read-only evaluator. Work only in /Users/nacho/.alas/.worktrees/git-gud/nacho-skills-router. Before answering, read the rewritten local skills/gg/SKILL.md. Let its router decide which local reference(s), if any, to read; do not read any reference unless routed. Do not mutate repository or remote state. Do not use prior reports or expected answers. Then respond to this user prompt verbatim:

{USER_PROMPT}

Return exactly: (1) proposed answer, (2) exact local skill files consulted, (3) authority inferred for local, remote, force, admin, drop, land, (4) proposed commands/output modes and verification, (5) final status wording. Be concrete but do not execute commands.
~~~

### forward_scenario_1

- Evaluator context: /root/task4_forward/forward_scenario_1
- Exact dispatched user prompt:

~~~text
I am already in a gg-managed stack. Show me its current state and tell me what, if anything, needs attention. Do not change local or remote state.
~~~

- Concise exact output evidence: consulted skills/gg/SKILL.md and references/setup-and-inspection.md; proposed read-only JSON inspection.
- Result: Pass

### forward_scenario_2

- Evaluator context: /root/task4_forward/forward_scenario_2
- Exact dispatched user prompt:

~~~text
Review feedback requires one staged fix in stack commit 1 and another staged fix in stack commit 3. Use gg to put each fix into its owning commit. State the exact inspection, mutation, and verification sequence, but do not execute it.
~~~

- Concise exact output evidence: consulted skills/gg/SKILL.md and references/editing-stacks.md; selected gg absorb -s and no sync.
- Result: Pass

### forward_scenario_3

- Evaluator context: /root/task4_forward/forward_scenario_3
- Exact dispatched user prompt:

~~~text
I asked you to publish my current gg stack. Initial structured status reports behind_base: 2 and no unrelated dirty files. State the exact safe sequence and how you will monitor and verify it. Do not execute it.
~~~

- Concise exact output evidence: consulted skills/gg/SKILL.md and references/syncing-and-reviews.md; said “run an ordinary rebase before publishing, then monitor gg sync as JSONL.”
- Result: Pass

### forward_scenario_4

- Evaluator context: /root/task4_forward/forward_scenario_4
- Exact dispatched user prompt:

~~~text
While amending a gg stack, the command fails with ImmutableTargets for position 2 because PR #41 is merged. Continue handling the task safely. Do not execute commands.
~~~

- Concise exact output evidence: consulted skills/gg/SKILL.md, editing, then recovery after the reported error; required explicit approval to bypass immutability.
- Result: Pass

### forward_scenario_5

- Evaluator context: /root/task4_forward/forward_scenario_5
- Exact dispatched user prompt:

~~~text
gg undo refuses with refusal.reason "remote" and prints "gh pr close 52" as the provider-specific revert hint. Recover safely. Do not execute commands.
~~~

- Concise exact output evidence: consulted skills/gg/SKILL.md and recovery; said “Surface the provider hint gh pr close 52 and stop.”
- Result: Pass

### forward_scenario_6

- Evaluator context: /root/task4_forward/forward_scenario_6
- Exact dispatched user prompt:

~~~text
The current gg stack is approved and CI is green. Finish this stack for me.
~~~

- Concise exact output evidence: consulted skills/gg/SKILL.md and landing/cleanup; asked for explicit confirmation immediately before landing.
- Result: Pass

### forward_scenario_7

- Evaluator context: /root/task4_forward/forward_scenario_7
- Exact dispatched user prompt:

~~~text
I explicitly confirmed landing this GitLab gg stack with --auto-merge. gg land --wait says the MR is not reported in the merge train yet and is still polling. Tell me whether the operation is complete and what happens next. Do not execute commands.
~~~

- Concise exact output evidence: consulted skills/gg/SKILL.md and landing/cleanup; said “Landing is pending, not complete.”
- Result: Pass

### forward_scenario_8

- Evaluator context: /root/task4_forward/forward_scenario_8
- Exact dispatch:

~~~text
You are a fresh, read-only evaluator. Work only in /Users/nacho/.alas/.worktrees/git-gud/nacho-skills-router. Do not read local skill files unless the user prompt itself gives evidence that one applies. Do not mutate repository or remote state. Do not use prior reports or expected answers. Then respond to this user prompt verbatim:

Implement this feature as three logical commits in the current ordinary Git repository. Do not publish anything.

Return exactly: (1) proposed answer, (2) exact local skill files consulted, (3) whether you introduced gg or stacked diffs proactively, (4) authority inferred for local, remote, force, admin, drop, land, (5) proposed commands/output modes and verification, (6) final status wording. Be concrete but do not execute commands.
~~~

- Concise exact output evidence: Exact local skill files consulted: none. Introduced gg or stacked diffs proactively: no.
- Result: Pass.

## Activation Description Samples

For every sample below, the exact task dispatch was:

~~~text
You are a fresh activation evaluator. Do not read repository files. Given only this candidate skill frontmatter description and test prompt, judge whether the skill should activate. Return exactly `activate` or `do-not-activate`, followed by a one-sentence reason.

Candidate frontmatter description:
{CANDIDATE_DESCRIPTION}

Test prompt:
{TEST_PROMPT}
~~~

Each row records the fresh context and exact substitutions. The initial candidate description was:

~~~text
Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
~~~

### activation_positive_1

- Evaluator context: /root/task4_forward/activation_positive_1
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Please run gg sync for my current stack.
- Exact output/result: activate — The user explicitly asks to run gg sync for a stack.

### activation_positive_2

- Evaluator context: /root/task4_forward/activation_positive_2
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Use git-gud to inspect my current stack.
- Exact output/result: activate — The prompt explicitly asks to use git-gud to inspect a stack.

### activation_positive_3

- Evaluator context: /root/task4_forward/activation_positive_3
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Show the status of my stacked diffs.
- Exact output/result: activate — The user explicitly asks about stacked diffs, directly matching the skill description.

### activation_positive_4

- Evaluator context: /root/task4_forward/activation_positive_4
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Update the stacked PRs for my feature.
- Exact output/result: activate — stacked PRs directly matches the skill’s activation criteria.

### activation_positive_5

- Evaluator context: /root/task4_forward/activation_positive_5
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: What is blocking these stacked MRs?
- Exact output/result: activate — The prompt explicitly concerns stacked MRs, which falls within the skill’s stated activation scope.

### activation_context_1

- Evaluator context: /root/task4_forward/activation_context_1
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: This current repository is a gg-managed stack. What should I check before editing?
- Exact output/result: activate — The prompt explicitly says the repository is a gg-managed stack and asks for workflow guidance before editing.

### activation_context_2

- Evaluator context: /root/task4_forward/activation_context_2
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Our active branch belongs to a gg-managed stack; summarize the review state.
- Exact output/result: activate — The prompt explicitly states the branch is a gg-managed stack and asks for review-state work.

### activation_context_3

- Evaluator context: /root/task4_forward/activation_context_3
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: The repository uses git-gud for its current stack. Explain what needs attention.
- Exact output/result: activate — The repository is explicitly described as using a git-gud stack.

### activation_context_4

- Evaluator context: /root/task4_forward/activation_context_4
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: We work in a gg stack here. Are any entries behind base?
- Exact output/result: activate — The prompt explicitly concerns a repository managed as a gg stack.

### activation_context_5

- Evaluator context: /root/task4_forward/activation_context_5
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: This repository is already managed as a gg stack. I need help deciding the next local step.
- Exact output/result: activate — The prompt explicitly says the repository is managed as a gg stack.

### activation_negative_1

- Evaluator context: /root/task4_forward/activation_negative_1
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Implement this feature as three logical commits in a normal Git repository.
- Exact output/result: do-not-activate — The prompt requests ordinary Git commits and does not mention gg, stacked diffs, or stacked PRs/MRs.

### activation_negative_2

- Evaluator context: /root/task4_forward/activation_negative_2
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Rebase my feature branch onto main with ordinary Git.
- Exact output/result: do-not-activate — The request explicitly asks for ordinary Git rather than git-gud or stacked-diff management.

### activation_negative_3

- Evaluator context: /root/task4_forward/activation_negative_3
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Review GitHub pull request #52 for correctness.
- Exact output/result: do-not-activate — Reviewing a GitHub pull request alone does not indicate gg or stacked-diff workflow.

### activation_negative_4

- Evaluator context: /root/task4_forward/activation_negative_4
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Amend the last ordinary Git commit with this typo fix.
- Exact output/result: do-not-activate — The request concerns an ordinary Git commit and does not mention gg, stacked diffs, or a gg-managed stack.

### activation_negative_5

- Evaluator context: /root/task4_forward/activation_negative_5
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Push my normal feature branch to GitHub.
- Exact output/result: do-not-activate — A normal feature-branch push does not inherently involve gg or stacked diffs.

### activation_ambiguous_1

- Evaluator context: /root/task4_forward/activation_ambiguous_1
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Stack these changes in the pantry inventory.
- Exact output/result: do-not-activate — Stack refers to pantry inventory rather than a clear git-gud or repository workflow.

### activation_ambiguous_2

- Evaluator context: /root/task4_forward/activation_ambiguous_2
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Stack these changes in the release notes draft.
- Exact output/result: do-not-activate — Stack here plausibly means arranging content in a release-notes draft, with no indication of git-gud or stacked diffs.

### activation_ambiguous_3

- Evaluator context: /root/task4_forward/activation_ambiguous_3
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Stack these changes into the slide deck.
- Exact output/result: do-not-activate — Stack these changes refers to arranging slide content and does not indicate git-gud or stacked diffs.

### activation_ambiguous_4

- Evaluator context: /root/task4_forward/activation_ambiguous_4
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Stack these changes for the board meeting.
- Exact output/result: activate — Stack these changes directly requests a stacked-diffs workflow.

### activation_ambiguous_5

- Evaluator context: /root/task4_forward/activation_ambiguous_5
- Exact candidate description: Use when a user asks to use git-gud (gg), stacked diffs, stacked PRs or MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Stack these changes into the weekly report.
- Exact output/result: do-not-activate — Stack here refers to arranging changes in a weekly report, not git-gud stacked diffs or PR/MR workflow.

### activation_ambiguous_4_rerun

- Evaluator context: /root/task4_forward/activation_ambiguous_4_rerun
- Exact candidate description: Use when a user explicitly asks to use git-gud (gg), the exact terms stacked diffs, stacked PRs, or stacked MRs, or when operating in a repository already managed as a gg stack.
- Exact test prompt: Stack these changes for the board meeting.
- Exact output/result: do-not-activate — Stack these changes is ambiguous and does not explicitly indicate git-gud or a gg-managed repository.

## Scope of This Evidence

The tracked record preserves every evaluator context identifier, dispatch template, exact user/test prompt, candidate description, and concise final evidence. The ignored SDD appendix remains a supplementary copy of full final messages; it is not needed to establish fresh context or verbatim dispatch provenance.

## Exact Final-Message Evidence

The following is the exact final-message evidence returned by the collaboration interface. It supplements the concise result lines above; no claim depends on an ignored artifact.

### forward_scenario_1

~~~text
1. Proposed answer

“I’ll inspect the active gg stack read-only: its identity and position, working-tree status, base relationship, and review inbox. I won’t modify local or remote state. I’ll report any dirty files, behind-base status, or review items that need attention.”

2. Exact local skill files consulted

- `skills/gg/SKILL.md`
- `skills/gg/references/setup-and-inspection.md`

3. Authority inferred

- Local: read-only inspection is authorized.
- Remote: no publish, PR/MR update, or other remote mutation is authorized.
- Force: not authorized; explicit approval required.
- Admin: not authorized; explicit approval required.
- Drop: not authorized; explicit approval required.
- Land: not authorized; explicit confirmation immediately before execution required.

4. Proposed commands/output modes and verification

- `rtk git status --short` — concise working-tree state.
- `rtk gg ls --json` — structured active-stack, base, provider, worktree, and `HEAD` state.
- `rtk gg log --json` — structured stack position and commit relationship.
- `rtk gg inbox --json` — structured review summary, if review attention is in scope.

Verification: re-run the relevant JSON inspection after inspection only if needed to confirm the active stack, base, provider, worktree, and `HEAD`; no mutation/re-inspection cycle is required because this is read-only.

5. Final status wording

“Read-only stack inspection complete. No local or remote state was changed. I’ll identify the active stack, dirty state, behind-base status, review summary, and any item requiring attention from the structured output.”
~~~

### forward_scenario_2

~~~text
1. Proposed answer

Inspect the staged changes and structured stack order, confirm the two owning commits are stack positions 1 and 3, and confirm neither is immutable. With both fixes already staged, run `gg absorb -s` once: it distributes each staged hunk into its matching owning commit, rewriting commits 1 and 3 while preserving stack order. Then re-inspect the stack, `HEAD`, and worktree. Do not sync; remote PRs remain unchanged.

2. Exact local skill files consulted

- `skills/gg/SKILL.md`
- `skills/gg/references/editing-stacks.md`

3. Authority inferred

- local: authorized
- remote: not authorized
- force: not authorized
- admin: not authorized
- drop: not authorized
- land: not authorized

4. Proposed commands/output modes and verification

```sh
git status --short
gg log --json
gg absorb -s
gg log --json
git rev-parse HEAD
git status --short
```

Use the JSON stack output to identify positions 1 and 3 and surface `ImmutableTargets` before the rewrite. Stop if unrelated changes exist, ownership is ambiguous, either target is immutable, or conflicts occur. Verify that the stack order remains intact, `HEAD` is at the expected top commit, and the working tree/index is clean except for any intentionally unabsorbed changes.

5. Final status wording

“Absorbed the two staged fixes into stack commits 1 and 3 locally. Stack order was rechecked; no remote state was changed. Publishing remains pending explicit authorization.”
~~~

### forward_scenario_3

~~~text
1. Proposed answer

I would first re-check the clean worktree and current structured stack state. Since it is two commits behind base, I would run an ordinary rebase before publishing, then monitor `gg sync` as JSONL. I would stop instead of overriding if gg finds immutable targets, a conflict, lint failure, authentication problem, or any terminal sync failure.

2. Exact local skill files consulted

- `skills/gg/SKILL.md`
- `skills/gg/references/syncing-and-reviews.md`

3. Authority inferred for local, remote, force, admin, drop, land

- Local: authorized for the ordinary rebase required to publish the current stack.
- Remote: authorized to publish/update PRs or MRs because publishing was explicitly requested.
- Force: not authorized; require explicit approval for `--force`.
- Admin: not authorized; require explicit approval for `--admin`.
- Drop: not authorized; require explicit approval identifying affected commits.
- Land: not authorized; require explicit confirmation immediately before landing.

4. Proposed commands/output modes and verification

```sh
git status --short
gg log --json
gg rebase
gg sync --jsonl
gg log --json
gg inbox --json
```

- Require `git status --short` to remain empty; stop on unrelated dirty files.
- Confirm the initial structured state reports `behind_base: 2` and identify every intended stack commit before changing anything.
- Run only ordinary `gg rebase`; if it reports `ImmutableTargets`, stop and request approval before considering `gg rebase --force`.
- Stream `gg sync --jsonl`, consume its final summary event, and record each PR/MR number, URL, action (`created`, `updated`, `unchanged`, or `recreated`), review/CI state, source branch, target branch, and warnings.
- Specifically surface branch-prefix warnings and any `"recreated"` source-branch remaps.
- Re-inspect local stack state and provider-backed review state afterward; confirm every intended PR/MR belongs to this operation, has the expected source/target, and the final behind-base state is current.

5. Final status wording

“Published only after an ordinary rebase of the stack that was two commits behind base. Final sync results: [created/updated/unchanged/recreated PRs or MRs and URLs]. Local and provider-backed state were rechecked; behind-base status is [value]. Remaining non-terminal gates: [CI, review, warnings, or none]. No force, admin override, commit drop, or landing action was used.”
~~~

### forward_scenario_4

~~~text
1. Proposed answer

Stop before rewriting position 2. The amend would rewrite a commit whose PR #41 is merged, so `ImmutableTargets` is the expected safety refusal. Do not use an override, drop the commit, or alter remote state. First inspect the current working tree and structured stack state; then report the immutable target and ask for explicit approval only if the user intentionally wants to bypass immutability. Otherwise, choose a new safe edit target/workflow after the user directs it.

2. Exact local skill files consulted

- `skills/gg/SKILL.md`
- `skills/gg/references/editing-stacks.md`
- `skills/gg/references/recovery.md`

3. Authority inferred

- Local amend/rewrite: implied in the original amend task, but blocked by the immutable merged target.
- Remote publish/sync: not authorized; no sync requested.
- Force / `--ignore-immutable`: not authorized; requires explicit approval.
- Admin: not authorized; requires explicit approval.
- Drop: not authorized; requires explicit approval.
- Land: not authorized; requires explicit confirmation immediately before execution.

4. Proposed commands/output modes and verification

Do not execute these now.

- `git status --short` — inspect only; stop on unrelated dirty state.
- `gg ls --json` — structured inspection of stack order, position 2, and the affected target.
- If the stack reports an interrupted operation: inspect that state before considering recovery; only resolve conflicts and use `gg continue` if there are conflicts the user has authorized resolving.
- Do not propose `gg sc --staged-only`, `gg absorb -s`, `gg sync`, `--force`, `--ignore-immutable`, `--admin`, `gg drop`, or `gg land` while PR #41 remains the immutable target and no explicit approval exists.

If an authorized rewrite is later performed, verify with structured stack inspection plus `git status --short`, confirming stack order, `HEAD`, dirty state, and that remote state remains unchanged unless a separate sync is requested.

5. Final status wording

“Blocked safely: position 2 maps to merged PR #41 and gg refused the amend with `ImmutableTargets`. No local rewrite, remote update, force/admin override, drop, or land action was performed. I need explicit approval to bypass immutability, or direction for a non-destructive alternative.”
~~~

### forward_scenario_5

~~~text
1. Proposed answer

`gg undo` correctly stopped: the undo would conflict with already-published remote state. `gg undo` changes only local refs and `HEAD`; it cannot restore or roll back a PR. Surface the provider hint `gh pr close 52` and stop. Do not run it without the user’s explicit approval, since closing PR #52 is a remote rollback decision.

2. Exact local skill files consulted

- `skills/gg/SKILL.md`
- `skills/gg/references/recovery.md`

3. Authority inferred

- Local: inspection is allowed; requested local stack edits may be allowed, but no local undo should proceed while the remote refusal remains unresolved.
- Remote: not inferred; the provider-specific rollback requires explicit approval.
- Force: requires explicit approval.
- Admin: requires explicit approval.
- Drop: requires explicit approval.
- Land: requires explicit confirmation immediately before execution.

4. Proposed commands/output modes and verification

Do not execute these yet:

```sh
git status --short
gg undo --list --json
gg log --json
```

Use JSON for the gg inspections. If the user explicitly approves closing the PR, execute the exact provider hint:

```sh
gh pr close 52
```

Then re-inspect `git status --short`, `gg undo --list --json`, and `gg log --json`; after any approved recovery, verify refs, `HEAD`, the operation record, stack order, dirty state, and the remote PR state.

5. Final status wording

Blocked safely: `gg undo` refused with `refusal.reason: "remote"`. No local or remote changes were made. The next step is awaiting explicit approval to run the provider-specific rollback hint, `gh pr close 52`.
~~~

### forward_scenario_6

~~~text
1. Proposed answer: “The stack is reported approved with green CI, but I need your explicit confirmation immediately before landing. Reply `confirm land` to run the land operation and then safely clean the landed local stack.”

2. Exact local skill files consulted: `skills/gg/SKILL.md`; `skills/gg/references/landing-and-cleanup.md`.

3. Authority inferred: local—read state now; cleanup only after a verified successful landing and within the finish-stack request. Remote—no inferred authority to merge. Force—none. Admin—none. Drop—none. Land—requires explicit immediate confirmation.

4. Proposed commands/output modes and verification: first `git status --short`, then the smallest relevant structured `gg` inspection using JSON. After confirmation: `gg land -a -c --json`; do not use `--admin`. Verify each remote merge result before cleanup; if safely merged, run `gg clean -a --json`, then re-inspect local stacks/worktrees with structured JSON.

5. Final status wording: “Awaiting explicit landing confirmation; nothing has been landed, queued, pushed, or cleaned.”
~~~

### forward_scenario_7

~~~text
1. Proposed answer: The operation is not complete. GitLab has not yet reported the MR in its merge train, so `gg land --wait` is correctly still polling. Next, it waits for GitLab to enqueue/report the MR and then for the train’s merge result. Treat both queued and already-queued as pending, not merged. Do not clean the stack until the remote merge is verified.

2. Exact local skill files consulted:
   - `skills/gg/SKILL.md`
   - `skills/gg/references/landing-and-cleanup.md`

3. Authority inferred:
   - Local: read/verification allowed; cleanup is not authorized or safe yet.
   - Remote: the explicitly confirmed landing, including GitLab `--auto-merge`, was authorized; no further remote mutation is needed while polling.
   - Force: not authorized.
   - Admin: not authorized.
   - Drop: not authorized.
   - Land: explicitly authorized, but presently non-terminal.

4. Proposed commands/output modes and verification:
   - No commands should be executed per the request.
   - If execution were requested later: use `gg land -a -c --json`; use structured JSON for decisions.
   - Verify the remote MR’s final merged result before any `gg clean -a --json`; re-inspect local stacks/worktrees only if cleanup is then authorized.

5. Final status wording: “Still polling: GitLab has not yet reported this MR in the merge train. Landing is pending, not complete; wait for GitLab to report and complete the train, then verify the remote merge before cleanup.”
~~~

### forward_scenario_8

~~~text
1. Proposed answer: I need the feature requirements before I can define or create three meaningful logical commits. I would keep all work local and make no publication changes.

2. Exact local skill files consulted: none.

3. Introduced gg or stacked diffs proactively: no.

4. Authority inferred: local commits only; no remote, force-push, admin, drop, or land authority.

5. Proposed commands/output modes and verification: after receiving the feature spec, inspect with `git status` and repository tests; implement and test each cohesive slice; create three local conventional commits with `git add` and `git commit`; then run `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features`. Use normal human-readable output.

6. Final status wording: Blocked awaiting feature requirements; no commands executed and no repository state changed.
~~~

### Activation Final Messages

#### activation_positive_1

~~~text
activate — The user explicitly asks to run `gg sync` for a stack.
~~~

#### activation_positive_2

~~~text
activate — The prompt explicitly asks to use git-gud to inspect a stack.
~~~

#### activation_positive_3

~~~text
activate — The user explicitly asks about stacked diffs, directly matching the skill description.
~~~

#### activation_positive_4

~~~text
activate — “stacked PRs” directly matches the skill’s activation criteria.
~~~

#### activation_positive_5

~~~text
activate — The prompt explicitly concerns stacked MRs, which falls within the skill’s stated activation scope.
~~~

#### activation_context_1

~~~text
activate — The prompt explicitly says the repository is a gg-managed stack and asks for workflow guidance before editing.
~~~

#### activation_context_2

~~~text
activate — The prompt explicitly states the branch is a gg-managed stack and asks for review-state work.
~~~

#### activation_context_3

~~~text
activate — The repository is explicitly described as using a git-gud stack.
~~~

#### activation_context_4

~~~text
activate — The prompt explicitly concerns a repository managed as a gg stack.
~~~

#### activation_context_5

~~~text
activate — The prompt explicitly says the repository is managed as a gg stack.
~~~

#### activation_negative_1

~~~text
do-not-activate — The prompt requests ordinary Git commits and does not mention gg, stacked diffs, or stacked PRs/MRs.
~~~

#### activation_negative_2

~~~text
do-not-activate — The request explicitly asks for ordinary Git rather than git-gud or stacked-diff management.
~~~

#### activation_negative_3

~~~text
do-not-activate — Reviewing a GitHub pull request alone does not indicate gg or stacked-diff workflow.
~~~

#### activation_negative_4

~~~text
do-not-activate — The request concerns an ordinary Git commit and does not mention gg, stacked diffs, or a gg-managed stack.
~~~

#### activation_negative_5

~~~text
do-not-activate — A normal feature-branch push does not inherently involve gg or stacked diffs.
~~~

#### activation_ambiguous_1

~~~text
do-not-activate — “Stack” refers to pantry inventory rather than a clear git-gud or repository workflow.
~~~

#### activation_ambiguous_2

~~~text
do-not-activate — “Stack” here plausibly means arranging content in a release-notes draft, with no indication of git-gud or stacked diffs.
~~~

#### activation_ambiguous_3

~~~text
do-not-activate — “Stack these changes” refers to arranging slide content and does not indicate git-gud or stacked diffs.
~~~

#### activation_ambiguous_4

~~~text
activate — “Stack these changes” directly requests a stacked-diffs workflow.
~~~

#### activation_ambiguous_5

~~~text
do-not-activate — “Stack” here refers to arranging changes in a weekly report, not git-gud stacked diffs or PR/MR workflow.
~~~

#### activation_ambiguous_4_rerun

~~~text
do-not-activate — “Stack these changes” is ambiguous and does not explicitly indicate git-gud or a gg-managed repository.
~~~

The tracked record therefore preserves the exact final-message evidence as well as every evaluator context identifier, dispatch template, exact user/test prompt, and activation candidate description. The ignored SDD appendix is redundant supplemental storage only.
