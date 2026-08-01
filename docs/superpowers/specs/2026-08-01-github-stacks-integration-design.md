# Native GitHub Stacks Integration Design

## Summary

Add an optional GitHub-only post-pass to `gg sync` that links git-gud's
existing pull request chain into GitHub's native Stacked PRs UI. The
integration uses the official `github/gh-stack` extension as its only mutation
backend and GitHub's documented read APIs to inspect current stack membership.

The feature defaults to `auto`. It remains best-effort and non-fatal: normal
GitHub sync continues to work without the extension or repository support, and
GitLab behavior is unchanged. Version one creates native stacks and appends new
top pull requests, but never removes or reorders existing native stack entries.

## Problem

Git-gud already creates the pull request graph required by GitHub Stacked PRs:

- one branch per git-gud stack entry
- the bottom pull request targets the configured stack base
- each higher pull request targets the active branch below it
- `gg sync` retargets pull requests after lower entries merge or close
- stable `GG-ID` metadata maps local entries to provider pull requests

Until now, GitHub did not expose a supported integration surface for telling it
that those pull requests form a native stack. GitHub now ships the public
preview `github/gh-stack` extension. Its `gh stack link` command is explicitly
designed for tools that retain ownership of their own local stack metadata.

Without integration, git-gud stacks remain structurally correct but do not gain
GitHub's native stack map, stack-aware rules and CI evaluation, or native stack
merge experience.

## Goals

- Link a fully synchronized GitHub pull request chain into a native GitHub
  stack.
- Append newly created top pull requests to a compatible existing native
  stack.
- Preserve merged lower pull requests already retained by GitHub.
- Default to automatic best-effort behavior without making `gh-stack` a hard
  dependency for ordinary GitHub usage.
- Keep every integration failure non-fatal to `gg sync`.
- Make the result explicit in human, atomic JSON, and streaming JSONL output.
- Preserve git-gud's local stack model, `GG-ID` metadata, and provider
  abstraction.
- Keep GitLab and partial `--until` syncs unchanged.

## Non-goals

- Do not replace git-gud's branch, commit, or pull request management with
  `gh-stack` local tracking.
- Do not automatically install or upgrade the `github/gh-stack` extension.
- Do not mutate GitHub Stacks through direct REST writes.
- Do not remove, reorder, split, or recreate a diverged native stack.
- Do not unstack merged, queued, closed, or open pull requests automatically.
- Do not make native-stack integration a prerequisite for `gg sync` success.
- Do not add equivalent behavior for GitLab.
- Do not change `gg land` to use native stack merging in this version.

## Configuration

Add GitHub-specific defaults alongside the existing GitLab-specific defaults:

```json
{
  "defaults": {
    "github": {
      "stacks_integration": "auto"
    }
  }
}
```

`GithubDefaults` contains `stacks_integration:
GithubStacksIntegration`. The enum serializes as lowercase and supports:

- `off`: never inspect or mutate native GitHub Stacks
- `auto`: attempt integration when the required extension and repository
  support are available
- `force`: perform the same safe reconciliation as `auto`, but surface missing
  capabilities as visible warnings

The serde and Rust defaults are `auto`. Missing `defaults.github` and missing
`stacks_integration` fields therefore enable automatic best-effort behavior for
existing configuration files.

`gg setup` presents a three-value selection only when GitHub is the effective
provider. GitLab setup preserves any existing GitHub setting without prompting
for it.

`force` does not bypass the create/append-only safety boundary, version checks,
or GitHub validation. It controls observability of capability failures; it
does not make the integration fatal.

## Backend Requirements

The mutation backend is the official `github/gh-stack` extension. The minimum
supported version is `v0.1.0`, which includes stack-number append mode.

The integration checks the installed extension and parses `gh stack
--version`. It never installs or upgrades the extension. Users enable the
backend with:

```sh
gh extension install github/gh-stack
```

GitHub's documented read APIs are used only to determine repository support,
pull request membership, and remote stack composition. All stack creation and
append mutations go through `gh stack link`.

The repository capability probe uses the Stacks list endpoint. A `404` means
the repository does not support native stacks; a successful response means the
feature is available. Other failures are backend errors rather than unsupported
capability.

## Architecture

Add a focused `github_stacks` module in `gg-core`. It owns the entire native
integration boundary:

- extension discovery and minimum-version validation
- repository support probing
- current pull request and native stack inspection
- conversion of provider responses into a small remote-state model
- pure create/append reconciliation planning
- `gh stack link` command construction and execution
- normalization of the final result for all output modes

The module exposes a single reconciliation entry point to `sync`. The sync
command supplies:

- effective `GithubStacksIntegration` mode
- configured stack base
- active pull request numbers in bottom-to-top order

