# Mid-stack commit insertion — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop `gg ls` from appearing to lose a commit made at a detached mid-stack HEAD, and make `gg restack` fold that commit into the stack while leaving HEAD on it.

**Architecture:** A shared detector in `stack.rs` recognizes "there is a commit at the detached HEAD that hasn't been integrated into the stack" using the existing nav context (`.git/gg/current_stack`). `gg ls` consumes it read-only (callout + JSON field, no mutation). `gg restack` consumes it as an early-return integration path: `git rebase --onto <head> <original> <branch>`, then re-checks-out the head commit detached and rewrites the nav context. Covers both inserted (committed on top) and amended (rewritten in place) detached commits.

**Tech Stack:** Rust, `git2`, existing `gg-core` command modules, integration tests via `CARGO_BIN_EXE_gg` in `crates/gg-cli/tests/integration_tests/`.

**Spec:** `docs/superpowers/specs/2026-06-09-mid-stack-commit-insertion-design.md`

---

## Background the engineer needs

- A stack's commits are computed by walking the parent chain from the branch ref `<user>/<stack>` down to base (`git.rs:415`). There is no stored commit list.
- `gg mv <n>` checks out a **detached HEAD** at that commit and writes a nav context file `.git/gg/current_stack` formatted `branch|position|oid` via `stack::save_nav_context` (`stack.rs:436`). `position` is **0-indexed**; `oid` is the commit you navigated to.
- `stack::read_nav_context(git_dir) -> Option<(String, usize, git2::Oid)>` reads it back (`stack.rs:453`).
- `Stack::load(repo, config)` computes `stack.entries` (each `StackEntry` has `position` (1-indexed), `oid`, `short_sha`, `title`, `gg_id`, etc.) and `stack.current_position: Option<usize>` (0-indexed index of the entry whose oid == HEAD, or `None` if HEAD isn't a stack entry).
- `git::checkout_commit(repo, &commit)` checks out a detached HEAD (used at `nav.rs:255`).
- `git2::Repository` provides `head_detached() -> Result<bool>`, `graph_descendant_of(commit, ancestor) -> Result<bool>`, and `graph_ahead_behind(local, upstream) -> Result<(usize, usize)>`.
- The existing `nav.rs::check_and_rebase_if_modified` (`nav.rs:285`) already performs `git rebase --onto <new> <orig> <branch>` for the modify-on-nav case — we are *not* changing it; we are adding a parallel, read-aware path that stays in place.

### Test setup pattern (copy this exactly)

Tests live in `crates/gg-cli/tests/integration_tests/<module>.rs` and use helpers from `crate::helpers`. Standard stack setup:

```rust
use crate::helpers::{create_test_repo, run_gg, run_git};
use std::fs;

// inside a #[test] fn:
let (_temp_dir, repo_path) = create_test_repo();

// gg config with a username (so branch names resolve)
let gg_dir = repo_path.join(".git/gg");
fs::create_dir_all(&gg_dir).unwrap();
fs::write(
    gg_dir.join("config.json"),
    r#"{"defaults":{"branch_username":"testuser"}}"#,
)
.unwrap();

// create a stack "testing" with two commits
run_gg(&repo_path, &["co", "testing"]);
run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
run_git(&repo_path, &["commit", "--allow-empty", "-m", "two"]);

// navigate to the middle (commit [1]) -> detached HEAD
let (ok, out, err) = run_gg(&repo_path, &["mv", "1"]);
assert!(ok, "mv failed: {out}{err}");

// insert a commit on top of [1]
run_git(&repo_path, &["commit", "--allow-empty", "-m", "inserted"]);
```

`run_gg` returns `(success: bool, stdout: String, stderr: String)`. `run_git` returns `(success: bool, stdout: String)`. There is also `run_git_full` returning stderr too.

---

## Task 1: `gg ls` detects and reports the un-integrated commit (read-only)

**Files:**
- Modify: `crates/gg-core/src/stack.rs` (add detector + types near the nav-context helpers, ~`stack.rs:466`)
- Modify: `crates/gg-core/src/commands/ls.rs` (`show_stack`, ~`ls.rs:521`-`670`)
- Modify: `crates/gg-core/src/output.rs` (`StackJson`, ~`output.rs:34`)
- Test: `crates/gg-cli/tests/integration_tests/ls.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/gg-cli/tests/integration_tests/ls.rs`:

```rust
#[test]
fn test_ls_reports_unintegrated_midstack_commit() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "two"]);
    let (ok, out, err) = run_gg(&repo_path, &["mv", "1"]);
    assert!(ok, "mv failed: {out}{err}");
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "inserted"]);

    // ls must NOT mutate and must surface the orphan commit.
    let (ok, out, err) = run_gg(&repo_path, &["ls"]);
    assert!(ok, "ls failed: {out}{err}");
    assert!(out.contains("Un-integrated commit"), "ls output: {out}");
    assert!(out.contains("inserted"), "ls output: {out}");
    assert!(out.contains("gg restack"), "ls output: {out}");

    // ls is read-only: the branch tip still has only the original 2 commits.
    let (_ok, log) = run_git(&repo_path, &["log", "--oneline", "testuser/testing"]);
    assert!(log.contains("two"), "branch log: {log}");
    assert!(!log.contains("inserted"), "branch must be untouched: {log}");
}

#[test]
fn test_ls_json_includes_unintegrated_commits() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "two"]);
    run_gg(&repo_path, &["mv", "1"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "inserted"]);

    let (ok, out, err) = run_gg(&repo_path, &["ls", "--json"]);
    assert!(ok, "ls --json failed: {out}{err}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    let arr = v["stack"]["unintegrated_commits"]
        .as_array()
        .expect("unintegrated_commits array present");
    assert_eq!(arr.len(), 1, "json: {out}");
    assert_eq!(arr[0]["subject"], "inserted", "json: {out}");
    assert_eq!(arr[0]["sits_on_position"], 1, "json: {out}");
}
```

Note: `serde_json` is already a dependency of the test crate (used elsewhere in these tests). If the import path differs, follow the existing usage in the test modules.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p gg-cli --test integration_tests test_ls_reports_unintegrated_midstack_commit test_ls_json_includes_unintegrated_commits`
Expected: FAIL — output lacks "Un-integrated commit" / JSON has no `unintegrated_commits`.

- [ ] **Step 3: Add the detector + types to `stack.rs`**

Add after `read_nav_context` (~`stack.rs:466`):

```rust
/// Whether the detached-HEAD commit was added on top of the navigated commit
/// (`Inserted`) or rewrites it in place (`Amended`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnintegratedKind {
    Inserted,
    Amended,
}

