# Agent Skills Plugin

git-gud ships as a [Claude Code plugin](https://code.claude.com/docs/en/plugins) and follows the open [Agent Skills](https://agentskills.io) standard. This means AI coding agents — Claude Code, Cursor, Gemini CLI, OpenAI Codex, VS Code integrations, and others — can use `gg` for stacked-diff workflows.

## What's included

The plugin provides one unified skill:

| Skill | Description |
|-------|-------------|
| `gg` | Use gg with GitHub PRs (`gh` CLI) or GitLab MRs (`glab` CLI, merge trains) |

Each skill includes:

- **SKILL.md** — concise instructions with agent operating rules
- **reference.md** — command reference and JSON schemas
- **examples/** — step-by-step workflow walkthroughs

## Installation

### 1) Claude Code marketplace (recommended)

```bash
claude plugin marketplace add https://github.com/mrmans0n/git-gud
claude plugin install git-gud
```

### 2) One-off plugin loading (CLI)

Use this when launching Claude Code directly:

```bash
claude --plugin-dir /path/to/git-gud
```

### 3) Project-level config (`.claude/settings.json`)

Use this when you want the plugin enabled by default for a repository:

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

### 4) Other Agent Skills-compatible tools

Tools that support the Agent Skills standard can load skills from the repo's `skills/` directory. In practice, this includes Claude Code, Cursor, Gemini CLI, OpenAI Codex, and other compatible agent hosts.

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
