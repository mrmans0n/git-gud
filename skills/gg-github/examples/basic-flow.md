# Basic Flow (GitHub): from zero to merged

## 1) Create stack in a worktree

```bash
gg co -w add-user-validation
```

## 2) Create commits

```bash
# Commit 1
$EDITOR src/validation/email.rs
git add -A
git commit -m "feat: add email validation"

# Commit 2
$EDITOR src/validation/phone.rs
git add -A
git commit -m "feat: add phone validation"
```

## 3) Inspect stack state (JSON)

```bash
gg ls --json
```

Confirm:
- `total_commits: 2`
- each entry has expected title/order

## 4) Publish PR chain

```bash
gg sync --json
```

Capture:
- `sync.entries[*].pr_number`
- `sync.entries[*].pr_url`

## 5) Refresh status after reviews/CI

```bash
gg ls --json
```

Before landing, ensure bottom entry is approved and green:
- `approved: true`
- `ci_status` indicates success

## 6) Land only after user confirmation

```bash
# Ask user first, then run:
gg land -a -c --json
```

## 7) Optional cleanup check

```bash
gg clean --json
```