The module does not load or modify git-gud configuration, commits, branches,
or stack files. It does not render terminal output or serialize JSON.

Command execution is injectable so unit tests can cover capability and backend
behavior without a real extension or network. Reconciliation planning is pure
and independently testable from subprocess execution.

## Sync Eligibility

`gg sync` evaluates native integration only after the normal per-entry push and
pull request loop. It is eligible when all of these conditions hold:

- the effective provider is GitHub
- the mode is not `off`
- the command is a full sync without `--until`
- at least two active pull requests are available
- every active entry has a resolved pull request number

An active pull request is open or draft. Merged and closed pull requests are
not supplied as local active entries because `gg sync` already chains around
them when computing bases.

`off`, GitLab, partial sync, insufficient active pull requests, or unresolved
active mappings produce an explicit skipped result where structured output is
available. They do not call GitHub or the extension.

The integration runs before navigation-comment reconciliation and final output.
The two post-passes remain independent: failure in one does not suppress the
other.

## Remote State Model

For each active pull request, the module reads its native stack membership.
Membership is either absent or contains a repository-scoped stack number and
position.

When any active pull request belongs to a native stack, the module fetches that
stack's ordered pull requests. Remote entries retain enough state to distinguish:

- merged immutable prefix entries
- open or draft active entries
- closed unmerged entries

A compatible remote stack may have a contiguous merged prefix followed only by
open or draft entries. A merged entry after an active entry, or a closed
unmerged entry anywhere in the retained remote structure, is treated as
divergence because version one cannot safely remove or reorder it.

All already-stacked active pull requests must belong to the same native stack.
Membership in multiple native stacks is divergence.

## Reconciliation Algorithm

The planner compares git-gud's local active pull request sequence with native
remote state and produces one of these decisions.

### Create

When none of the active pull requests belongs to a native stack, create one:

```sh
gh stack link --base <stack-base> <bottom-pr> ... <top-pr>
```

Only numeric pull request arguments are used. The extension therefore cannot
push a branch or create a missing pull request.

### Unchanged

When the active portion of one compatible native stack exactly matches the
local active sequence, perform no mutation.

The module may read the stack number for output, but does not invoke `gh stack
link`.

### Append

When the native active sequence is an exact ordered prefix of the local active
sequence, append only the delta:

```sh
gh stack link <stack-number> <first-new-pr> ... <new-top-pr>
```

Stack-number append mode preserves GitHub's merged immutable prefix without
relisting it or retargeting the first active pull request back onto a merged
branch.

### Diverged

Perform no mutation and return an actionable warning when:

- local order differs from native order
- the local sequence would remove a native active pull request
- an active pull request belongs to another native stack
- the native stack contains a closed unmerged middle entry
- the native stack shape is otherwise not an ordered prefix

The warning directs the user to inspect and explicitly unstack or repair the
native stack before rerunning `gg sync`. Version one never invokes `gh stack
unstack`.

## Result Model

Every evaluated integration produces a `GithubStackSyncResult` with:

- `mode`: `off`, `auto`, or `force`
- `action`: `created`, `appended`, `unchanged`, `skipped`, or `warning`
- `reason`: optional stable machine-readable reason
- `stack_number`: the native stack number when known
- `pr_numbers`: the local active pull requests considered, bottom to top
- `message`: optional human-readable detail for warnings

Stable skipped or warning reasons include:

- `disabled`
- `partial_sync`
- `insufficient_prs`
- `unresolved_prs`
- `missing_extension`
- `outdated_extension`
- `unsupported_repository`
- `diverged`
- `backend_failed`

After successful creation, the module rereads the first active pull request's
membership to obtain the new stack number. Failure of this confirmation does
not turn successful creation into a sync failure; it returns `created` with an
unknown stack number and a diagnostic message.

## Human Output

Human mode prints one concise line after native reconciliation:

- creation: `OK Created GitHub stack #N`
- append: `OK Added N PR(s) to GitHub stack #N`
- unchanged: a dim `GitHub stack #N is already up to date`
- divergence or backend failure: a yellow actionable warning

Expected `auto` skips for a missing or outdated extension and unsupported
repositories are silent. The structured result still records them.

`force` prints those capability skips as warnings. `off`, partial sync, and
insufficient pull requests do not print terminal noise in either mode.

Captured stdout and stderr from `gh` and `gh stack` are never forwarded directly
to command stdout, so atomic JSON and JSONL remain valid.

## Structured Output

Atomic `gg sync --json` adds `github_stack` to `sync`:

