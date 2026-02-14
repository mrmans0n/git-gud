# `gg land`

Merge approved PRs/MRs starting from the first stack commit.

```bash
gg land [OPTIONS]
```

Options:

- `-a, --all`: land all approved PRs/MRs in order
- `--auto-merge`: GitLab only; queue auto-merge
- `--no-squash`: disable squash merge
- `-w, --wait`: wait for CI/approvals before merge
- `-u, --until <UNTIL>`: land up to target commit
- `-c, --clean`: cleanup stack after landing all
- `--no-clean`: disable post-land cleanup

Examples:

```bash
gg land
gg land --all --wait
gg land --all --clean
```
