# Progressive `gg inbox` Refresh Design

## Summary

Make `gg inbox` useful immediately instead of waiting for every provider
request to finish. Human mode will show stable rows as soon as local discovery
completes and update each row as its remote state arrives. A new `--jsonl`
mode will stream machine-readable completion events while preserving `--json`
as the atomic final snapshot.

The refresh path will also reduce redundant provider requests and process up to
four PRs or MRs concurrently. The final inbox remains deterministic even though
incremental results arrive in network completion order.

## Problem

The current implementation has two independent latency problems:

1. Each PR or MR performs up to three provider calls for details, CI, and
   approval.
2. Entries and stacks are processed serially, and all results are buffered
   before the command renders its inbox.

Human mode prints only `Refreshing PR status...` or
`Refreshing MR status...` during this work. `--json` emits nothing until the
last request completes. A single slow provider call therefore delays every
result for both people and automation.

Some of the provider work is also redundant:

- GitHub's existing PR-details response already contains the review decision,
  while inbox performs a second approval request.
- GitHub CI can be requested in the same `gh pr view --json` invocation.
- GitLab's MR-details response is fetched separately for details and CI even
  though both can be parsed from the same response.

## Goals

- Show the human inbox immediately after local candidate discovery.
- Keep human rows stable while individual remote snapshots arrive.
- Stream useful machine-readable results in completion order.
- Preserve `gg inbox --json` as one deterministic aggregate document.
- Reduce provider subprocess and request count.
- Refresh at most four PRs or MRs concurrently.
- Preserve useful results when one stack or provider refresh fails.
- Keep the final grouped human inbox and existing bucket semantics.
- Establish the same JSON-versus-JSONL convention used by `gg sync`.

## Non-goals

- Do not add persistent caching or stale-while-revalidate behavior.
- Do not add a configurable concurrency flag or setting.
- Do not build provider-wide GraphQL or paginated batch fetching.
- Do not change the MCP transport to stream events.
- Do not generalize the worker pool into a repository-wide job framework.
- Do not migrate other commands to JSONL as part of this change.
- Do not change which local stacks or entries `gg inbox` discovers.

## User Experience

### Interactive human mode

Live rendering is enabled only for ordinary human mode when both stdout and
stderr are terminals. The renderer writes its temporary display to stderr so
stdout remains the home of the durable final inbox.

Immediately after local discovery, the renderer shows:

- one row for every locally known PR or MR candidate
- stable stack and position ordering
- `refreshing` as the initial state for each row
- a `refreshed N/M` counter

As a snapshot completes, only that row's status changes. Rows do not move into
action buckets during refresh. This prevents the terminal from reordering
content while the user is reading it.

Successfully refreshed entries that will be omitted from the final inbox stay
in place until the live display clears. Their temporary status reads
`merged (hidden; use --all)` or `closed (hidden)` rather than disappearing and
making the completion count look inconsistent.

After all candidates complete, the live display clears and the existing
grouped inbox is printed to stdout. The final display adds a
`Refresh failed` group before the existing action groups.

### Redirected and non-TTY output

If either output stream is not a terminal, human mode does not create or repaint
the live display. It prints only the final grouped inbox. For example,
`gg inbox > report.txt` produces a clean report without progress output.

### Empty inbox

If local discovery finds no candidates, human mode skips the live display and
prints the existing empty-inbox message immediately.

## Architecture

The command becomes a three-stage pipeline.

### 1. Local discovery

Local discovery resolves usernames, stack branches, bases, commits, configured
PR or MR numbers, and behind-base counts. It produces:

- an ordered `Vec<InboxCandidate>`
- zero or more `StackLoadError` values

`InboxCandidate` contains only immutable local identity and ordering data:

- discovery index
- stack name and position
- commit SHA and title
- PR or MR number
- behind-base count

Provider traffic does not begin until discovery is complete. This gives the
human renderer a complete, stable set of rows before any row updates.

If candidates exist, the coordinator detects the provider and checks that its
CLI is installed once before starting workers. Failure of either preflight is a
fatal command-level error because no candidate can be refreshed. The command
does not add an authentication preflight request; authentication failures from
an otherwise available provider CLI are handled as per-entry refresh errors.

### 2. Bounded remote refresh

`refresh_candidates` processes candidates with a fixed maximum of four worker
threads. The worker count is `min(4, candidate_count)`.

Workers receive immutable candidates and call
`Provider::get_inbox_snapshot(number)`. They do not write stdout or stderr.
Provider subprocess output is captured as it is today.

Each completed worker sends an `InboxCompletion` to the coordinator. A
completion contains either:

- the candidate plus a complete `InboxSnapshot`, or
- the candidate plus a provider refresh error

The refresh function is injectable in tests so concurrency and ordering can be
verified without real provider calls or timing-sensitive network behavior.

### 3. Single coordinator