```json
{
  "version": 1,
  "sync": {
    "stack": "feature",
    "base": "main",
    "github_stack": {
      "mode": "auto",
      "action": "created",
      "reason": null,
      "stack_number": 7,
      "pr_numbers": [41, 42],
      "message": null
    }
  }
}
```

`github_stack` is `null` when the provider is not GitHub or sync exits before
provider detection. GitHub syncs include an explicit result, including skipped
outcomes.

`gg sync --jsonl` emits a `github_stack` event immediately after reconciliation
and repeats the result in the final summary. The event's standard `status` is:

- `ok` for created, appended, unchanged, and skipped
- `warning` for warning outcomes

The atomic document remains a final snapshot. JSONL continues to flush each
event immediately, and the final summary remains deterministic.

Warnings that are visible in human mode are also appended to the existing
summary `warnings` array. Expected silent `auto` skips are represented only by
the structured result and are not summary warnings.

## Failure Semantics

Native integration never changes the exit status of an otherwise successful
`gg sync`.

Mode-specific capability behavior is:

- `auto`: missing extension, outdated extension, or repository `404` becomes a
  silent skipped result
- `force`: the same conditions become visible warning results

Safety or execution failures are warning results in both active modes:

- incompatible remote shape
- multiple native stack memberships
- malformed API or extension output
- authentication, network, or other API errors
- non-zero `gh stack link` exit

`force` never runs an unsafe plan and never bypasses extension or repository
validation.

## Operation History and Undo

Create and append are remote mutations with no safe local inverse. Before
invoking `gh stack link`, sync marks its operation guard as remotely touched.
It remains marked even when the extension returns failure because the
subprocess may have performed partial remote work before exiting.

Unchanged and skipped decisions do not mark the operation as remotely touched.
No new inverse `RemoteEffect` is recorded because version one does not attempt
automatic native-stack rollback.

## Testing

### Configuration tests

- `GithubStacksIntegration` defaults to `auto`
- missing `defaults.github` loads as `auto`
- `off`, `auto`, and `force` round-trip through JSON
- local and global configuration merging preserves the GitHub setting
- `gg setup` prompts only for an effective GitHub provider and preserves the
  setting for GitLab

### Pure planner tests

- no membership plans create
- exact active sequence plans unchanged
- ordered remote prefix plans append with the correct delta
- merged remote prefix is preserved during unchanged and append decisions
- fully merged prior stack followed by unstacked active PRs plans a new create
- local reorder plans divergence
- local removal plans divergence
- closed unmerged native entry plans divergence
- PRs in multiple native stacks plan divergence
- fewer than two active PRs plans skip

### Backend tests

- missing and outdated extension detection
- minimum version `v0.1.0` acceptance and later version acceptance
- repository `404` maps to unsupported capability
- non-404 API failures map to backend failure
- create command uses `--base` and only numeric PR arguments
- append command uses the stack number and delta only
- extension stdout and stderr are captured
- successful creation confirms and returns the new stack number
- non-zero extension exit maps to a warning result

### Sync integration tests

Integration tests use a fake `gh` executable and cover:

- `off` performs no capability or mutation calls
- GitLab performs no GitHub Stacks calls
- `--until` performs no GitHub Stacks calls
- insufficient and unresolved active PRs do not invoke the extension
- `auto` silently skips missing, outdated, and unsupported capability
- `force` reports the same capability states as warnings
- supported create invokes the exact command once
- merged-prefix append invokes stack-number mode with only the delta
- divergence warns and performs no mutation
- backend failure remains non-fatal
- invoked create or append prevents local undo replay
- human, JSON, and JSONL output remain isolated and truthful
- JSONL emits one progressive `github_stack` event and includes the same result
  in its final summary

## Documentation

Update:

- README feature and configuration tables
- `docs/src/configuration.md`
- `docs/src/commands/setup.md`
- `docs/src/commands/sync.md`
- relevant getting-started guidance for installing `github/gh-stack`

Documentation explains that GitHub Stacked PRs are in public preview, the
extension is optional, `auto` is the default, integration is create/append
only, and divergence requires explicit user recovery.

The unified `skills/gg` agent skill remains unchanged. The CLI owns the
capability and safety decisions, and this feature does not introduce a new
agent authority boundary or multi-command workflow. The new JSON fields do not
change what an agent is authorized to do.

## Verification

Before implementation is considered complete, run:

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
mdbook build docs
```

Also verify `gh stack --version` against an isolated installation of the latest
official extension without mutating a real repository stack.

## Future Work

Potential later work, intentionally outside this design:

- explicit native-stack repair or recreate commands
- direct REST mutation fallback
- native stack merge support in `gg land`
- reconcile support after reorder, drop, split, or unstack operations
- GitHub webhook or MCP exposure of native stack identifiers