/// A commit (or chain of commits) made at a detached mid-stack HEAD that has not
/// yet been folded into the stack branch.
#[derive(Debug, Clone)]
pub struct UnintegratedCommit {
    /// Current detached HEAD oid (the tip of the un-integrated work).
    pub head_oid: git2::Oid,
    /// The commit we navigated to (`gg mv`), recorded in the nav context.
    pub original_oid: git2::Oid,
    /// Stack branch name from the nav context.
    pub branch_name: String,
    /// 0-indexed position of the navigated commit within the stack.
    pub saved_position: usize,
    /// 1-indexed display position the un-integrated work sits on.
    pub sits_on_position: usize,
    /// Short sha of `head_oid`.
    pub short_sha: String,
    /// First line of the head commit message.
    pub subject: String,
    /// Number of un-integrated commits (1 for an amend).
    pub count: usize,
    pub kind: UnintegratedKind,
}

/// Detect a commit made at a detached mid-stack HEAD that has not been folded
/// into the stack yet. Returns `None` when there is nothing to integrate.
///
/// Pure detection — never mutates the repository.
pub fn detect_unintegrated(
    repo: &Repository,
    stack: &Stack,
) -> Result<Option<UnintegratedCommit>> {
    let (branch_name, saved_position, original_oid) = match read_nav_context(repo.path()) {
        Some(ctx) => ctx,
        None => return Ok(None),
    };

    // Must be detached with HEAD different from the navigated commit.
    if !repo.head_detached()? {
        return Ok(None);
    }
    let head = repo.head()?.peel_to_commit()?;
    let head_oid = head.id();
    if head_oid == original_oid {
        return Ok(None);
    }

    // There must be commits above the navigated position to move over.
    if saved_position + 1 >= stack.len() {
        return Ok(None);
    }

    // Classify: committed on top (Inserted) vs rewritten in place (Amended).
    let kind = if repo.graph_descendant_of(head_oid, original_oid)? {
        UnintegratedKind::Inserted
    } else {
        let original = repo.find_commit(original_oid)?;
        let original_parent = original.parent_id(0).ok();
        let head_parent = head.parent_id(0).ok();
        if original_parent.is_some() && original_parent == head_parent {
            UnintegratedKind::Amended
        } else {
            // Unrelated detached state — don't touch it.
            return Ok(None);
        }
    };

    // Guard against re-detecting after integration: if the branch tip already
    // descends from HEAD, the upper commits have been moved over.
    let branch_tip = match stack.entries.last() {
        Some(entry) => entry.oid,
        None => return Ok(None),
    };
    if branch_tip == head_oid || repo.graph_descendant_of(branch_tip, head_oid)? {
        return Ok(None);
    }

    let (count, _) = repo.graph_ahead_behind(head_oid, original_oid)?;
    let count = count.max(1);

    let subject = head
        .summary()
        .unwrap_or("(no message)")
        .to_string();
    let short_sha = repo
        .find_object(head_oid, None)?
        .short_id()?
        .as_str()
        .unwrap_or("")
        .to_string();

    Ok(Some(UnintegratedCommit {
        head_oid,
        original_oid,
        branch_name,
        saved_position,
        sits_on_position: saved_position + 1,
        short_sha,
        subject,
        count,
        kind,
    }))
}
```

If `Repository` / `Result` aren't already in scope at that location in `stack.rs`, they are imported at the top of the file (the module already uses `Repository` and the crate `Result`). Reuse the existing imports.

- [ ] **Step 4: Add the JSON field to `output.rs`**

In `crates/gg-core/src/output.rs`, add to `StackJson` (after `entries`):

```rust
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unintegrated_commits: Vec<UnintegratedCommitJson>,
```

And add the new struct near `StackEntryJson`:

```rust
#[derive(Serialize)]
pub struct UnintegratedCommitJson {
    pub sha: String,
    pub subject: String,
    pub sits_on_position: usize,
    pub count: usize,
}
```

- [ ] **Step 5: Wire detection into `ls.rs::show_stack`**

In `crates/gg-core/src/commands/ls.rs`, inside `show_stack` after `let repo = git::open_repo()?;` (~`ls.rs:524`) add:

```rust
    let unintegrated = stack::detect_unintegrated(&repo, stack)?;
