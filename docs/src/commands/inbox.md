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
```

## Buckets

`gg inbox` classifies each PR or MR into exactly one bucket, in priority order:

1. `ready_to_land`
2. `changes_requested`
3. `blocked_on_ci`
4. `awaiting_review`
5. `behind_base`
6. `draft`
7. `merged` (only with `--all`)

### Classification notes

- A canceled CI run counts as `blocked_on_ci`.
- If remote refresh fails transiently, the entry stays visible instead of disappearing, so the inbox does not look empty because of a temporary provider error.
- `behind_base` is computed from the real stack tip versus `origin/<base>`, not from the state of your local base branch.

## Example human output

```text
Inbox (3 items across 2 stacks)

Ready to land (1):
  auth #2  abc1234  Add login button  stack/auth  PR #41

Blocked on CI (1):
  auth #3  def5678  Add login API  stack/auth  PR #42 ⏳

Awaiting review (1):
  billing #1  9876abc  Add invoice export  stack/billing  PR #51
```

## JSON

With `--json`, `gg inbox` returns a versioned response designed for automation and MCP.

Example:

```json
{
  "version": 1,
  "total_items": 2,
  "buckets": {
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

## Flags

- `--all`: include items already marked as `merged`
- `--json`: emit structured output for tooling and MCP

## Relationship to other commands

- `gg ls` shows detailed status for the current stack
- `gg log` gives you a smartlog view of the current stack
- `gg inbox` is for cross-stack triage across multiple stacks
