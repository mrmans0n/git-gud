# CI/CD Integration

## Docs publishing with GitHub Pages

This repository includes a workflow to:

1. Build docs with `mdbook build docs`
2. Upload `docs/book` as a Pages artifact
3. Deploy with `actions/deploy-pages`

It runs on pushes to `main` when files under `docs/` change.

## PR previews

A separate workflow runs on pull requests touching `docs/`.

It uses `rossjrw/pr-preview-action` to publish previews to a PR-specific subdirectory on `gh-pages` and comments the preview URL on the PR.

## Lint in CI

`gg` can run lint before syncing using:

```bash
gg sync --lint
```

You can also configure default lint commands in `.git/gg/config.json` under `defaults.lint` and run them with:

```bash
gg lint
```