```

In the **JSON branch**, populate the field on `StackJson` (add to the struct literal at ~`ls.rs:560`):

```rust
                unintegrated_commits: unintegrated
                    .iter()
                    .map(|u| crate::output::UnintegratedCommitJson {
                        sha: u.short_sha.clone(),
                        subject: u.subject.clone(),
                        sits_on_position: u.sits_on_position,
                        count: u.count,
                    })
                    .collect(),
```

In the **text branch**: when `unintegrated.is_some()`, suppress the misleading `<- HEAD` marker (HEAD is on the orphan, not a listed entry) and print the callout after the entry loop. Replace the two `is_current` computations (`ls.rs:538` and `ls.rs:617`) so they are forced `false` when an orphan exists. The simplest change: introduce `let has_orphan = unintegrated.is_some();` and change each `let is_current = <expr>;` to `let is_current = !has_orphan && (<expr>);` (in the JSON branch use a separate `let has_orphan` based on the array; in the text branch use `unintegrated.is_some()`).

After the entry-printing `for` loop (after `ls.rs:670`'s closing `}`), add:

```rust
    if let Some(u) = &unintegrated {
        println!();
        let more = if u.count > 1 {
            format!(" (+{} more)", u.count - 1)
        } else {
            String::new()
        };
        println!(
            "  {} Un-integrated commit at HEAD (detached):",
            style("⚠").yellow().bold()
        );
        println!(
            "      {} {}{}  — sits on top of [{}]",
            style(&u.short_sha).yellow(),
            u.subject,
            more,
            u.sits_on_position
        );
        println!(
            "    {}",
            style("Run `gg restack` to fold it into the stack.").dim()
        );
    }
