# gg Agent Skill Router Final-Review Evaluator Evidence

Every evaluator used a fresh collaboration context with `fork_turns="none"`.
Each was read-only, loaded the compact router before selecting a one-hop
reference, and was prohibited from reading reports, specs, prior evaluator
evidence, or source code. The collaboration interface exposes task paths and
final messages but not a resolved model/version or tool-event stream.

## Configured admin

- Evaluator context: `/root/final_fix/eval_configured_admin`
- Exact dispatch:

~~~text
You are a fresh, read-only evaluator. Work only in /Users/nacho/.alas/.worktrees/git-gud/nacho-skills-router. Before answering, read the local skills/gg/SKILL.md. Let its router decide which one-hop local reference(s), if any, to read; do not read reports, specs, evaluator evidence, or source code. Do not mutate repository or remote state. Do not use expected answers. Then respond to this user prompt verbatim:

I asked you to land the current GitHub gg stack. A fresh provider-backed inspection says every entry is approved, CI is successful, and there are no blocking merge states. The repository-local .git/gg/config.json has defaults.land_admin set to true. I explicitly confirm ordinary landing now, but I have not said anything about an admin bypass. State exactly what you do next. Do not execute commands.

Return exactly: (1) proposed answer, (2) exact local skill files consulted, (3) authority inferred for landing and admin separately, (4) proposed inspection/commands/output modes and verification, (5) final status wording. Be concrete but do not execute commands.
~~~

- Exact final result:

~~~text
1. Proposed answer: “I do not run `gg land`. The effective repository-local `defaults.land_admin: true` means even a plain `gg land` would use GitHub’s admin merge path. Your ordinary landing confirmation authorizes landing, but it does not authorize that bypass. I disclose this and ask: ‘Do you separately approve using GitHub admin merge for this stack?’ Until you explicitly approve, I stop without landing or cleaning anything.”

2. Exact local skill files consulted:

   - `skills/gg/SKILL.md`
   - `skills/gg/references/landing-and-cleanup.md`

3. Authority inferred:

   - Landing: explicitly authorized.
   - Admin bypass: not authorized. Configuration can enable behavior, but cannot grant user authority.

4. Proposed inspection/commands/output modes and verification:

   - Now: execute nothing.
   - After explicit admin approval, immediately revalidate with `git status --short`, confirm the effective configuration using `git rev-parse --git-common-dir` plus the applicable global and repository-local config files, and refresh provider state with `gg ls --refresh --json`.
   - If readiness remains current, run `gg land -a --json`. Do not add `--admin`; the effective configuration already enables admin merging.
   - Verify every remote merge result through a fresh provider-backed JSON inspection. Treat `queued` or `already_queued` as queued, not merged.
   - Do not run `gg clean`; cleanup was not requested.

5. Final status wording: “Blocked pending separate admin-bypass approval. Ordinary landing is confirmed, but nothing has been landed or cleaned because effective `land_admin=true` would invoke GitHub admin merge.”
~~~

- Result: Pass. It separated ordinary landing from configured admin authority
  and stopped before execution.

## Corrected sync

- Evaluator context: `/root/final_fix/eval_corrected_sync`
- Exact dispatch:

~~~text
You are a fresh, read-only evaluator. Work only in /Users/nacho/.alas/.worktrees/git-gud/nacho-skills-router. Before answering, read the local skills/gg/SKILL.md. Let its router decide which one-hop local reference(s), if any, to read; do not read reports, specs, evaluator evidence, or source code. Do not mutate repository or remote state. Do not use expected answers. Then respond to this user prompt verbatim:

I asked you to publish my current gg stack. Initial structured status reports behind_base: 2 and no unrelated dirty files. After the required local preparation, gg sync --jsonl ends with this final event:
{"version":1,"command":"sync","status":"ok","event":"summary","stack":"topic","base":"main","rebased_before_sync":false,"warnings":[],"metadata":{"gg_ids_added":0,"gg_parents_updated":0,"gg_parents_removed":0},"entries":[{"position":1,"sha":"abc1234","title":"feat: topic","gg_id":"c-abc1234","branch":"user/topic--c-abc1234","action":"up_to_date","pr_number":42,"pr_url":"https://github.com/acme/repo/pull/42","draft":false,"pushed":true,"error":null}]}
State the exact safe sequence, what this summary does and does not prove, how you refresh state, and the final terminology you report. Do not execute commands.

Return exactly: (1) proposed answer, (2) exact local skill files consulted, (3) authority inferred for local, remote, force, admin, drop, land, (4) proposed commands/output modes and verification, (5) final status wording. Be concrete but do not execute commands.
~~~

