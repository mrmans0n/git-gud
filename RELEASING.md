# Releasing to crates.io

This document describes how to publish a new version of `gg-stack` to crates.io.

## Prerequisites

- Push access to the repository
- `CRATES_IO_TOKEN` secret configured in GitHub repository settings

## Release Process

### 1. Update version in Cargo.toml

```bash
# Bump version in Cargo.toml (e.g., 0.1.0 -> 0.1.1)
# Edit Cargo.toml manually or use cargo-edit:
# cargo set-version 0.1.1
```

### 2. Commit and push

```bash
git add Cargo.toml
git commit -m "Bump version to 0.1.1"
git push
```

> ℹ️ **Note**: `Cargo.lock` is automatically updated by a GitHub Action when you push changes to `Cargo.toml`. Wait for the "Update Cargo.lock" workflow to complete before creating the release.

### 3. Create a GitHub Release

1. Go to [Releases](https://github.com/mrmans0n/git-gud/releases)
2. Click "Draft a new release"
3. Create a new tag: `v0.1.1` (must match the version in `Cargo.toml` with `v` prefix)
4. Set the release title (e.g., `v0.1.1`)
5. Add release notes
6. Click "Publish release"

The release workflow will automatically:
- Verify the tag version matches `Cargo.toml`
- Publish to crates.io

### 4. Verify publication

Check that the package is available:
```bash
cargo search gg-stack
```

Or visit: https://crates.io/crates/gg-stack

## Troubleshooting

### "Cargo.lock contains uncommitted changes"

The "Update Cargo.lock" workflow hasn't finished yet. Wait for it to complete, then re-run the release workflow, or manually trigger a new release.

### "Version mismatch" error

The tag name doesn't match the version in `Cargo.toml`:
- Tag: `v0.1.1` → Cargo.toml should have `version = "0.1.1"`
- Tag: `v1.0.0` → Cargo.toml should have `version = "1.0.0"`

### "no token found" or authentication error

The `CRATES_IO_TOKEN` secret is missing or invalid. Update it in:
Settings → Secrets and variables → Actions → Repository secrets

## Quick Reference

```bash
# Full release workflow (example for version 0.2.0)
vim Cargo.toml                     # Set version = "0.2.0"
git add Cargo.toml
git commit -m "Bump version to 0.2.0"
git push
# Wait for "Update Cargo.lock" workflow to complete
# Then create GitHub release with tag v0.2.0
```
