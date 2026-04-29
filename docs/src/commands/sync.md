# `gg sync`

Push entry branches and create/update PRs/MRs for the current stack.

```bash
gg sync [OPTIONS]
```

## Options

- `-d, --draft`: Create new PRs/MRs as draft (does not affect existing PRs/MRs)
- `-f, --force`: Force push even if remote is ahead
- `--update-descriptions`: Update PR/MR descriptions from commit messages
- `--update-title`: Update PR/MR titles from commit messages
- `-l, --lint`: Run lint before sync (aborts sync on lint failure and restores repository state to the pre-sync snapshot)
- `--no-lint`: Disable lint before sync (overrides config default)
- `--no-rebase-check`: Skip checking whether your stack base is behind `origin/<base>`
- `--no-verify`: Skip the pre-push hook for pushes performed by this sync (forwards `git push --no-verify`). Opt-in per invocation; does not affect other hooks.
- `-u, --until <UNTIL>`: Sync up to target commit (position, GG-ID, or SHA)
- `--json`: Output structured JSON for automation (suppresses human/progress output)

Before pushing, `gg sync` checks whether your stack base is behind `origin/<base>`. If it is behind by at least the configured threshold, git-gud warns and suggests rebasing first (`gg rebase`).

When you run `gg sync --lint`, lint runs before any push/PR updates. If lint fails, sync aborts immediately and git-gud restores your repository to the pre-sync snapshot.

Before pushing, `gg sync` also normalizes commit metadata (`GG-ID` and `GG-Parent`) for the whole stack. This normalization is always enforced during sync (including adding missing `GG-ID` trailers) to keep stack identity and PR/MR mappings stable.

If the current stack branch has a valid stack shape but uses a different prefix
than `defaults.branch_username`, `gg sync` continues and warns that stack
discovery, listing, and saved PR/MR mappings may be inaccurate. In `--json`
mode, this message is included in `sync.warnings`.

If an existing mapped PR/MR is attached to the wrong source branch (for
example after moving commits into a new stack with `gg unstack`), providers
cannot retarget that source branch in place. `gg sync` creates a replacement
PR/MR with the correct branch, updates the local mapping, comments on the old
PR/MR, and closes it. In JSON output that entry uses action `"recreated"`.

You can control this behavior with config:

- `defaults.sync_auto_rebase` (`sync.auto_rebase`): automatically run `gg rebase` before sync when behind threshold is reached
- `defaults.sync_behind_threshold` (`sync.behind_threshold`): minimum number of commits behind before warning/rebase logic applies (`0` disables the check)

## Examples

```bash
# First publish as drafts
gg sync --draft

# Sync only first two entries
gg sync --until 2

# Refresh PR/MR descriptions after commit message edits
gg sync --update-descriptions

# Also update PR/MR titles to match commit subjects
gg sync --update-title

# Run lint as part of sync
gg sync --lint

# Skip behind-base check once
gg sync --no-rebase-check

# Machine-readable output
# (useful in scripts/agents)
gg sync --json

# Skip pre-push hooks for this sync only
gg sync --no-verify
```

## Target Branch Resolution

When computing the target branch for each PR/MR, `gg sync` walks backwards through predecessor entries and skips any that are already merged or closed. If all predecessors have been merged, the target falls back to `stack.base`. This ensures downstream MRs are correctly retargeted after an upstream MR is merged — whether merged via `gg land` or directly in the provider UI.

## PR/MR Body Ownership

When `gg sync` creates a new PR/MR, the generated description is wrapped in invisible HTML comment markers:

```
<!-- gg:managed:start -->
(generated content from commit message / template)
<!-- gg:managed:end -->
```

On subsequent syncs with `--update-descriptions` (or when `sync_update_descriptions` is enabled in config), only the content inside the managed block is regenerated. Any text you add **above or below** the markers on GitHub/GitLab is preserved across syncs.

This means you can safely:

- Add review checklists above or below the managed block
- Write reviewer notes that survive re-syncs
- Check/uncheck task boxes outside the managed section

Content **inside** the managed block (the generated description) is regenerated on every sync. If your PR template includes a checklist, place persistent checklists outside the markers after creation.

**Legacy PRs** (created before this feature) have no managed markers. `gg sync` will skip body updates for these PRs and log a warning, to avoid overwriting manual edits.

## Stack navigation comments

If `defaults.stack_nav_comments` is enabled in `.git/gg/config.json`, every
full `gg sync` (no `--until`) reconciles a managed comment on each PR/MR in
the stack. The comment shows all entries in the stack in bottom-up order,
with a 👉 marker on the entry that PR corresponds to — letting reviewers see
where they are in the chain and click through to siblings.

The comment is identified by a hidden HTML marker (`<!-- gg:stack-nav -->`)
and never touches comments git-gud didn't create. Disabling the setting and
re-syncing cleans up any previously-posted comments automatically.

Merged or closed PRs are left alone — `gg sync` never modifies comments on
historical PRs.

When running with `--json`, each entry includes an optional `nav_comment_action`
field (one of `"created"`, `"updated"`, `"unchanged"`, `"deleted"`, `"error"`)
when a reconcile decision was made.

Example JSON (shape):

```json
{
  "version": 1,
  "sync": {
    "stack": "my-stack",
    "base": "main",
    "rebased_before_sync": false,
    "metadata": {
      "gg_ids_added": 0,
      "gg_parents_updated": 1,
      "gg_parents_removed": 0
    },
    "entries": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "Add feature",
        "gg_id": "c-abc1234",
        "branch": "user/my-stack--c-abc1234",
        "action": "created",
        "pr_number": 42,
        "pr_url": "https://github.com/org/repo/pull/42",
        "draft": false,
        "pushed": true,
        "error": null
      }
    ]
  }
}
```
