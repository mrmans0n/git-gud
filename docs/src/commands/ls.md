# `gg ls`

List the current stack, all local stacks, or remote-only stacks.

When the stack base is behind `origin/<base>`, output includes a `↓N` indicator (`N` = commits behind).

When showing the current stack, if the current branch has a valid stack shape
but uses a different prefix than `defaults.branch_username`, `gg ls` warns that
stack discovery, listing, and saved PR/MR mappings may be inaccurate. Rename the
branch to the configured prefix to keep stack metadata aligned. `gg ls --all`
and `gg ls --remote` do not show this warning.

```bash
gg ls [OPTIONS]
```

## Options

- `-a, --all`: Show all local stacks
- `-r, --refresh`: Refresh PR/MR status from remote
- `--remote`: List remote stacks not checked out locally. Stacks whose PRs/MRs are all merged are shown in a separate "Landed" section at the bottom with a `✓` marker
- `--json`: Print structured JSON output (for scripts and automation). Automatically performs a best-effort refresh of PR/MR state from the provider API, so `pr_state` and `ci_status` fields are populated without needing `--refresh`.

## Examples

```bash
# Current stack status
gg ls

# All local stacks
gg ls --all

# Remote stacks (active first, then landed)
gg ls --remote

# Refresh status badges from provider
gg ls --refresh

# Structured JSON for automation
gg ls --json
gg ls --all --json
gg ls --remote --json
```

## Un-integrated commits at HEAD

`gg ls` is read-only — it never mutates the stack. When you navigate to a mid-stack commit with `gg mv` and make a `git commit` (or `git commit --amend`) there, HEAD becomes detached with a commit that isn't part of the stack yet. `gg ls` detects this and shows a callout instead of silently losing the commit:

```
⚠ Un-integrated commit at HEAD (detached):
    3fb873d inserted  — sits on top of [1]
  Run `gg restack` to fold it into the stack.
```

The commit is not lost — run `gg restack` to fold it into the stack.

### `unintegrated_commits` in JSON output

When `gg ls --json` is called for the current stack, an `unintegrated_commits` array is included on the stack object when there are commits at a detached HEAD that haven't been integrated yet. The array is omitted entirely when empty.

```json
{
  "version": 1,
  "stack": {
    "name": "my-feature",
    "entries": [...],
    "unintegrated_commits": [
      {
        "sha": "3fb873d",
        "subject": "inserted",
        "sits_on_position": 1,
        "count": 1
      }
    ]
  }
}
```

Field types for each entry in `unintegrated_commits`:
- `sha`: `string` — short SHA of the un-integrated commit
- `subject`: `string` — first line of the commit message
- `sits_on_position`: `number` — position of the stack entry that this commit sits directly on top of
- `count`: `number` — total number of un-integrated commits at HEAD (useful when multiple commits were made before restacking)
