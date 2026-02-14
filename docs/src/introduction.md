# Introduction

`git-gud` (`gg`) is a stacked-diffs CLI for GitHub and GitLab.

It helps you split large changes into a sequence of small commits that reviewers can understand quickly. In git-gud, each commit in your stack maps to its own PR/MR, and dependencies are wired automatically.

## Why this workflow exists

Stacked diffs solve a common problem: features are often too large for a single review, but splitting work manually into many dependent branches is painful.

With git-gud, you can:

- Keep reviews small and focused
- Keep moving while earlier changes are in review
- Preserve clean, logical commit history
- Land big projects incrementally without long-lived feature branches

## Learn more about stacked diffs

- [Pragmatic Engineer: Stacked Diffs](https://newsletter.pragmaticengineer.com/p/stacked-diffs)
- [Graphite Guide: What are stacked diffs?](https://graphite.com/guides/stacked-diffs)

- [Introducing git-gud](https://nlopez.io/introducing-git-gud-a-stacked-diffs-cli-for-github-and-gitlab/) — the story behind this tool, by its author

## Provider support

git-gud supports:

- **GitHub** through `gh`
- **GitLab** through `glab`

Provider selection is auto-detected from your remote URL (`github.com` / `gitlab.com`). For self-hosted instances, run `gg setup` and select the provider explicitly.