```

Ensure `stack` is imported in `ls.rs` (it uses `crate::stack::...` elsewhere; if only `Stack` is imported, use `crate::stack::detect_unintegrated`).

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p gg-cli --test integration_tests test_ls_reports_unintegrated_midstack_commit test_ls_json_includes_unintegrated_commits`
Expected: PASS.

- [ ] **Step 7: Format, clippy, full build of affected crate**

Run: `cargo fmt --all && cargo clippy -p gg-core -p gg-cli --all-targets --all-features -- -D warnings`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/gg-core/src/stack.rs crates/gg-core/src/output.rs crates/gg-core/src/commands/ls.rs crates/gg-cli/tests/integration_tests/ls.rs
git commit -m "feat(core): detect and report un-integrated mid-stack commits in gg ls (#348)"
```

---

## Task 2: `gg restack` integrates the inserted commit and stays on it

**Files:**
- Modify: `crates/gg-core/src/commands/restack.rs` (`run`, add `integrate_unintegrated` helper)
- Test: `crates/gg-cli/tests/integration_tests/restack.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/gg-cli/tests/integration_tests/restack.rs`:

```rust
#[test]
fn test_restack_integrates_inserted_midstack_commit() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "two"]);
    run_gg(&repo_path, &["mv", "1"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "inserted"]);

    let (ok, out, err) = run_gg(&repo_path, &["restack"]);
    assert!(ok, "restack failed: {out}{err}");

    // Branch now contains all three in order: one -> inserted -> two.
    let (_ok, log) = run_git(&repo_path, &["log", "--oneline", "testuser/testing"]);
    assert!(log.contains("one") && log.contains("inserted") && log.contains("two"), "log: {log}");

    // HEAD stays on the inserted commit (detached).
    let (_ok, head_subj) = run_git(&repo_path, &["log", "-1", "--pretty=%s", "HEAD"]);
    assert_eq!(head_subj.trim(), "inserted", "HEAD subject: {head_subj}");

    // ls now shows 3 commits with HEAD on [2] and no orphan warning.
    let (ok, out, err) = run_gg(&repo_path, &["ls"]);
    assert!(ok, "ls failed: {out}{err}");
    assert!(out.contains("3 commits"), "ls: {out}");
    assert!(!out.contains("Un-integrated commit"), "ls should be clean: {out}");
    assert!(out.contains("inserted") && out.contains("<- HEAD"), "ls: {out}");
}

#[test]
fn test_restack_integrates_multiple_inserted_commits() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "two"]);
    run_gg(&repo_path, &["mv", "1"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "ins_a"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "ins_b"]);

    let (ok, out, err) = run_gg(&repo_path, &["restack"]);
    assert!(ok, "restack failed: {out}{err}");

    let (_ok, head_subj) = run_git(&repo_path, &["log", "-1", "--pretty=%s", "HEAD"]);
    assert_eq!(head_subj.trim(), "ins_b", "HEAD subject: {head_subj}");

    let (_ok, log) = run_git(&repo_path, &["log", "--oneline", "testuser/testing"]);
    for m in ["one", "ins_a", "ins_b", "two"] {
        assert!(log.contains(m), "missing {m} in log: {log}");
    }
}

