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

Immediately after queueing, GitLab can temporarily omit an MR from the train listing. If `gg land --wait` says GitLab has not reported the MR in the train yet, keep polling unless the command reports a terminal error such as closed, skipped, failed CI, timeout, or repeated API errors.

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
