# Setup and inspection

## Preconditions

- Know the repository path.
- Read-only inspection requires no mutation authority.

## Procedure

1. Run `gg --version` when compatibility matters. Use
   `gg <command> --help` for installed command and flag availability.
2. Run `git status --short`.
3. Inspect repository, worktree, branch, and `HEAD` with
   `git rev-parse --show-toplevel --git-common-dir --git-dir`,
   `git worktree list --porcelain`, `git branch --show-current`, and
   `git rev-parse HEAD`.
4. Inspect effective configuration without inventing a `gg config` command.
   Read `~/.config/gg/config.json` and
   `<git-common-dir>/gg/config.json` when present. Resolution is hardcoded
   defaults, then global config, then repository-local config; when a local
   file exists its `defaults` object replaces global defaults. Run
   `git remote get-url origin` when provider auto-detection must be checked:
   explicit effective `defaults.provider` wins, otherwise gg detects GitHub or
   GitLab from the remote URL.
5. Match the stack scope with `gg ls --json`, `gg log --json`, or
   `gg inbox --json`.
6. Before reporting behind-base state, fetch the configured base when it is
   remote-backed and compare the stack tip with the fetched remote base:
   `git merge-base --is-ancestor origin/<base> <stack-tip>`. Do not rely on
   `gg ls` behind counts for this predicate.
7. Run `gg setup` only when the user requested initialization or
   reconfiguration. In an already initialized repository, `gg setup` may update
   existing configuration when that is the explicit request.
8. For parent-shell integration or shell completions, inspect installed help
   first with `gg init --help` or `gg completions --help`, then run only the
   requested command. Emit generated completion output or installation
   instructions unless the user explicitly authorized editing shell startup
   files.
9. Navigate within the current stack with `gg first`, `gg last`, `gg prev`,
   `gg next`, or `gg mv <target>` when the user asks to move position without
   rewriting commits. Before `gg next`, `gg last`, or forward `gg mv` from a
   detached stack entry, compare `git rev-parse HEAD` with the recorded current
   entry SHA from `gg log --json` or `gg ls --json`. If they differ, moving
   toward descendants can rebase later entries; require explicit local-history
   mutation authority and the fresh immutable-target preflight from
   [editing stacks](editing-stacks.md) for every downstream entry that can be
   replayed, or stop. Re-inspect `HEAD` and stack position afterward.
10. Create or switch stacks with `gg co -w <stack>` by default.
11. After `gg co -w`, use the worktree path printed by gg. If shell integration
   cannot change the parent shell, run `cd <printed-worktree-path>` explicitly,
   then repeat the repository, worktree, stack, and `HEAD` inspections there.

Do not reproduce setup JSON or authentication tutorials. Use CLI prompts and
the [hosted mdBook setup guide](https://mrmans0n.github.io/git-gud/commands/setup.html)
for explanatory detail.

## Stop conditions

Stop on a missing repository, unrelated dirty state before a requested
mutation, an ambiguous stack, or missing provider authentication for a remote
operation.

## Verification

Re-run the relevant structured inspection. Confirm the stack, base, provider,
worktree, and `HEAD`.

## Report

Report stack identity, position, dirty state, behind-base state, review summary,
and anything requiring attention. If commands were only proposed, say that no
inspection, setup, checkout, or directory change was executed; do not report
the setup/reconfiguration applied, or the worktree entered or verified, until
the relevant post-command inspections were observed.
