# Introduction

`git-gud` (`gg`) is a stacked-diffs CLI for GitHub and GitLab.

It helps you split large changes into small, reviewable commits. Each commit maps to its own PR/MR, with dependencies automatically chained in order.

## Why use stacked diffs?

- Faster reviews through smaller changes
- Easier parallel development while waiting for feedback
- Cleaner, more logical commit history

## Provider support

`gg` supports:

- GitHub (via `gh`)
- GitLab (via `glab`)

Provider selection is auto-detected from your remote URL (`github.com` / `gitlab.com`). For self-hosted instances, run `gg setup` and choose the provider explicitly.
