# Multi-commit maintenance flow

Scenario: You already have a 3-commit stack synced and need to adjust multiple commits.

## 1) Open stack and inspect

```bash
gg co -w billing-refactor
gg ls --json
```

## 2) Fix only commit 1

```bash
gg mv 1
$EDITOR src/billing/parser.rs
git add <files>
gg sc
```

Return to top:

```bash
gg last
```

## 3) Distribute cross-cutting staged edits

```bash
$EDITOR src/billing/*.rs
git add <files>
gg absorb -s
```

If uncertain first:

```bash
gg absorb --dry-run
```

## 4) Reorder commits for clearer review

```bash
gg reorder -o "2,1,3"
```

## 5) Re-sync rewritten history

```bash
gg sync -f --json
```

## 6) Validate lint + status

```bash
gg lint --json
gg ls --json
```

## 7) Land when confirmed by user

```bash
# ask user explicitly first
gg land -a -c --json
```
