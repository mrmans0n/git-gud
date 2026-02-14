# Navigation commands

Current `gg` versions expose navigation as separate commands.

## `gg first`
Move to the first commit in the stack.

## `gg last`
Move to stack head.

## `gg prev`
Move to the previous commit.

## `gg next`
Move to the next commit.

## `gg mv <TARGET>`
Move to a specific commit by position (1-indexed), GG-ID, or SHA.

Examples:

```bash
gg first
gg next
gg prev
gg last
gg mv 1
gg mv c-abc1234
gg mv a1b2c3d
```
