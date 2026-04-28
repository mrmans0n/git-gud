# Agent Skills

git-gud follows the open [Agent Skills](https://agentskills.io) standard. AI coding agents with shell access — including Codex, Claude Code, Cursor, Gemini CLI, VS Code integrations, and others — can use `gg` for stacked-diff workflows.

## What's included

The integration provides one unified skill:

| Skill | Description |
|-------|-------------|
| `gg` | Use gg with GitHub PRs (`gh` CLI) or GitLab MRs (`glab` CLI, merge trains) |

Each skill includes:

- **SKILL.md** — concise instructions with agent operating rules
- **reference.md** — command reference and JSON schemas
- **examples/** — step-by-step workflow walkthroughs

## Installation

### 1) Generic install (recommended)

```bash
npx skills add mrmans0n/git-gud
```

The `skills` CLI installs the skill into the agent setup it detects. If it cannot decide, it prompts you to choose where to install it.

### 2) Install for a specific agent

Use `--agent` when you already know the host you want to target:

```bash
npx skills add mrmans0n/git-gud --agent codex
npx skills add mrmans0n/git-gud --agent claude-code
npx skills add mrmans0n/git-gud --agent cursor
npx skills add mrmans0n/git-gud --agent gemini-cli
```

For a shared repository setup, run the command from the project root. For a user-level install, add `--global`.

### 3) Claude Code marketplace

Claude Code users can also install git-gud as a plugin:

```bash
claude plugin marketplace add https://github.com/mrmans0n/git-gud
claude plugin install git-gud
```

### 4) Claude Code local checkout

Use this when launching Claude Code directly from a local git-gud checkout:

```bash
claude --plugin-dir /path/to/git-gud
```

### 5) Claude Code project-level config (`.claude/settings.json`)

Use this when you want the local checkout enabled by default for a repository:

```json
{
  "plugins": [
    {
      "name": "git-gud",
      "path": "/path/to/git-gud"
    }
  ]
}
```

### 6) Manual setup for compatible tools

Tools that support Agent Skills can also load the repo's `skills/gg/` directory directly. Use this fallback if your agent does not use the `skills` CLI yet, or if you want to manage skill files yourself.

## How agents typically use gg

A practical AI-assisted stacked-diff workflow looks like this:

1. Agent creates or switches to a stack (`gg co ...`, ideally with a worktree)
2. Agent makes small commits and keeps each commit focused
3. Agent syncs the stack (`gg sync`) so PRs/MRs are created/updated in order
4. Agent iterates on review feedback (amend/reorder/re-sync)
5. Agent asks for explicit user confirmation before `gg land`

This keeps work reviewable while preserving user control over merges.

## JSON output for tool-driven agents

For machine-readable parsing, `gg` supports `--json` on key commands:

- `gg ls --json`
- `gg sync --json`
- `gg land --json`
- `gg clean --json`
- `gg lint --json`

Use these outputs in agents and automation for reliable state checks and decisions. For full response schemas, see each skill's `reference.md`.

## Safety model (required behavior)

When using AI agents with `gg`, keep these rules:

1. **Never land without explicit user confirmation**
2. **Never run `git add -A` blindly** (stage only reviewed/intended files)
3. **Prefer worktrees** for isolation (`gg co --wt`)
4. **Use structured output (`--json`)** when automation must parse command results

## Skill references

For full operational details, prompts, and examples:

- Unified skill: [`skills/gg/SKILL.md`](https://github.com/mrmans0n/git-gud/blob/main/skills/gg/SKILL.md)

## File structure

```
.claude-plugin/
  plugin.json           # Plugin manifest
skills/
  gg/
    SKILL.md            # Unified GitHub + GitLab skill
    reference.md        # Command reference + JSON schemas
    examples/
      basic-flow.md     # Provider-agnostic feature workflow
      multi-commit.md   # Absorb, reorder, lint
      merge-train.md    # GitLab merge train workflow
```
