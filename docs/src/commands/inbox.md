# gg inbox

`gg inbox` shows an actionable repository-wide triage view for all local stacks. Instead of inspecting stacks one by one, it groups PRs or MRs by what they need right now.

Use it when you want quick answers to questions like:

- which PRs are ready to land
- which ones are blocked on CI
- where changes were requested
- which stacks have fallen behind their base

## Usage

```bash
gg inbox
gg inbox --all
gg inbox --json
gg inbox --jsonl
```

## Buckets

`gg inbox` classifies each PR or MR into exactly one bucket, in priority order:

1. `refresh_failed`
2. `ready_to_land`
3. `changes_requested`
4. `blocked_on_ci`
5. `awaiting_review`
6. `behind_base`
7. `draft`
8. `merged` (only with `--all`)

### Classification notes

- A canceled CI run counts as `blocked_on_ci`.
- If remote refresh fails transiently, the entry stays visible in `refresh_failed` instead of disappearing, so the inbox does not look empty because of a temporary provider error.
- `behind_base` is computed from the real stack tip versus `origin/<base>`, not from the state of your local base branch.

## Example human output

The labels and number prefixes adapt to the detected provider.

**GitHub:**

```text
Inbox (3 items across 2 stacks)

Ready to land (1):
  auth #2  abc1234  Add login button  stack/auth  PR #41

Blocked on CI (1):
  auth #3  def5678  Add login API  stack/auth  PR #42 ⏳

Awaiting review (1):
  billing #1  9876abc  Add invoice export  stack/billing  PR #51
```

**GitLab:**

```text
Inbox (2 items across 1 stack)

Ready to land (1):
  auth #2  abc1234  Add login button  stack/auth  MR !41

Awaiting review (1):
  auth #3  def5678  Add login API  stack/auth  MR !42
```

## Live refresh

When both stdout and stderr are terminals, `gg inbox` renders one stable row
per discovered PR or MR while remote state is refreshed. Rows stay in discovery
order, the aggregate counter advances as `refreshed N/M`, and a completed row
remains visible until the refresh finishes. The live display is cleared before
the final grouped inbox is printed.

When output is redirected or piped, live progress is suppressed: stdout contains
only the final grouped inbox and stderr contains no refresh progress.

## JSON

With `--json`, `gg inbox` returns a versioned response designed for automation and MCP.

Example:

```json
{
  "version": 1,
  "total_items": 2,
  "buckets": {
    "refresh_failed": [],
    "ready_to_land": [
      {
        "stack_name": "auth",
        "position": 1,
        "sha": "abc1234",
        "title": "Add login",
        "pr_number": 42,
        "pr_url": "https://github.com/org/repo/pull/42",
        "ci_status": "success",
        "behind_base": null
      }
    ],
    "blocked_on_ci": [
      {
        "stack_name": "auth",
        "position": 2,
        "sha": "def5678",
        "title": "Add login API",
        "pr_number": 43,
        "pr_url": "https://github.com/org/repo/pull/43",
        "ci_status": "running",
        "behind_base": 2
      }
    ]
  }
}
```

### Per-entry fields

- `stack_name`: stack name
- `position`: commit position inside the stack
- `sha`: short SHA
- `title`: commit title
- `pr_number`: PR or MR number
- `pr_url`: PR or MR URL
- `ci_status`: `pending`, `running`, `success`, `failed`, `canceled`, `unknown`, or omitted
- `behind_base`: number of commits behind `origin/<base>`, or `null`
- `refresh_error`: remote-refresh error text, present for entries in `refresh_failed`

## Streaming NDJSON (`--jsonl`)

`gg inbox --jsonl` emits one flushed JSON object per line as discovery and
refresh work completes. The first event is `start`; each discovery error emits
`stack_error`; refresh completions emit `entry` or `entry_error`; and the final
`summary` is the same deterministic payload as `--json` (apart from its event
envelope). Consumers must wait for `summary` before making a complete-inbox
claim. A fatal failure before the summary emits one `error` event and exits
nonzero.

These representative lines use the exact serialized event shapes covered by
the inbox output tests:

```ndjson
{"event":"start","total_candidates":2,"total_stack_errors":1,"version":1,"command":"inbox"}
{"event":"stack_error","stack_name":"stale","error":"missing base","version":1,"command":"inbox"}
{"event":"entry","completed":1,"total_candidates":2,"included":true,"bucket":"ready_to_land","remote_state":"open","entry":{"stack_name":"auth","position":1,"sha":"abc1234","title":"Add login","pr_number":42,"pr_url":"https://github.com/org/repo/pull/42","ci_status":"success","behind_base":null},"version":1,"command":"inbox"}
{"event":"entry_error","completed":1,"total_candidates":1,"included":true,"bucket":"refresh_failed","entry":{"stack_name":"auth","position":1,"sha":"abc1234","title":"Add login","pr_number":42,"behind_base":null},"error":"provider unavailable","version":1,"command":"inbox"}
{"event":"summary","total_items":1,"buckets":{"refresh_failed":[],"ready_to_land":[{"stack_name":"auth","position":1,"sha":"abc1234","title":"Add login","pr_number":42,"pr_url":"https://github.com/org/repo/pull/42","ci_status":"success","behind_base":null}],"changes_requested":[],"blocked_on_ci":[],"awaiting_review":[],"behind_base":[],"draft":[]},"stack_errors":[{"stack_name":"stale","error":"missing base"}],"version":1,"command":"inbox"}
{"version":1,"command":"inbox","status":"error","event":"error","message":"Not in a git repository"}
```

`entry` events are emitted in refresh-completion order. An entry omitted from
the final inbox (for example, a merged entry without `--all`) has
`"included": false` and `"bucket": null`.

### `--json` vs `--jsonl`

- Use `--json` for one final, structured cross-stack snapshot.
- Use `--jsonl` when incremental results matter and a line-oriented consumer
  can process completion events before the final summary.

## Partial failures

An individual remote refresh failure does not make `gg inbox` fail: the command
exits zero and puts that entry in `buckets.refresh_failed` with a
`refresh_error`. Discovery failures are reported in `stack_errors` and are
also included in the JSONL `stack_error` events. Inspect both fields before
treating the inbox as complete.

## Flags

- `--all`: include items already marked as `merged`
- `--json`: emit structured output for tooling and MCP
- `--jsonl`: emit flushed NDJSON events as refreshes complete; conflicts with `--json`

## Relationship to other commands

- `gg ls` shows detailed status for the current stack
- `gg log` gives you a smartlog view of the current stack
- `gg inbox` is for cross-stack triage across multiple stacks
