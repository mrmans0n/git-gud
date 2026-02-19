# `gg clean`

Delete merged stacks (and associated managed worktrees).

```bash
gg clean [OPTIONS]
```

## Options

- `-a, --all`: Clean all merged stacks without prompting
- `--json`: Emit machine-readable JSON output

## Examples

```bash
gg clean
gg clean --all
gg clean --json
```

`--json` prints:
- `version`: output schema version
- `clean.cleaned`: stacks that were cleaned
- `clean.skipped`: stacks skipped (unmerged or declined in interactive mode)