#[test]
fn test_restack_without_orphan_is_unchanged() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "two"]);

    // No detached orphan: restack should report a consistent stack (no error).
    let (ok, out, err) = run_gg(&repo_path, &["restack"]);
    assert!(ok, "restack failed: {out}{err}");
    assert!(!out.contains("Integrated"), "should not integrate: {out}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p gg-cli --test integration_tests test_restack_integrates_inserted_midstack_commit test_restack_integrates_multiple_inserted_commits test_restack_without_orphan_is_unchanged`
Expected: FAIL — the inserted commit is not integrated / HEAD not preserved.

- [ ] **Step 3: Add the integration helper to `restack.rs`**

Add this function above `pub fn run` (~`restack.rs:140`):

```rust
/// Integrate a commit made at a detached mid-stack HEAD into the stack branch,
/// then leave HEAD detached on that commit. Returns early from `run`.
fn integrate_unintegrated(
    repo: &git2::Repository,
    config: &Config,
    unintegrated: stack::UnintegratedCommit,
    json: bool,
) -> Result<()> {
    let guard = git::begin_recorded_op(
        repo,
        config,
        OperationKind::Restack,
        std::env::args().skip(1).collect(),
        None,
        SnapshotScope::AllUserBranches,
    )?;

    // Move the commits above the navigated position onto the detached HEAD work.
    // git rebase --onto <new_base> <old_base> <branch>
    let output = std::process::Command::new("git")
        .args([
            "rebase",
            "--onto",
            &unintegrated.head_oid.to_string(),
            &unintegrated.original_oid.to_string(),
            &unintegrated.branch_name,
        ])
        .current_dir(repo.workdir().unwrap_or_else(|| repo.path()))
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stderr.contains("CONFLICT")
            || stderr.contains("conflict")
            || stdout.contains("CONFLICT")
            || stdout.contains("conflict")
        {
            return Err(GgError::RebaseConflict);
        }
        return Err(GgError::Other(format!(
            "Failed to integrate commit: {}",
            if stderr.is_empty() { stdout } else { stderr }
        )));
    }

    // Stay on the integrated commit (detached HEAD).
    let head_commit = repo.find_commit(unintegrated.head_oid)?;
    git::checkout_commit(repo, &head_commit)?;

    // Rewrite the nav context to the new position of the integrated commit.
    let reloaded = Stack::load(repo, config)?;
    let new_position = reloaded
        .current_position
        .unwrap_or(unintegrated.saved_position);
    stack::save_nav_context(
        repo.path(),
        &unintegrated.branch_name,
        new_position,
        unintegrated.head_oid,
    )?;

    guard.finalize_with_scope(repo, config, SnapshotScope::AllUserBranches, vec![], false)?;

    if json {
        print_json(&RestackResponse {
            version: OUTPUT_VERSION,
            restack: RestackResultJson {
                stack_name: reloaded.name.clone(),
                total_entries: reloaded.entries.len(),
                entries_restacked: unintegrated.count,
                entries_ok: reloaded.entries.len().saturating_sub(unintegrated.count),
                dry_run: false,
                steps: vec![],
            },
        });
    } else {
        println!(
            "{} Integrated {} commit(s) into stack {:?}; HEAD stays on {} {}",
            style("✓").green().bold(),
            unintegrated.count,
            reloaded.name,
            style(&unintegrated.short_sha).yellow(),
            unintegrated.subject,
        );
        println!(
            "  {}",
            style("Hint: Run `gg sync` to push the updated stack.").dim()
        );
    }

    Ok(())
}
```

- [ ] **Step 4: Call the helper from `run` before plan building**

In `restack.rs::run`, after `let stack = Stack::load(&repo, &config)?;` and the `is_empty` check (after ~`restack.rs:164`), add:

```rust
    // If there is a commit made at a detached mid-stack HEAD, fold it in and
    // stay on it (early return). This is the only restack path that mutates
    // while keeping HEAD detached.
    if let Some(unintegrated) = stack::detect_unintegrated(&repo, &stack)? {
        return integrate_unintegrated(&repo, &config, unintegrated, options.json);
    }
```

Note ordering: this is placed after `git::require_clean_working_directory(&repo)?` (`restack.rs:157`) and after `Stack::load`, so the working tree is already verified clean. `detect_unintegrated` is read-only.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p gg-cli --test integration_tests test_restack_integrates_inserted_midstack_commit test_restack_integrates_multiple_inserted_commits test_restack_without_orphan_is_unchanged`
Expected: PASS.

- [ ] **Step 6: Format + clippy**

Run: `cargo fmt --all && cargo clippy -p gg-core -p gg-cli --all-targets --all-features -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/gg-core/src/commands/restack.rs crates/gg-cli/tests/integration_tests/restack.rs
git commit -m "feat(core): integrate detached mid-stack commits via gg restack (#348)"
```

---

## Task 3: `gg restack` also integrates an amended detached commit

**Files:**
- Test: `crates/gg-cli/tests/integration_tests/restack.rs`
- (No new production code expected — `detect_unintegrated` already classifies `Amended` and `integrate_unintegrated` uses the same `rebase --onto`. This task verifies and, only if a test fails, fixes.)

- [ ] **Step 1: Write the failing test**

Add to `crates/gg-cli/tests/integration_tests/restack.rs`:

```rust
#[test]
fn test_restack_integrates_amended_midstack_commit() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "two"]);
    run_gg(&repo_path, &["mv", "1"]);
    // Amend the navigated commit in place (rewrites "one").
    run_git(&repo_path, &["commit", "--amend", "-m", "one_amended"]);

    let (ok, out, err) = run_gg(&repo_path, &["restack"]);
    assert!(ok, "restack failed: {out}{err}");

    // HEAD stays on the amended commit.
    let (_ok, head_subj) = run_git(&repo_path, &["log", "-1", "--pretty=%s", "HEAD"]);
    assert_eq!(head_subj.trim(), "one_amended", "HEAD subject: {head_subj}");

    // Branch is one_amended -> two (2 commits, "one" gone).
    let (_ok, log) = run_git(&repo_path, &["log", "--oneline", "testuser/testing"]);
    assert!(log.contains("one_amended") && log.contains("two"), "log: {log}");
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p gg-cli --test integration_tests test_restack_integrates_amended_midstack_commit`
Expected: PASS if the `Amended` classification works end-to-end. If it FAILS, debug `detect_unintegrated` (the `original_parent == head_parent` branch) using `superpowers:systematic-debugging`, fix in `crates/gg-core/src/stack.rs`, and re-run.

- [ ] **Step 3: Format + clippy (only if code changed)**

Run: `cargo fmt --all && cargo clippy -p gg-core -p gg-cli --all-targets --all-features -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/gg-cli/tests/integration_tests/restack.rs crates/gg-core/src/stack.rs
git commit -m "test(core): cover amended detached mid-stack commit integration (#348)"
```

---

## Task 4: Conflict path test

**Files:**
- Test: `crates/gg-cli/tests/integration_tests/restack.rs`

Verifies that when folding the inserted commit conflicts with an upper commit, restack surfaces the existing conflict guidance and exits non-zero.

- [ ] **Step 1: Write the test**

Add to `crates/gg-cli/tests/integration_tests/restack.rs`:

```rust
#[test]
fn test_restack_integration_conflict_reports_guidance() {
    use crate::helpers::{create_test_repo, run_gg, run_git};
    use std::fs;

    let (_temp_dir, repo_path) = create_test_repo();
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).unwrap();
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .unwrap();

    run_gg(&repo_path, &["co", "testing"]);
    // commit "one" leaves conflict.txt absent; "two" adds conflict.txt = "two".
    run_git(&repo_path, &["commit", "--allow-empty", "-m", "one"]);
    fs::write(repo_path.join("conflict.txt"), "two\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "two"]);

    run_gg(&repo_path, &["mv", "1"]);
    // Inserted commit writes the same file with different content -> conflict
    // when "two" is replayed on top.
    fs::write(repo_path.join("conflict.txt"), "inserted\n").unwrap();
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "inserted"]);

    let (ok, out, err) = run_gg(&repo_path, &["restack"]);
    assert!(!ok, "expected conflict failure: {out}{err}");
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("gg continue") || combined.contains("gg abort") || combined.contains("conflict"),
        "expected conflict guidance: {combined}"
    );
}
```

Note: `GgError::RebaseConflict`'s user-facing message is produced by the CLI's error rendering — assert on whichever of `gg continue` / `gg abort` / `conflict` the existing `RebaseConflict` rendering emits (check `crates/gg-core/src/error.rs` for the exact text and tighten the assertion to match).

- [ ] **Step 2: Run the test**

Run: `cargo test -p gg-cli --test integration_tests test_restack_integration_conflict_reports_guidance`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/gg-cli/tests/integration_tests/restack.rs
git commit -m "test(core): cover conflict path when integrating mid-stack commit (#348)"
```