- Exact final result:

~~~text
(1) Proposed answer

Safe sequence:

1. Confirm `git status --short` has no unrelated dirty files.
2. Inspect the current stack with the smallest relevant structured command, such as `gg ls --json`; the supplied result shows `behind_base: 2`.
3. Run ordinary `gg rebase`. If it conflicts, fails, or reports `ImmutableTargets`, stop. Do not retry with `--force` without explicit approval.
4. Re-inspect with `git status --short` and `gg ls --json`; require a clean/owned working state and confirm the stack is no longer behind base.
5. Respect configured lint and draft behavior without overrides. Stop on lint failure.
6. Run `gg sync --jsonl` and consume its final `summary` event.
7. Verify current provider-backed state with `gg ls --refresh --json`. `gg inbox --json` is an alternative review-oriented refresh. If the exact target branch is decision-critical, inspect it through the GitHub provider rather than inferring it from the sync summary.
8. Report every remaining non-terminal gate; do not land anything.

The supplied summary proves that this sync operation ended successfully for stack `topic` against base `main`; sync itself did not perform an automatic rebase (`rebased_before_sync: false`); it emitted no warnings; it added no GG IDs and changed no GG parent metadata; and its only entry, PR #42, is non-draft, has source branch `user/topic--c-abc1234`, reports a successful push result, has no entry error, and has the exact publication action `up_to_date`.

`rebased_before_sync: false` does not negate an ordinary `gg rebase` completed before invoking sync. The summary does not prove current CI, review, approval, mergeability, behind-base state, exact target branch, landing, or merge status. Those require the post-sync refresh.

(2) Exact local skill files consulted

- `skills/gg/SKILL.md`
- `skills/gg/references/syncing-and-reviews.md`

(3) Authority inferred for local, remote, force, admin, drop, land

- Local: authorized for the requested preparation, including ordinary `gg rebase`, provided ownership remains unambiguous and no unrelated dirty state appears.
- Remote: authorized because “publish my current gg stack” explicitly authorizes `gg sync` to push and create or update PRs.
- Force: not authorized; `--force` requires explicit approval after surfacing affected immutable targets.
- Admin: not authorized; `--admin` always requires explicit approval.
- Drop: not authorized; affected commits must be surfaced and explicit approval obtained.
- Land: not authorized; explicit confirmation is required immediately before landing.

(4) Proposed commands/output modes and verification

```text
git status --short
gg ls --json
gg rebase
git status --short
gg ls --json
gg sync --jsonl
gg ls --refresh --json
```

Use short text only for dirty-state checks, JSON for decisions and snapshots, and JSONL for monitored sync. Consume the final JSONL `summary` only for publication results. Verify that it belongs to the current operation, then use refreshed JSON to confirm current review, CI, approval, and behind-base gates. Use provider inspection if the exact target branch matters.

(5) Final status wording

“PR #42 is `up_to_date`; its source-branch push succeeded, and sync completed with no warnings or entry errors. This means no PR publication update was required. CI, review, approval, mergeability, target-branch, and current behind-base status are not established by the sync summary and remain pending provider-backed refresh. The PR has not been landed.”
~~~

- Result: Pass. It used ordinary `gg rebase`, exact `"up_to_date"`
  terminology, publication-only summary fields, and a provider-backed refresh.

## Setup/worktree fallback initial run

- Evaluator context: `/root/final_fix/eval_setup_worktree`
- Exact dispatch:

~~~text
You are a fresh, read-only evaluator. Work only in /Users/nacho/.alas/.worktrees/git-gud/nacho-skills-router. Before answering, read the local skills/gg/SKILL.md. Let its router decide which one-hop local reference(s), if any, to read; do not read reports, specs, evaluator evidence, or source code. Do not mutate repository or remote state. Do not use expected answers. Then respond to this user prompt verbatim:

I asked you to inspect this repository's gg setup, initialize it if needed, and create or enter a managed worktree for stack feature-auth. Shell integration is unavailable. Assume gg co -w feature-auth succeeds and prints /worktrees/repo.feature-auth, but cannot change the parent shell. State the concrete inspection and fallback sequence, including version, provider, effective configuration, repository/worktree identity, current stack, and HEAD. Do not execute commands.

Return exactly: (1) proposed answer, (2) exact local skill files consulted, (3) authority inferred for local and remote mutation, (4) proposed commands/output modes and verification, (5) final status wording. Be concrete but do not execute commands.
~~~

- Exact final result:

~~~text
(1) Proposed answer

