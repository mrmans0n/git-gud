---
name: gg
description: Use when a user explicitly asks to use git-gud (gg), the exact terms stacked diffs, stacked PRs, or stacked MRs, or when operating in a repository already managed as a gg stack.
---

# Operating gg

## Core principle

Use the installed `gg` CLI as runtime truth. Inspect before acting, load one
workflow reference at a time, mutate only within the user's authority, then
verify the affected local and remote state.

## Authority

| Action | Agent authority |
| --- | --- |
| Read repository and stack state | Run immediately |
| Make requested local stack edits | Allowed when implied by the task |
| Push or create/update PRs or MRs with `gg sync` | Only when publishing or syncing is requested |
| Drop commits or bypass immutability | Surface affected targets and obtain explicit approval |
| Land | Obtain explicit confirmation immediately before execution |
| Use `--force`, `--ignore-immutable`, or `--admin` | Never infer; require explicit approval |

## Inspect before acting

Run `git status --short`, then the smallest relevant structured gg inspection.
Stop on unrelated dirty state or ambiguous ownership.

## Route by goal

| User intent | Primary reference |
| --- | --- |
| Initialize, reconfigure, configure shell integration or completions, enter, navigate within, or inspect a stack | [setup and inspection](references/setup-and-inspection.md) |
| Amend, absorb, split, reorder, unstack, drop, rebase, restack, reconcile, lint, or run per-commit commands | [editing stacks](references/editing-stacks.md) |
| Publish, update, or monitor PRs or MRs | [syncing and reviews](references/syncing-and-reviews.md) |
| Land or clean stacks | [landing and cleanup](references/landing-and-cleanup.md) |
| Resolve conflicts, undo, or recover interrupted work | [recovery](references/recovery.md) |
| Integrate a native client or MCP surface | [native clients](references/native-clients.md) |

Read one primary reference at a time. If the outcome spans phases, read the next
reference only when entering that phase. Read recovery additionally only after
an error or interrupted state.

## Shared execution contract

- Use `gg <command> --help` for installed flags.
- Use JSON for decisions and JSONL for streaming sync.
- Prefer worktrees for newly created stacks.
- Stage explicit reviewed files; never blindly stage all files.
- Surface `ImmutableTargets` before requesting an override.
- Re-inspect after mutation.

## Verify and report

Report completed actions, remote effects, current blockers, and any approval
still required. Never describe pending CI, review, merge-train, or merge state
as complete.