---

## Task 5: Documentation and agent skill updates

**Files:**
- Modify: relevant page(s) under `docs/src/` (the stack-navigation / restack guide and command reference)
- Modify: `skills/gg/SKILL.md`
- Modify: `skills/gg/reference.md`

- [ ] **Step 1: Find the right docs pages**

Run: `rg -l "restack|gg mv|navigation" docs/src`
Identify the navigation guide and the `restack` reference entry.

- [ ] **Step 2: Document the insert-in-the-middle workflow**

In the navigation/restack guide, add a short section: inserting a commit in the middle of a stack is done by `gg mv <n>`, making one or more `git commit`s, then `gg restack` — which folds them in and leaves you on the new commit. Note that `gg ls` will, in the meantime, show the commit as "un-integrated" rather than losing it. Include a concrete example mirroring issue #348:

```
gg co testing
git commit -m one
git commit -m two
gg mv 1
git commit -m inserted   # HEAD now detached on top of [1]
gg ls                    # shows "Un-integrated commit at HEAD"
gg restack               # folds it in -> one, inserted, two; HEAD stays on inserted
```

- [ ] **Step 3: Update the restack command reference**

In the `restack` reference page, note that `restack` also integrates a commit made at a detached mid-stack HEAD (inserted or amended), keeping HEAD on that commit.

