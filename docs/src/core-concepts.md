# Core Concepts

## Stack model

A stack is an ordered list of commits on top of a base branch.

- First commit targets the base branch (`main`, `master`, etc.)
- Each later commit targets the previous commit branch

This produces a dependency chain of PRs/MRs.

## Branch naming

- Stack branch: `<username>/<stack-name>`
- Per-commit branch: `<username>/<stack-name>--<entry-id>`

Example:

- `nacho/user-auth`
- `nacho/user-auth--c-abc1234`

## GG-ID trailers

Each commit carries a stable trailer:

```text
GG-ID: c-abc1234
```

GG-IDs are used to map commits to PR/MR records even after rebases and reordering.

## Navigation

Use stack navigation commands to move through commit positions:

- `gg first`
- `gg last`
- `gg prev`
- `gg next`
- `gg mv <target>`