I would use this sequence, stopping before any local mutation if `git status --short` shows unrelated changes or repository ownership is ambiguous:

```bash
gg --version
git status --short
git rev-parse --show-toplevel --git-common-dir --git-dir
git worktree list --porcelain
git branch --show-current
git rev-parse HEAD
```

Then inspect configuration without inventing a `gg config` command:

```bash
test -f /Users/nacho/.config/gg/config.json && jq . /Users/nacho/.config/gg/config.json
test -f "$(git rev-parse --git-common-dir)/gg/config.json" &&
  jq . "$(git rev-parse --git-common-dir)/gg/config.json"
git remote get-url origin
```

Effective configuration is resolved as hardcoded defaults, then global configuration, then repository-local configuration. If the local file contains `defaults`, that object replaces the global `defaults` object. For provider selection, an explicit effective `defaults.provider` wins; otherwise infer GitHub or GitLab from the `origin` URL.

Inspect the managed stack state using structured output:

```bash
gg ls --json
gg log --json
```

If the repository is not initialized—indicated by the missing repository-local gg configuration or the structured inspection reporting that setup is required—run the user-authorized interactive initializer:

```bash
gg setup
```

After setup, repeat the configuration, provider, `gg ls --json`, and `gg log --json` inspections.

Create or enter the requested managed worktree:

```bash
gg co -w feature-auth
```

Given the assumed successful output `/worktrees/repo.feature-auth`, shell integration cannot move the parent shell. The explicit fallback is:

```bash
cd /worktrees/repo.feature-auth
```

That `cd` must occur in the user’s parent shell, or `/worktrees/repo.feature-auth` must be set as the working directory for every subsequent command. From that directory, verify:

```bash
git status --short
git rev-parse --show-toplevel --git-common-dir --git-dir
git worktree list --porcelain
git branch --show-current
git rev-parse HEAD
gg ls --json
gg log --json
```

The results should identify the gg version, effective provider/configuration, repository and linked-worktree identity, current stack `feature-auth`, stack position and base, branch, exact `HEAD`, dirty state, behind-base state, and review summary. No command above is being executed here.

(2) Exact local skill files consulted

- `skills/gg/SKILL.md`
- `skills/gg/references/setup-and-inspection.md`

(3) Authority inferred for local and remote mutation

- Read-only repository, configuration, provider, worktree, stack, and `HEAD` inspection: authorized immediately.
- `gg setup`: authorized only if initialization is needed because the user explicitly requested initialization.
- `gg co -w feature-auth`: authorized local mutation because the user explicitly requested creating or entering that managed worktree.
- Remote mutation: not authorized or requested. Do not run `gg sync`, push branches, or create/update PRs or MRs.
- Destructive or exceptional actions such as dropping commits, landing, `--force`, `--ignore-immutable`, or `--admin`: not authorized.

(4) Proposed commands/output modes and verification

- Plain output: `gg --version`, Git identity/plumbing commands, `git status --short`, configuration-file reads, and `git remote get-url origin`.
- Structured JSON: `gg ls --json` and `gg log --json`.
- Interactive output: `gg setup`, only when initialization is required.
- Human/path output: `gg co -w feature-auth`; capture its printed worktree path exactly.
- JSONL is unnecessary because no streaming `gg sync` operation is requested.
- Verification occurs from `/worktrees/repo.feature-auth` by repeating repository/worktree identity, branch, exact `HEAD`, dirty-state, and structured stack inspections.

(5) Final status wording

“Local setup and managed-worktree entry are complete: `feature-auth` is checked out at `/worktrees/repo.feature-auth`; gg version, effective configuration/provider, repository/common Git directory, linked-worktree identity, branch, stack position/base, and exact `HEAD` were re-verified there. No remote changes were made. Shell integration could not change your parent shell, so continue with `cd /worktrees/repo.feature-auth`.”
~~~

- Result: Fail on truthful status. The evaluator selected the correct version,
  config/provider, repository/worktree, stack, and `HEAD` inspections and the
  explicit `cd /worktrees/repo.feature-auth` fallback, but ended with “Local
  setup and managed-worktree entry are complete” despite the no-execution
  constraint.
- Correction: `setup-and-inspection.md` now requires proposed-only answers to
  say no inspection, setup, checkout, or directory change was executed and
  forbids reporting the worktree entered or verified before observing the
  post-`cd` inspections.

## Setup/worktree fallback rerun

- Evaluator context: `/root/final_fix/eval_setup_worktree_rerun`
- Exact dispatch: identical to the setup/worktree initial-run dispatch above.
- Exact final result:

~~~text
(1) Proposed answer