- [ ] **Step 4: Update the skill**

In `skills/gg/SKILL.md`, add the insert workflow to the relevant operating rules (how to insert a commit mid-stack, and that `gg ls` flags un-integrated commits). In `skills/gg/reference.md`, document the new `unintegrated_commits` array on the `gg ls --json` single-stack output (fields: `sha`, `subject`, `sits_on_position`, `count`) and the restack integration behavior.

- [ ] **Step 5: Build the docs to verify**

Run: `mdbook build docs`
Expected: builds without error. (If `mdbook` is not installed, skip and note it.)

- [ ] **Step 6: Commit**

```bash
git add docs skills
git commit -m "docs: document mid-stack commit insertion workflow (#348)"
```

---

## Final verification

- [ ] **Run the full suite**

Run: `cargo fmt --all && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features`
Expected: format clean, no clippy warnings, all tests pass.

---

## Self-review notes (author)

- **Spec coverage:** Detection (Task 1 §3) ✓; ls read-only callout + JSON (Task 1) ✓; restack integration + stay-in-place + nav-context rewrite + GG-ID left blank (Task 2) ✓; amend-in-place (Task 3) ✓; conflict path reuse (Task 4) ✓; docs/skill (Task 5) ✓.
- **GG-IDs left blank:** the integration path deliberately does *not* call `normalize_stack_metadata`, matching the issue's `(id: -)` expected output; IDs are assigned on `gg sync` as for any new commit.
- **Type consistency:** `detect_unintegrated`, `UnintegratedCommit`, `UnintegratedKind`, `UnintegratedCommitJson`, and field names (`head_oid`, `original_oid`, `branch_name`, `saved_position`, `sits_on_position`, `short_sha`, `subject`, `count`) are used identically across Tasks 1–4.
- **Non-goal honored:** manual `git checkout <sha>` (no nav context) → `read_nav_context` returns `None` → detector returns `None`, so it's untouched, as specified.
