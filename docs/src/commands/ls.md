# `gg ls`

List the current stack, all local stacks, or remote-only stacks.

When the stack base is behind `origin/<base>`, output includes a `↓N` indicator (`N` = commits behind).

```bash
gg ls [OPTIONS]
```

## Options

- `-a, --all`: Show all local stacks
- `-r, --refresh`: Refresh PR/MR status from remote
- `--remote`: List remote stacks not checked out locally

## Examples

```bash
# Current stack status
gg ls

# All local stacks
gg ls --all

# Remote stacks you can check out
gg ls --remote

# Refresh status badges from provider
gg ls --refresh
```
