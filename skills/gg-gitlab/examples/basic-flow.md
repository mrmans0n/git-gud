# Basic Flow (GitLab): from zero to merged

## 1) Create stack in a worktree

```bash
gg co -w add-audit-events
```

## 2) Create commits

```bash
$EDITOR src/audit/model.rs
git add -A
git commit -m "feat: add audit event model"

$EDITOR src/audit/store.rs
git add -A
git commit -m "feat: persist audit events"
```

## 3) Inspect stack

```bash
gg ls --json
```

## 4) Sync to create/update MRs

```bash
gg sync --json
```

Optionally verify in GitLab CLI:

```bash
# Use gg ls --json to see the MRs created by sync
# Fields are pr_number and pr_state (even for GitLab MRs)
gg ls --json
```

## 5) Check approvals and CI

```bash
gg ls --json
```

Ensure each MR to be landed is approved and green.

## 6) Land after user confirmation

```bash
# ask user first
gg land -a -c --json
```
