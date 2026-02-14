# FAQ / Troubleshooting

## `gh` / `glab` is not installed

Install the provider CLI:

- GitHub: https://cli.github.com/
- GitLab: https://gitlab.com/gitlab-org/cli

## Not authenticated with provider

```bash
gh auth login    # GitHub
glab auth login  # GitLab
```

## "Not on a stack branch"

Switch to or create a stack branch:

```bash
gg co <stack-name>
```

## Merge commits are not supported

Use linear history for stack branches:

```bash
git rebase main
```

## I pushed with `git push` and mappings are missing

Run reconciliation:

```bash
gg reconcile --dry-run
gg reconcile
```
