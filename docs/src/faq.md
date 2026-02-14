# Troubleshooting / FAQ

## `gh` or `glab` is missing

Install the provider CLI:

- GitHub: <https://cli.github.com/>
- GitLab: <https://gitlab.com/gitlab-org/cli>

## Not authenticated with provider

```bash
gh auth login
glab auth login
```

## "Not on a stack branch"

You're on a branch that doesn't match the stack naming scheme.

```bash
gg co <stack-name>
```

## I pushed with `git push` and now mappings are wrong

Run reconcile:

```bash
gg reconcile --dry-run
gg reconcile
```

## Merge commits are not supported

Stacks require linear history. Rebase your branch:

```bash
git rebase main
```

## `gg land --wait` times out

Increase timeout in config:

```json
{
  "defaults": {
    "land_wait_timeout_minutes": 60
  }
}
```

## When should I use `gg absorb` vs `gg sc`?

- Use `gg sc` when you're on the exact commit you want to modify.
- Use `gg absorb` when staged edits belong to multiple commits and you want git-gud to distribute them.
