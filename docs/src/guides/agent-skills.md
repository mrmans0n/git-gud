# Agent Skills Plugin

git-gud ships as a [Claude Code plugin](https://code.claude.com/docs/en/plugins) and follows the open [Agent Skills](https://agentskills.io) standard. This means AI coding agents — Claude Code, Cursor, Gemini CLI, OpenAI Codex, and others — can learn to use `gg` for stacked-diff workflows.

## What's included

The plugin provides two skills:

| Skill | Description |
|-------|-------------|
| `gg-github` | Use gg with GitHub PRs (`gh` CLI) |
| `gg-gitlab` | Use gg with GitLab MRs (`glab` CLI, merge trains) |

Each skill includes:

- **SKILL.md** — concise instructions with agent operating rules
- **reference.md** — full command reference with JSON output schemas
- **examples/** — step-by-step workflow walkthroughs

## Using with Claude Code

Load the plugin from the git-gud repo:

```bash
claude --plugin-dir /path/to/git-gud
```

Then use the skills as slash commands:

```
/git-gud:gg-github
/git-gud:gg-gitlab
```

Or let Claude pick them up automatically when you ask about stacked diffs or PRs.

## Using with other tools

Any tool supporting the Agent Skills standard can consume the skills from the `skills/` directory. The `SKILL.md` files use standard YAML frontmatter with `name` and `description` fields.

## Agent operating rules

The skills enforce several safety rules for AI agents:

1. **Never land without user confirmation** — agents must always ask before merging
2. **Always use `--json`** for parseable output
3. **Prefer worktrees** (`gg co -w`) for isolation
4. **Never `git add -A` blindly** — review and stage specific files only
5. **Verify CI + approval** before suggesting to land

## File structure

```
.claude-plugin/
  plugin.json           # Plugin manifest
skills/
  gg-github/
    SKILL.md            # GitHub skill
    reference.md        # Command reference + JSON schemas
    examples/
      basic-flow.md     # Simple feature workflow
      multi-commit.md   # Absorb, reorder, lint
  gg-gitlab/
    SKILL.md            # GitLab skill
    reference.md        # Command reference + merge trains
    examples/
      basic-flow.md     # Simple feature workflow
      merge-train.md    # Merge train workflow
```
