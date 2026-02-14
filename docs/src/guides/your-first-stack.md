# Your First Stack

This walkthrough covers the full lifecycle: create → commit → sync → edit → land → clean.

## 1) Create a stack

```bash
gg co user-auth
```

This creates/switches to stack branch `your-user/user-auth`.

## 2) Build the feature in reviewable commits

```bash
git add . && git commit -m "Add user model"
git add . && git commit -m "Add auth endpoints"
git add . && git commit -m "Add login UI"
```

Keep each commit small and self-contained.

## 3) Inspect stack structure

```bash
gg ls
```

You should see ordered entries, each with a GG-ID.

## 4) Publish review chain

```bash
gg sync --draft
```

This pushes one branch per entry and creates one PR/MR per commit, chained by dependencies.

## 5) Address feedback in an older commit

```bash
gg mv 1
# edit files
git add .
gg sc
```

`gg sc` amends the current entry and rebases subsequent entries automatically.

## 6) Update remote PRs/MRs

```bash
gg sync
```

## 7) Land approved entries

```bash
gg land --all
```

Use `--wait` if you want git-gud to wait for CI/approvals:

```bash
gg land --all --wait
```

## 8) Cleanup

```bash
gg clean
```