The coordinator is the only owner of command output. It consumes completions
in arrival order and:

1. updates the corresponding stable live row, or emits one JSONL event
2. records the completion by discovery index
3. builds the final ordered item list after all candidates complete
4. applies `--all` filtering
5. prints or emits the deterministic final summary

Network completion order therefore affects only progressive output. Final human
and JSON output use discovery order within each action bucket.

## Component Boundaries

### `InboxCandidate`

Represents a locally discovered PR or MR. It has no provider-derived state and
can be rendered immediately.

### `InboxSnapshot`

Contains every remote field required for classification:

- remote state
- URL
- approval
- changes requested
- mergeability
- CI status

An `InboxSnapshot` is complete or the refresh is an error. The command does not
silently classify a partially refreshed entry.

`CiStatus::Unknown` is a valid completed value and is not itself an error.

### `refresh_candidates`

Owns bounded scheduling only. It does not bucket, render, serialize, or mutate
repository state.

### `LiveInboxRenderer`

Owns terminal rows, the completion counter, and clearing the temporary display.
It receives candidates and completion states but does not perform provider work
or classification.

### Classification and serialization

The existing pure bucketing and JSON-building functions remain responsible for
the final model. A provider refresh error maps to the new
`ActionBucket::RefreshFailed`, which has higher display priority than the
existing buckets.

## Provider Snapshots

### GitHub

GitHub uses one `gh pr view <number> --json ...` request per candidate. The
requested fields cover:

- number, title, state, URL, and head branch
- draft state
- mergeability
- review decision
- status-check rollup

The response is parsed once into `InboxSnapshot`. The inbox path no longer
calls `check_pr_approved` or `get_pr_ci_status` separately.

### GitLab

GitLab uses one `glab mr view <number> --output json` response for MR details
and CI. Pipeline status is parsed from the typed response instead of scanning
the raw JSON text.

GitLab then performs its approvals API request because approval is not
available in the MR-details response used by the current integration. Failure
of either call makes that candidate a refresh error. This avoids confidently
classifying an MR from incomplete review state.

### Future batching

The snapshot boundary permits later provider-wide batching without changing
the worker/coordinator or output contracts. Provider-wide GraphQL and list API
work is intentionally deferred until measurements show that per-entry
snapshots remain insufficient.

## Human Classification and Filtering

Final bucket order is:

1. `refresh_failed`
2. `ready_to_land`
3. `changes_requested`
4. `blocked_on_ci`
5. `awaiting_review`
6. `behind_base`
7. `draft`
8. `merged`

The existing first-match classification rules remain unchanged for successful
snapshots. A failed refresh is classified before remote-state rules because its
state cannot be trusted.

`--all` continues to control inclusion of successfully refreshed merged
entries. Closed entries remain excluded. A refresh failure remains visible
regardless of `--all` because the command cannot safely infer whether it should
be hidden.

## Structured Output

### Atomic JSON

`gg inbox --json` remains one pretty-printed JSON document emitted only after
refresh completes. Existing top-level fields remain:

- `version`
- `total_items`
- `buckets`
- `stack_errors`

The schema gains:

- `buckets.refresh_failed`
- optional `refresh_error` on an inbox entry

Successful entries omit `refresh_error`. `total_items` continues to mean items
included in the final inbox after state and `--all` filtering.

### Streaming JSONL

`gg inbox --jsonl` conflicts with `--json`. It suppresses human/progress output
and emits compact NDJSON on stdout, flushing after every event. Every event
contains:

- `version: 1`
- `command: "inbox"`
- `event`

The event sequence begins with `start` and, for every non-fatal run, ends with
`summary`.

#### `start`

Emitted after local discovery and before remote refresh:

```json
{"version":1,"command":"inbox","event":"start","total_candidates":5,"total_stack_errors":1}
```

#### `stack_error`

Emitted once for every stack error discovered locally:

```json
{"version":1,"command":"inbox","event":"stack_error","stack_name":"legacy","error":"base branch not found"}
```

Stack errors are emitted immediately after `start` in deterministic discovery
order.

#### `entry`

Emitted when a provider snapshot completes successfully:

```json
{"version":1,"command":"inbox","event":"entry","completed":1,"total_candidates":5,"included":true,"bucket":"ready_to_land","remote_state":"open","entry":{"stack_name":"auth","position":1,"sha":"abc1234","title":"Add login","pr_number":42,"pr_url":"https://github.com/org/repo/pull/42","ci_status":"success","behind_base":null}}
```

Every successful candidate emits an `entry` event. A merged entry without
`--all`, or a closed entry, uses `included: false` and `bucket: null`. This
keeps completion accounting exact without implying that the entry appears in
the final inbox.

#### `entry_error`

Emitted when a provider snapshot fails:

