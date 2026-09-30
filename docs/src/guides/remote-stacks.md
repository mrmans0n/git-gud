# Working with Remote Stacks

Use this when a stack exists on origin but not in your local checkout (new machine, pairing, takeover).

## Discover remote-only stacks

```bash
gg ls --remote
```

Active stacks are shown first. Stacks whose PRs/MRs have all been merged appear in a separate "Landed" section at the bottom, so you can focus on work that still needs attention.

## Check out a remote stack

```bash
gg co user-auth
```

If a local stack doesn't exist, git-gud uses the remote stack branch when present.
Otherwise, it reconstructs the stack from remote entry branches named
`<username>/<stack-name>--<gg-id>` and imports available PR/MR mappings. Checkout
selects the tip by Git ancestry, so the local branch includes all entries even
when GG-IDs sort in a different order. The selected stack history above its
configured base must be linear.

If checkout reports that entry branches have diverged, no single linear tip
contains all entries; joining divergent entries with a merge commit is not a
supported stack. Inspect the remote branches and resolve the divergence before
retrying. It leaves your current checkout unchanged and does not create a
partial local stack.

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
