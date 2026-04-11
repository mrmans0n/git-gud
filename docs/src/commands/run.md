# `gg run`

Run an arbitrary command on each commit in the stack. Like `jj run` in Jujutsu — useful for validating changes (build, test), applying auto-fixers (formatters, codemods), or any per-commit shell command.

```bash
gg run [OPTIONS] -- <COMMAND>...
```

Arguments after `--` are passed through as-is. Use `--` whenever your command has its own flags so clap knows where `gg run`'s options end.

## Modes

`gg run` has three modes that control what happens to files the command modifies:

- **Read-only** (default): if the command modifies tracked files, the commit is marked failed. Ideal for build/test/lint validation.
- `--amend`: stage and fold any modifications into the current commit, then rebase the rest of the stack on top. Ideal for formatters and codemods (`cargo fmt`, `prettier`, `ruff --fix`).
- `--discard`: revert any working-tree changes after each commit. Ideal for commands with known side effects you want to ignore.

## Options

- `-u, --until <UNTIL>`: stop at this commit position (default: current).
- `--amend`: fold file changes into each commit (see above).
- `--discard`: discard file changes after each commit.
- `--keep-going`: continue on command failure instead of stopping at the first failed commit (default is to stop).
- `-j, --jobs <N>`: number of parallel workers. `0` = auto-detect CPUs, `1` = sequential (default). Parallel mode only applies to read-only runs and uses isolated temporary worktrees, one per commit.
- `--json`: emit structured JSON output instead of human-readable text.

## Examples

```bash
# Read-only validation across the whole stack
gg run -- cargo test

# Apply a formatter and fold changes into each commit
gg run --amend -- cargo fmt

# Run across up to commit 2 only
gg run --until 2 -- cargo check

# Parallel read-only run with 4 workers
gg run -j 4 -- cargo test

# Continue past failing commits (useful for auditing)
gg run --keep-going -- cargo clippy

# Pass a command with quoted arguments — argv boundaries are preserved
gg run -- sh -c 'echo "$GG_CURRENT_COMMIT" && cargo test --test integration'
```

## JSON output

With `--json`, `gg run` emits a `RunResponse`:

```json
{
  "version": 1,
  "run": {
    "results": [
      {
        "position": 1,
        "sha": "abc1234",
        "title": "Add feature",
        "passed": true,
        "commands": [
          {"command": "cargo test", "passed": true, "output": null}
        ]
      }
    ],
    "all_passed": true
  }
}
```

The `command` field is a copy-pasteable shell form: arguments containing whitespace or special characters are single-quoted. It reflects your input, not the actual argv.

## Notes

- Shell aliases (`alias gw=./gradlew`) are not expanded. Use the real command path.
- `.git/` paths in the command (e.g. `.git/gg/lint.sh`) are rewritten to the real git commondir so they resolve inside linked worktrees and inside parallel worker worktrees.
- Parallel mode creates temporary worktrees under `$TMPDIR/gg-run-<pid>/`, cleaned up automatically when the run finishes.
- Under `--amend` + default stop-on-error, if a later commit fails, the already-amended commits above are preserved — the branch is not force-reset. Use `gg continue` / `gg abort` to resume if a rebase conflict happened instead.
