# Merge train workflow (GitLab)

Use this flow when target branch is protected by merge trains.

## 1) Prepare and sync stack

```bash
gg co -w payments-train
gg ls --json
gg sync --json
```

## 2) Request auto-merge / enqueue into train

```bash
# ask user for confirmation first
gg land -a --auto-merge -w --json
```

## 3) Monitor queue state

```bash
gg ls --json
```

For each entry, inspect:
- `approved`
- `ci_status`
- `in_merge_train`
- `merge_train_position`

Example status check with `jq`:

```bash
gg ls --json | jq '.stack.entries[] | {position,title,approved,ci_status,in_merge_train,merge_train_position}'
```

## 4) Optional MR-level checks via glab

```bash
glab mr view <iid>
glab mr checks <iid>
```

## 5) Cleanup landed stacks

```bash
gg clean -a --json
```