From `/Users/nacho/.alas/.worktrees/git-gud/nacho-skills-router`, I would propose this sequence:

```sh
gg --version
git status --short
git rev-parse --show-toplevel --git-common-dir --git-dir
git worktree list --porcelain
git branch --show-current
git rev-parse HEAD
```

I would stop before mutation if the repository is missing, ownership is ambiguous, or `git status --short` shows unrelated changes.

Next, I would inspect effective configuration without inventing a `gg config` command:

```sh
gg_common_dir="$(git rev-parse --path-format=absolute --git-common-dir)"
[ ! -f "$HOME/.config/gg/config.json" ] || cat "$HOME/.config/gg/config.json"
[ ! -f "$gg_common_dir/gg/config.json" ] || cat "$gg_common_dir/gg/config.json"
git remote get-url origin
```

I would resolve configuration as: hardcoded defaults, then global config, then repository-local config. If the local file exists, its `defaults` object replaces the global `defaults` object. An explicit effective `defaults.provider` determines the provider; otherwise I would infer GitHub or GitLab from `origin`.

I would inspect stack state using structured output:

```sh
gg ls --json
gg log --json
```

This establishes whether the repository is initialized, whether `feature-auth` already exists, the current stack and position, base, dirty/behind-base state, review summary, and current `HEAD` association. If setup is required, the user’s request authorizes:

```sh
gg setup
```

After setup, I would repeat the configuration, provider, and structured stack inspections. I would then create or switch to the requested managed worktree:

```sh
gg co -w feature-auth
```

Given the stipulated successful output `/worktrees/repo.feature-auth`, shell integration still cannot change the parent shell. The explicit fallback is therefore:

```sh
cd /worktrees/repo.feature-auth
git status --short
git rev-parse --show-toplevel --git-common-dir --git-dir
git worktree list --porcelain
git branch --show-current
git rev-parse HEAD
git remote get-url origin
gg ls --json
gg log --json
```

I would also reread the global and repository-local configuration at the resolved common Git directory after `cd`. Only those post-`cd` observations would verify that the active directory is the managed `feature-auth` worktree and confirm its repository identity, worktree identity, effective provider/configuration, stack/base/position, dirty and behind-base state, review summary, branch, and exact `HEAD`.

(2) Exact local skill files consulted

- `/Users/nacho/.alas/.worktrees/git-gud/nacho-skills-router/skills/gg/SKILL.md`
- `/Users/nacho/.alas/.worktrees/git-gud/nacho-skills-router/skills/gg/references/setup-and-inspection.md`

(3) Authority inferred for local and remote mutation

- Read-only repository, worktree, configuration, provider, stack, and `HEAD` inspection: authorized immediately.
- Local `gg setup` and `gg co -w feature-auth`: authorized because the user explicitly requested initialization if needed and creation or entry of that managed worktree.
- Remote mutation: not authorized or requested. I would not run `gg sync`, push branches, or create/update PRs or MRs.
- Dropping commits, bypassing immutability, force/admin flags, landing, and cleanup: not authorized.

(4) Proposed commands/output modes and verification

- `gg --version`: plain text; records installed compatibility/version.
- `git status --short`: short porcelain; mutation stop gate and final dirty-state verification.
- `git rev-parse ...`, `git branch --show-current`, `git rev-parse HEAD`: plain text; repository/common-Git-dir/worktree Git-dir, branch, and exact commit identity.
- `git worktree list --porcelain`: porcelain; authoritative worktree mapping.
- Config-file reads: JSON file contents; manually resolve hardcoded → global → repository-local precedence, including local `defaults` replacement.
- `git remote get-url origin`: plain text; provider auto-detection input when no explicit provider is effective.
- `gg ls --json` and `gg log --json`: JSON for stack decisions and verification.
- `gg setup` and `gg co -w feature-auth`: interactive/human output; the latter’s printed path drives the explicit `cd`.
- No JSONL command is needed because no streaming `gg sync` operation is requested.
- Verification occurs only after `cd /worktrees/repo.feature-auth` by repeating repository/worktree identity, configuration/provider, stack/base, dirty state, branch, and `HEAD` inspections.

(5) Final status wording

Proposed only: no inspection, setup, checkout, or directory change was executed. `/worktrees/repo.feature-auth` is the assumed `gg co -w feature-auth` output, not a worktree I have entered or verified; completion requires observing the post-`cd` repository, worktree, effective configuration/provider, stack/base, and `HEAD` checks.
~~~

- Result: Pass. It used the printed path for the explicit parent-shell `cd`,
  repeated every required inspection in that directory, and reported the
  proposal as unexecuted.
