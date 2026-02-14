# Working with Remote Stacks

Use this when a stack exists on origin but not in your local checkout (new machine, pairing, takeover).

## Discover remote-only stacks

```bash
gg ls --remote
```

## Check out a remote stack

```bash
gg co user-auth
```

If a local stack doesn't exist, git-gud can reconstruct it from remote entry branches and mappings.

## Typical collaboration loop

```bash
gg co teammate-feature
gg ls
# make changes
gg sync
```

Tips:

- Prefer `gg sync` over manual `git push` to keep mappings healthy
- If mappings drift, use [Reconciling Out-of-Sync Stacks](./reconciling.md)