```json
{"version":1,"command":"inbox","event":"entry_error","completed":2,"total_candidates":5,"included":true,"bucket":"refresh_failed","entry":{"stack_name":"docs","position":1,"sha":"def5678","title":"Document login","pr_number":43,"behind_base":null},"error":"failed to refresh PR #43"}
```

Entry and entry-error events are intentionally emitted in network completion
order. `completed` is monotonic and reaches `total_candidates`.

#### `summary`

Emitted last after every candidate has completed:

```json
{"version":1,"command":"inbox","event":"summary","total_items":2,"buckets":{"refresh_failed":[],"ready_to_land":[],"changes_requested":[],"blocked_on_ci":[],"awaiting_review":[],"behind_base":[],"draft":[]},"stack_errors":[]}
```

The summary fields after the streaming envelope match the atomic `--json`
payload. Bucket entries use deterministic discovery order.

#### `error`

A command-level failure that prevents a useful summary emits an `error` event
and exits nonzero:

```json
{"version":1,"command":"inbox","event":"error","message":"not a git repository"}
```

The existing streaming writer behavior applies: a broken pipe stops the
producer promptly.

## Error and Exit Semantics

- Repository or configuration errors that prevent local discovery are fatal.
- Provider detection failure or a missing provider CLI is fatal when candidates
  exist.
- A broken stack becomes `stack_error`; other stacks continue.
- A provider failure becomes an `entry_error` and a final
  `refresh_failed` item; other entries continue.
- Partial refresh failure exits zero after emitting a summary.
- If every provider refresh fails, the command still exits zero with all
  candidates in `refresh_failed`.
- Process success means the inbox operation completed, not that every remote
  refresh succeeded. Structured consumers inspect `refresh_failed` and
  `stack_errors`.
- JSON and JSONL modes emit no human progress output.

## Determinism

The following are deterministic:

- local candidate discovery order
- stack-error event order
- order within final buckets
- atomic JSON
- final JSONL summary

Only per-candidate `entry` and `entry_error` event order is nondeterministic.
This is required to avoid head-of-line blocking behind a slow earlier entry.

## Testing

### Unit tests

- Preserve all existing successful bucketing cases.
- Verify refresh failures map to the highest-priority bucket.
- Parse GitHub review, mergeability, and CI from one snapshot response.
- Parse GitLab details and pipeline status from one typed MR response.
- Treat GitLab approval failure as snapshot failure.
- Verify atomic JSON omits `refresh_error` for successful entries.
- Verify deterministic final ordering from out-of-order completions.

### Concurrency tests

Use injected refresh functions plus channels and barriers to verify:

- no more than four refreshes run simultaneously
- all candidates are processed
- a fast later candidate completes before a blocked earlier candidate
- worker errors do not stop remaining work

Avoid wall-clock performance assertions and long sleeps.

### Rendering tests

Use an in-memory `indicatif::TermLike`, following the existing sync progress
tests, to verify:

- all candidate rows appear before refresh begins
- completing one candidate updates only its stable row
- the counter advances correctly
- the live display clears before final output
- non-TTY mode creates no live renderer

### JSONL tests

- `--jsonl` appears in help and conflicts with `--json`.
- Every line is valid JSON and flushed independently.
- `start` is first and `summary` is last for successful, partial, all-failed,
  and empty runs.
- Stack errors follow `start` deterministically.
- Entry events follow controlled completion order.
- Hidden merged and closed candidates report `included: false`.
- Fatal errors emit an `error` event and no human stderr output.

### Integration and regression tests

Use fake provider executables to verify provider invocation count and the full
CLI path without network access:

- GitHub performs one details/CI/review request per candidate.
- GitLab reuses one MR-details response and performs one approval request.
- Existing atomic JSON fixtures continue to parse with additive fields.
- Human GitHub `PR #N` and GitLab `MR !N` labels remain provider-specific.

## Documentation

Update:

- `docs/src/commands/inbox.md` with live behavior, `--jsonl`, event schemas,
  partial-success semantics, and `refresh_failed`
- CLI help for `--jsonl`
- the README command summary
- `docs/src/mcp-server.md` for the additive atomic JSON bucket and error field
- `skills/gg/SKILL.md` so JSONL applies to supported streaming commands rather
  than sync alone
- `skills/gg/references/setup-and-inspection.md` so agents select `--json` for
  a final inbox snapshot and `--jsonl` when incremental results matter

The MCP `stack_inbox` tool continues to invoke `gg inbox --json`.

## Verification

Before publication:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
mdbook build docs
skills-ref validate skills/gg
```

Also run the focused inbox, provider-parser, streaming-output, and skill
contract tests during implementation.

## Rollout and Compatibility

`--jsonl` and the new JSON fields are additive. Existing `--json` callers keep
receiving one complete document. Human redirected output remains final-only.
The MCP tool retains its atomic response contract.

No migration or configuration change is required.
