# Native GitHub Stacks Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make full `gg sync` runs create or append GitHub native Stacked PR associations through the official `github/gh-stack` extension while preserving git-gud ownership and non-fatal sync behavior.

**Architecture:** Add a GitHub-specific configuration enum, then build a focused `github_stacks` module with a pure create/append reconciliation planner and an injectable `gh` command backend. `sync` supplies its ordered active PR numbers, records mutation attempts as remote touches, and maps the normalized result into human, JSON, and JSONL output without exposing extension subprocess output.

**Tech Stack:** Rust 2021, serde/serde_json, semver, std::process::Command, clap/dialoguer configuration UI, Cargo unit and CLI integration tests, mdBook.

## Global Constraints

- The only mutation backend is the official `github/gh-stack` extension.
- The minimum supported extension version is exactly `v0.1.0`.
- `defaults.github.stacks_integration` accepts exactly `off`, `auto`, and `force`; the default is `auto`.
- Version one may create a native stack or append new top PRs; it must never remove, reorder, unstack, or recreate a diverged native stack.
- Every native-stack failure is non-fatal to an otherwise successful `gg sync`.
- `auto` silently skips missing/outdated extension and unsupported repositories; `force` reports those capability states as warnings.
- `force` never bypasses version, repository, or create/append-only safety checks.
- Native integration runs only for GitHub and never mutates git-gud local stack tracking.
- Partial `gg sync --until` runs and GitLab runs must not call the backend.
- Only numeric PR arguments may be passed to `gh stack link`.
- `--json` remains atomic; `--jsonl` emits an immediate `github_stack` event and a deterministic final summary.
- Existing GitLab behavior, `GG-ID`/`GG-Parent` behavior, and current PR base chaining remain unchanged.
- Do not update `skills/gg`; the CLI owns this decision and no agent authority boundary changes.
- Every production change must have tests, `cargo fmt --all` must pass, Clippy must pass with `-D warnings`, and the full all-features test suite must pass.

---

## File Map

- Modify `crates/gg-core/Cargo.toml`: add the direct `semver` dependency used for the extension version floor.
- Modify `Cargo.lock`: record the direct workspace dependency relationship after Cargo updates the lockfile.
- Modify `crates/gg-core/src/config.rs`: define `GithubDefaults` and `GithubStacksIntegration`, default/serde behavior, getter, and unit tests.
- Modify `crates/gg-core/src/commands/setup.rs`: add the GitHub-only full-setup selector and pure selection helpers/tests.
- Create `crates/gg-core/src/github_stacks.rs`: own remote models, normalized results, pure reconciliation planning, extension/API execution, and unit tests.
- Modify `crates/gg-core/src/lib.rs`: export the focused `github_stacks` module.
- Modify `crates/gg-core/src/output.rs`: add `github_stack` to atomic/summary output and the progressive event/status mapping.
- Modify `crates/gg-core/src/commands/sync.rs`: collect active PRs, run or skip reconciliation, mark remote attempts, render results, and populate output.
- Create `crates/gg-cli/tests/integration_tests/github_stacks.rs`: focused fake-`gh` command-contract and structured-output tests.
- Modify `crates/gg-cli/tests/integration_tests/main.rs`: register the new integration-test module.
- Modify `crates/gg-cli/tests/integration_tests/sync.rs`: opt unrelated multi-entry fake-`gh` fixtures out where their scripts intentionally do not model `gh-stack`.
- Modify `README.md`: feature summary, prerequisite, sync behavior, and configuration table.
- Modify `docs/src/configuration.md`: document the nested GitHub mode and default.
- Modify `docs/src/commands/setup.md`: document the GitHub selector.
- Modify `docs/src/commands/sync.md`: document eligibility, outcomes, recovery, JSON, and JSONL.
- Modify `docs/src/getting-started.md`: document optional extension installation for the native UI.

---

### Task 1: Add GitHub Stacks Configuration and Setup Selection

**Files:**
- Modify: `crates/gg-core/src/config.rs`
- Modify: `crates/gg-core/src/commands/setup.rs`

**Interfaces:**
- Produces: `GithubStacksIntegration::{Off, Auto, Force}` with lowercase serde names and `Default::default() == Auto`.
- Produces: `GithubDefaults { pub stacks_integration: GithubStacksIntegration }`.
- Produces: `Config::get_github_stacks_integration(&self) -> GithubStacksIntegration`.
- Consumes later: Tasks 2-4 use the enum and getter; no task may parse mode strings independently.

- [ ] **Step 1: Add failing configuration tests**

Add tests beside the existing GitLab/default tests in `config.rs`:

```rust
#[test]
fn test_github_stacks_integration_defaults_to_auto() {
    let config: Config = serde_json::from_str(r#"{"defaults":{}}"#).unwrap();
    assert_eq!(
        config.get_github_stacks_integration(),
        GithubStacksIntegration::Auto
    );
}

#[test]
fn test_github_stacks_integration_round_trips_all_modes() {
    for mode in [
        GithubStacksIntegration::Off,
        GithubStacksIntegration::Auto,
        GithubStacksIntegration::Force,
    ] {
        let mut config = Config::default();
        config.defaults.github.stacks_integration = mode;
        let json = serde_json::to_string(&config).unwrap();
        let parsed: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.get_github_stacks_integration(), mode);
    }
}

#[test]
fn test_github_stacks_integration_uses_lowercase_json() {
    let config: Config = serde_json::from_str(
        r#"{"defaults":{"github":{"stacks_integration":"force"}}}"#,
    )
    .unwrap();
    assert_eq!(
        config.get_github_stacks_integration(),
        GithubStacksIntegration::Force
    );
}

#[test]
fn test_local_config_overrides_global_github_stacks_mode() {
    let mut effective = Config::default();
    effective.defaults.github.stacks_integration = GithubStacksIntegration::Force;
    let mut local = Config::default();
    local.defaults.github.stacks_integration = GithubStacksIntegration::Off;
    effective.merge_local(local);
    assert_eq!(
        effective.get_github_stacks_integration(),
        GithubStacksIntegration::Off
    );
}
```

- [ ] **Step 2: Run the config tests and verify the missing types fail compilation**

Run:

```sh
cargo test -p gg-core config::tests::test_github_stacks_integration -- --nocapture
```

Expected: FAIL with unresolved `GithubStacksIntegration` and missing `Defaults::github`/getter errors.

- [ ] **Step 3: Implement the configuration model and getter**

Add the GitHub defaults next to `GitLabDefaults` and place `github` next to `gitlab` in `Defaults`:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GithubStacksIntegration {
    Off,
    #[default]
    Auto,
    Force,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GithubDefaults {
    #[serde(default)]
    pub stacks_integration: GithubStacksIntegration,
}
```

Add `#[serde(default)] pub github: GithubDefaults` to `Defaults`, initialize it in `Defaults::default`, and add:

```rust
pub fn get_github_stacks_integration(&self) -> GithubStacksIntegration {
    self.defaults.github.stacks_integration
}
```

- [ ] **Step 4: Run focused configuration tests**

Run:

```sh
cargo test -p gg-core config::tests::test_github_stacks_integration -- --nocapture
```

Expected: PASS for default, round-trip, and lowercase tests.

- [ ] **Step 5: Add failing pure setup-selection tests**

Extract selection helpers in `commands/setup.rs` and test them without opening an interactive terminal:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_stacks_prompt_is_github_only() {
        assert!(should_prompt_github_stacks(Some("github")));
        assert!(!should_prompt_github_stacks(Some("gitlab")));
        assert!(!should_prompt_github_stacks(None));
    }

    #[test]
    fn github_stacks_mode_indices_round_trip() {
        for mode in [
            GithubStacksIntegration::Auto,
            GithubStacksIntegration::Off,
            GithubStacksIntegration::Force,
        ] {
            assert_eq!(github_stacks_mode_from_index(github_stacks_mode_index(mode)), mode);
        }
    }

    #[test]
    fn gitlab_setup_preserves_existing_github_stacks_mode() {
        let mut defaults = Defaults::default();
        defaults.github.stacks_integration = GithubStacksIntegration::Force;
        if should_prompt_github_stacks(Some("gitlab")) {
            defaults.github.stacks_integration = GithubStacksIntegration::Off;
        }
        assert_eq!(
            defaults.github.stacks_integration,
            GithubStacksIntegration::Force
        );
    }
}
```

- [ ] **Step 6: Run setup tests and verify helper names fail**

Run:

```sh
cargo test -p gg-core commands::setup::tests::github_stacks -- --nocapture
```

Expected: FAIL with unresolved helper/import errors.

- [ ] **Step 7: Implement the GitHub-only full-setup selector**

Import `GithubStacksIntegration`, then add these helpers:

```rust
fn should_prompt_github_stacks(provider: Option<&str>) -> bool {
    provider == Some("github")
}

fn github_stacks_mode_index(mode: GithubStacksIntegration) -> usize {
    match mode {
        GithubStacksIntegration::Auto => 0,
        GithubStacksIntegration::Off => 1,
        GithubStacksIntegration::Force => 2,
    }
}

fn github_stacks_mode_from_index(index: usize) -> GithubStacksIntegration {
    match index {
        1 => GithubStacksIntegration::Off,
        2 => GithubStacksIntegration::Force,
        _ => GithubStacksIntegration::Auto,
    }
}
```

In `prompt_defaults_full`, after the Sync group and before Land, show a `GitHub` group only when the effective provider is GitHub:

```rust
if should_prompt_github_stacks(defaults.provider.as_deref()) {
    print_group_header("GitHub");
    let choices = [
        "Auto (recommended)",
        "Off",
        "Force (warn when unavailable)",
    ];
    let selected = Select::with_theme(theme)
        .with_prompt("Link synced PRs into GitHub's native Stacked PRs UI?")
        .items(&choices)
        .default(github_stacks_mode_index(existing.github.stacks_integration))
        .interact()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;
    defaults.github.stacks_integration = github_stacks_mode_from_index(selected);
}
```

Because `defaults` starts as `existing.clone()`, GitLab full setup preserves the existing GitHub value.

- [ ] **Step 8: Run focused and crate tests**

Run:

```sh
cargo test -p gg-core config::tests::test_github_stacks_integration -- --nocapture
cargo test -p gg-core commands::setup::tests::github_stacks -- --nocapture
cargo test -p gg-core
```

Expected: all PASS.

- [ ] **Step 9: Commit the configuration deliverable**

```sh
git add crates/gg-core/src/config.rs crates/gg-core/src/commands/setup.rs
git commit -m "feat(config): add GitHub Stacks integration mode"
```

---

### Task 2: Build the Pure Native-Stack Reconciliation Planner

**Files:**
- Create: `crates/gg-core/src/github_stacks.rs`
- Modify: `crates/gg-core/src/lib.rs`

**Interfaces:**
- Consumes: `crate::config::GithubStacksIntegration` from Task 1.
- Produces: `RemotePullRequestState`, `RemoteStackEntry`, and `RemoteStackSnapshot` as the backend-neutral remote model.
- Produces: `ReconcilePlan::{Create, Append, Unchanged, Diverged}`.
- Produces: `plan_reconciliation(local_pr_numbers: &[u64], remote_stacks: &[RemoteStackSnapshot]) -> ReconcilePlan`.
- Produces: serialized `GithubStackAction`, `GithubStackReason`, and `GithubStackSyncResult` used by Tasks 3-4.

- [ ] **Step 1: Register the empty focused module**

Add `pub mod github_stacks;` to `lib.rs` and create `github_stacks.rs` with the imports and type declarations needed by the tests in the next step. Do not add subprocess execution in this task.

- [ ] **Step 2: Add failing planner tests for create, unchanged, and append**

Define a small test helper and concrete expectations:

```rust
fn entry(number: u64, state: RemotePullRequestState) -> RemoteStackEntry {
    RemoteStackEntry { number, state }
}

#[test]
fn plans_create_when_no_local_pr_is_stacked() {
    assert_eq!(plan_reconciliation(&[41, 42], &[]), ReconcilePlan::Create);
}

#[test]
fn plans_unchanged_for_exact_active_sequence_after_merged_prefix() {
    let remote = RemoteStackSnapshot {
        number: 7,
        entries: vec![
            entry(40, RemotePullRequestState::Merged),
            entry(41, RemotePullRequestState::Open),
            entry(42, RemotePullRequestState::Draft),
        ],
    };
    assert_eq!(
        plan_reconciliation(&[41, 42], &[remote]),
        ReconcilePlan::Unchanged { stack_number: 7 }
    );
}

#[test]
fn plans_append_with_only_the_new_top_delta() {
    let remote = RemoteStackSnapshot {
        number: 7,
        entries: vec![entry(41, RemotePullRequestState::Open)],
    };
    assert_eq!(
        plan_reconciliation(&[41, 42, 43], &[remote]),
        ReconcilePlan::Append {
            stack_number: 7,
            delta: vec![42, 43],
        }
    );
}
```

- [ ] **Step 3: Add failing divergence tests**

Cover each safety boundary with named tests:

```rust
#[test]
fn rejects_reordered_active_prs() {
    let remote = RemoteStackSnapshot {
        number: 7,
        entries: vec![
            entry(41, RemotePullRequestState::Open),
            entry(42, RemotePullRequestState::Open),
        ],
    };
    assert!(matches!(
        plan_reconciliation(&[42, 41], &[remote]),
        ReconcilePlan::Diverged { .. }
    ));
}

#[test]
fn rejects_closed_unmerged_remote_entry() {
    let remote = RemoteStackSnapshot {
        number: 7,
        entries: vec![
            entry(41, RemotePullRequestState::Open),
            entry(42, RemotePullRequestState::Closed),
        ],
    };
    assert!(matches!(
        plan_reconciliation(&[41, 43], &[remote]),
        ReconcilePlan::Diverged { .. }
    ));
}

#[test]
fn rejects_membership_in_multiple_native_stacks() {
    let first = RemoteStackSnapshot {
        number: 7,
        entries: vec![entry(41, RemotePullRequestState::Open)],
    };
    let second = RemoteStackSnapshot {
        number: 8,
        entries: vec![entry(42, RemotePullRequestState::Open)],
    };
    assert!(matches!(
        plan_reconciliation(&[41, 42], &[first, second]),
        ReconcilePlan::Diverged { .. }
    ));
}
```

Also add named tests for local removal, a merged entry after an active entry, and a fully merged unrelated prior stack yielding `Create` because no local active PR belongs to it.

- [ ] **Step 4: Run planner tests and verify they fail**

Run:

```sh
cargo test -p gg-core github_stacks::tests::plans -- --nocapture
cargo test -p gg-core github_stacks::tests::rejects -- --nocapture
```

Expected: FAIL until `plan_reconciliation` implements the prefix rules.

- [ ] **Step 5: Implement the remote model and pure planner**

Use these exact public shapes:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemotePullRequestState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteStackEntry {
    pub number: u64,
    pub state: RemotePullRequestState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteStackSnapshot {
    pub number: u64,
    pub entries: Vec<RemoteStackEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcilePlan {
    Create,
    Append { stack_number: u64, delta: Vec<u64> },
    Unchanged { stack_number: u64 },
    Diverged { message: String },
}
```

`plan_reconciliation` must:

1. Keep only remote stacks containing at least one local PR.
2. Return `Create` when that set is empty.
3. Return `Diverged` when more than one stack remains.
4. Accept only a contiguous merged prefix.
5. Reject `Closed` or later `Merged` entries in the retained active suffix.
6. Compare the remote open/draft suffix with the local list.
7. Return `Unchanged` for equality, `Append` when remote is a prefix, and `Diverged` otherwise.

- [ ] **Step 6: Add the normalized serializable result model**

Add:

```rust
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubStackAction {
    Created,
    Appended,
    Unchanged,
    Skipped,
    Warning,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GithubStackReason {
    Disabled,
    PartialSync,
    InsufficientPrs,
    UnresolvedPrs,
    MissingExtension,
    OutdatedExtension,
    UnsupportedRepository,
    Diverged,
    BackendFailed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GithubStackSyncResult {
    pub mode: GithubStacksIntegration,
    pub action: GithubStackAction,
    pub reason: Option<GithubStackReason>,
    pub stack_number: Option<u64>,
    pub pr_numbers: Vec<u64>,
    pub message: Option<String>,
}
```

Add constructors `skipped(mode, reason, pr_numbers)` and `warning(mode, reason, pr_numbers, message)`, plus `is_warning(&self) -> bool`.

- [ ] **Step 7: Run focused and crate tests**

Run:

```sh
cargo test -p gg-core github_stacks::tests -- --nocapture
cargo test -p gg-core
```

Expected: all PASS without subprocess or network access.

- [ ] **Step 8: Commit the planner deliverable**

```sh
git add crates/gg-core/src/lib.rs crates/gg-core/src/github_stacks.rs
git commit -m "feat(github): plan native stack reconciliation"
```

---

### Task 3: Implement the Official `gh-stack` Backend

**Files:**
- Modify: `crates/gg-core/Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/gg-core/src/github_stacks.rs`

**Interfaces:**
- Consumes: Task 2's `plan_reconciliation` and result types.
- Produces: `GithubStackReconcileOutcome { pub result: GithubStackSyncResult, pub mutation_attempted: bool }`.
- Produces: `reconcile_with_gh(mode: GithubStacksIntegration, base: &str, pr_numbers: &[u64]) -> GithubStackReconcileOutcome` for Task 4.
- Keeps injectable internals: `GhCommandRunner::run(&self, args: &[String]) -> std::io::Result<GhCommandOutput>` and generic `reconcile_with_runner` for unit tests.

- [ ] **Step 1: Add the direct semver dependency**

Add `semver = "1"` under gg-core Utilities and run:

```sh
cargo check -p gg-core
```

Expected: PASS and `Cargo.lock` updated without a new semver version family.

- [ ] **Step 2: Add failing version and capability tests**

Create a scripted fake runner whose queue asserts exact argument vectors and returns `GhCommandOutput { success, stdout, stderr }`. Add tests:

```rust
#[test]
fn accepts_minimum_gh_stack_version() {
    assert_eq!(
        parse_gh_stack_version("gh stack version 0.1.0\n").unwrap(),
        semver::Version::new(0, 1, 0)
    );
}

#[test]
fn rejects_outdated_gh_stack_version() {
    let runner = FakeRunner::new([response(
        &["stack", "--version"],
        true,
        "gh stack version 0.0.8\n",
        "",
    )]);
    let outcome = reconcile_with_runner(
        &runner,
        GithubStacksIntegration::Auto,
        "main",
        &[41, 42],
    );
    assert_eq!(outcome.result.action, GithubStackAction::Skipped);
    assert_eq!(outcome.result.reason, Some(GithubStackReason::OutdatedExtension));
    assert!(!outcome.mutation_attempted);
}
```

Add a `Force` version of the missing/outdated test and assert `Warning` rather than `Skipped`.

- [ ] **Step 3: Add failing API parsing and planner-input tests**

Deserialize the documented stack resource shape. Use fixture JSON containing `number`, `state`, `draft`, and `merged_at`:

```rust
const STACK_7: &str = r#"[{
  "number": 7,
  "pull_requests": [
    {"number": 40, "state": "closed", "draft": false, "merged_at": "2026-07-01T00:00:00Z"},
    {"number": 41, "state": "open", "draft": false, "merged_at": null}
  ]
}]"#;

#[test]
fn parses_merged_prefix_and_open_entry() {
    let stacks = parse_stack_list(STACK_7).unwrap();
    assert_eq!(stacks[0].entries[0].state, RemotePullRequestState::Merged);
    assert_eq!(stacks[0].entries[1].state, RemotePullRequestState::Open);
}
```

Add tests mapping open drafts to `Draft`, closed/unmerged to `Closed`, malformed JSON to `BackendFailed`, and an API stderr containing `HTTP 404` to repository unsupported.

- [ ] **Step 4: Run backend tests and verify missing implementation failures**

Run:

```sh
cargo test -p gg-core github_stacks::tests::accepts_minimum -- --nocapture
cargo test -p gg-core github_stacks::tests::rejects_outdated -- --nocapture
cargo test -p gg-core github_stacks::tests::parses_ -- --nocapture
```

Expected: FAIL for missing parser, runner, and backend functions.

- [ ] **Step 5: Implement command execution and capability checks**

Add:

```rust
const MIN_GH_STACK_VERSION: &str = "0.1.0";

pub struct GithubStackReconcileOutcome {
    pub result: GithubStackSyncResult,
    pub mutation_attempted: bool,
}

struct GhCommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

trait GhCommandRunner {
    fn run(&self, args: &[String]) -> std::io::Result<GhCommandOutput>;
}
```

`SystemGhRunner` executes `Command::new("gh").args(args).output()` and captures both streams. `parse_gh_stack_version` finds the first whitespace token accepted by `semver::Version::parse` after trimming an optional leading `v`.

Before API reads, run exactly `gh stack --version`. A non-zero command is missing extension. A parse failure is `BackendFailed`; a parsed version below `0.1.0` is outdated.

- [ ] **Step 6: Implement read-only native stack discovery**

For every local PR number, run:

```text
gh api repos/{owner}/{repo}/stacks?pull_request=<number>
```

Pass the endpoint as one argument so `gh` substitutes `{owner}` and `{repo}` from the current repository. Parse each response as `Vec<GhStackJson>`, convert it to `RemoteStackSnapshot`, and deduplicate by stack number before calling `plan_reconciliation`.

An `HTTP 404` from any membership query maps to `UnsupportedRepository`. Any
other non-zero response or parse error maps to `BackendFailed`.

- [ ] **Step 7: Add failing exact create/append command tests**

Create test scripts with complete expected calls:

```rust
#[test]
fn create_uses_base_and_numeric_prs_only() {
    let runner = FakeRunner::new([
        response(&["stack", "--version"], true, "gh stack version 0.1.0\n", ""),
        response(&["api", "repos/{owner}/{repo}/stacks?pull_request=41"], true, "[]", ""),
        response(&["api", "repos/{owner}/{repo}/stacks?pull_request=42"], true, "[]", ""),
        response(&["stack", "link", "--base", "main", "41", "42"], true, "Created stack\n", ""),
        response(&["api", "repos/{owner}/{repo}/stacks?pull_request=41"], true, STACK_7_CREATED, ""),
    ]);
    let outcome = reconcile_with_runner(&runner, GithubStacksIntegration::Auto, "main", &[41, 42]);
    assert_eq!(outcome.result.action, GithubStackAction::Created);
    assert_eq!(outcome.result.stack_number, Some(7));
    assert!(outcome.mutation_attempted);
    runner.assert_exhausted();
}
```

Add an append test whose membership response contains merged PR 40 and active PR 41, then expect exactly:

```text
gh stack link 7 42 43
```

Add an unchanged test and assert no `stack link` call. Add a link-failure test and assert `Warning(BackendFailed)` with `mutation_attempted == true`.

Add a create-confirmation failure test: the `stack link` call succeeds, the
follow-up membership read fails, and the result remains `Created` with
`stack_number == None`, `mutation_attempted == true`, and a diagnostic
`message`.

- [ ] **Step 8: Implement create, append, unchanged, and divergence execution**

Map planner decisions as follows:

- `Create`: set `mutation_attempted = true`, run `gh stack link --base <base> <all PRs>`, then reread the first PR to confirm the stack number.
- `Append`: set `mutation_attempted = true`, run `gh stack link <stack-number> <delta>`.
- `Unchanged`: return the known number without invoking link.
- `Diverged`: return `Warning(Diverged)` without invoking link.

Build each argument as its own `String`; never invoke a shell. Convert all non-zero link exits into a concise message using captured stderr first, stdout second, and the exit context last.

- [ ] **Step 9: Run focused backend and crate tests**

Run:

```sh
cargo test -p gg-core github_stacks::tests -- --nocapture
cargo test -p gg-core
cargo clippy -p gg-core --all-targets --all-features -- -D warnings
```

Expected: all PASS with no Clippy warnings.

- [ ] **Step 10: Commit the backend deliverable**

```sh
git add crates/gg-core/Cargo.toml Cargo.lock crates/gg-core/src/github_stacks.rs
git commit -m "feat(github): link PRs with gh-stack"
```

---

### Task 4: Integrate Reconciliation into Sync and Structured Output

**Files:**
- Modify: `crates/gg-core/src/output.rs`
- Modify: `crates/gg-core/src/commands/sync.rs`
- Create: `crates/gg-cli/tests/integration_tests/github_stacks.rs`
- Modify: `crates/gg-cli/tests/integration_tests/main.rs`
- Modify: `crates/gg-cli/tests/integration_tests/sync.rs`

**Interfaces:**
- Consumes: `Config::get_github_stacks_integration` and `github_stacks::reconcile_with_gh`.
- Consumes: `GithubStackReconcileOutcome::mutation_attempted` to protect operation history.
- Produces: `SyncResultJson::github_stack: Option<GithubStackSyncResult>`.
- Produces: `SyncStreamingEvent::GithubStack` and `Summary::github_stack`.
- Preserves: output version `1`; the new fields/events are additive.

- [ ] **Step 1: Add failing serialization tests**

In `output.rs`, construct a result and require both atomic and progressive shapes:

```rust
fn created_github_stack_result() -> GithubStackSyncResult {
    GithubStackSyncResult {
        mode: GithubStacksIntegration::Auto,
        action: GithubStackAction::Created,
        reason: None,
        stack_number: Some(7),
        pr_numbers: vec![41, 42],
        message: None,
    }
}

#[test]
fn sync_streaming_github_stack_event_is_flat_and_ok() {
    let response = SyncStreamingResponse {
        version: OUTPUT_VERSION,
        command: "sync".to_string(),
        event: SyncStreamingEvent::GithubStack {
            result: created_github_stack_result(),
        },
    };
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["event"], "github_stack");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["action"], "created");
    assert_eq!(value["stack_number"], 7);
}

#[test]
fn sync_streaming_github_stack_warning_has_warning_status() {
    let mut result = created_github_stack_result();
    result.action = GithubStackAction::Warning;
    result.reason = Some(GithubStackReason::Diverged);
    let response = SyncStreamingResponse {
        version: OUTPUT_VERSION,
        command: "sync".to_string(),
        event: SyncStreamingEvent::GithubStack { result },
    };
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["status"], "warning");
}
```

Update the existing atomic response and summary tests to assert a `github_stack` field.

- [ ] **Step 2: Run output tests and verify missing fields/variant fail**

Run:

```sh
cargo test -p gg-core output::tests::sync_streaming_github_stack -- --nocapture
cargo test -p gg-core commands::sync::tests::test_sync_json_response_structure -- --nocapture
```

Expected: FAIL for missing `GithubStack` event and `SyncResultJson::github_stack`.

- [ ] **Step 3: Implement the additive output schema**

Import `GithubStackAction` and `GithubStackSyncResult` into `output.rs`. Add:

```rust
GithubStack {
    #[serde(flatten)]
    result: GithubStackSyncResult,
},
```

to `SyncStreamingEvent`. Add `github_stack: Option<GithubStackSyncResult>` to both `SyncResultJson` and the `Summary` variant.

Update streaming `status` selection so only `GithubStackAction::Warning` yields `"warning"`; existing error events remain `"error"`, and all other events remain `"ok"`.

Update every `SyncResultJson` and `Summary` constructor in `sync.rs` and its unit tests. Early exits before provider detection use `github_stack: None`.

- [ ] **Step 4: Add a pure sync eligibility helper and failing tests**

Inside `commands/sync.rs`, add a helper returning either a skipped result or `None` to proceed:

```rust
fn github_stack_preflight_skip(
    mode: GithubStacksIntegration,
    until: Option<&str>,
    active_pr_numbers: &[u64],
    has_unresolved_active_pr: bool,
) -> Option<GithubStackSyncResult>
```

Add tests asserting precedence and reasons:

```rust
#[test]
fn github_stack_preflight_off_is_disabled() {
    let result = github_stack_preflight_skip(
        GithubStacksIntegration::Off,
        None,
        &[41, 42],
        false,
    )
    .unwrap();
    assert_eq!(result.reason, Some(GithubStackReason::Disabled));
}

#[test]
fn github_stack_preflight_partial_sync_skips_before_backend() {
    let result = github_stack_preflight_skip(
        GithubStacksIntegration::Auto,
        Some("2"),
        &[41, 42],
        false,
    )
    .unwrap();
    assert_eq!(result.reason, Some(GithubStackReason::PartialSync));
}
```

Also test unresolved active PRs and fewer than two active PRs.

- [ ] **Step 5: Implement sync orchestration before nav-comment reconciliation**

Make the existing `warnings` binding mutable. After the entry loop finishes and the progress bar stops, derive active state by zipping `nav_snapshots` with `entry_is_closed`:

```rust
let active_pr_numbers: Vec<u64> = nav_snapshots
    .iter()
    .zip(&entry_is_closed)
    .filter_map(|(snapshot, is_closed)| {
        if *is_closed {
            None
        } else {
            snapshot.as_ref().map(|value| value.pr_number)
        }
    })
    .collect();
let has_unresolved_active_pr = nav_snapshots
    .iter()
    .zip(&entry_is_closed)
    .any(|(snapshot, is_closed)| !*is_closed && snapshot.is_none());
```

For `Provider::GitHub`, get the mode and either use `github_stack_preflight_skip` or call `reconcile_with_gh`. For GitLab, leave the result `None`.

When `mutation_attempted` is true, immediately set `touched_remote = true` and call `guard.mark_touched_remote()` regardless of backend success.

When the result is a warning, append its message to `warnings`. In human mode render the approved success/dim/warning line; suppress missing/outdated/unsupported `auto` skips. In JSONL mode emit the `GithubStack` event immediately. Store the same result for atomic output or final summary.

- [ ] **Step 6: Add the focused CLI integration fixture**

Register `mod github_stacks;` in `integration_tests/main.rs`. In the new file, build a two-entry GitHub stack with fixed GG-IDs and MR mappings, then install a fake executable `gh` in a prepended `PATH`.

The fake must log every invocation and handle all normal sync calls before the new backend calls:

```sh
if [ "$1" = "--version" ]; then echo "gh version 2.97.0"; exit 0; fi
if [ "$1" = "auth" ] && [ "$2" = "status" ]; then exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  # Return OPEN JSON for mapped PR 41 or 42 with the expected headRefName.
fi
if [ "$1" = "pr" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "api" ] && printf '%s' "$*" | grep -q '/issues/'; then echo '[]'; exit 0; fi
if [ "$1" = "stack" ] && [ "$2" = "--version" ]; then
  echo "gh stack version ${GG_FAKE_STACK_VERSION:-0.1.0}"
  exit "${GG_FAKE_STACK_VERSION_EXIT:-0}"
fi
```

Handle Stacks API responses and link success/failure from environment-selected fixture files. The helper returns the repo path, fake log path, and environment vector so each test changes only its scenario.

- [ ] **Step 7: Add CLI tests for create and JSONL append**

Add:

```rust
#[test]
fn sync_json_creates_native_stack_with_exact_pr_order() {
    // Stacks queries return [] before link and stack #7 after link.
    // Assert success, valid atomic JSON, action=created, stack_number=7,
    // pr_numbers=[41,42], and log contains exactly:
    // stack link --base main 41 42
}

#[test]
fn sync_jsonl_appends_after_merged_prefix_and_repeats_result_in_summary() {
    // Stack response is [merged #40, open #41]; local active list is [41,42].
    // Assert one event=github_stack with action=appended and stack_number=7,
    // the final summary contains the identical object, and the log contains:
    // stack link 7 42
}
```

Parse every JSONL line with `serde_json`, assert the event occurs before summary, and assert stderr is empty.

- [ ] **Step 8: Add CLI tests for safety and non-fatal modes**

Add named tests and exact assertions:

- `sync_until_reports_partial_skip_without_stack_command`: `github_stack.reason == "partial_sync"`; fake log has no `stack --version` or `/stacks`.
- `sync_off_reports_disabled_without_stack_command`: config mode `off`; no capability call.
- `sync_auto_missing_extension_is_silent_and_non_fatal`: successful sync, `action == "skipped"`, `reason == "missing_extension"`, empty warnings/stderr.
- `sync_force_missing_extension_is_warning_and_non_fatal`: successful sync, `action == "warning"`, `reason == "missing_extension"`, warning appears in summary.
- `sync_divergence_warns_without_link`: reordered remote active list; no `stack link` call.
- `sync_link_failure_warns_without_corrupting_json`: link exits non-zero with stderr; JSON remains one valid document and sync exits success.
- `sync_gitlab_never_invokes_or_serializes_github_stacks`: use a fake `glab`,
  assert no `gh` executable is needed, and assert `sync.github_stack` is null.

- [ ] **Step 9: Update unrelated sync fixtures for default-auto compatibility**

Search all multi-entry sync integration configs and fake `gh` scripts:

```sh
rg -n 'sync|fake_bin.join\("gh"\)|provider.*github' crates/gg-cli/tests/integration_tests/sync.rs
```

For tests unrelated to native stacks whose scripts intentionally reject unknown calls, add this nested config:

```json
"github": { "stacks_integration": "off" }
```

Do not weaken the dedicated GitHub Stacks tests. One-entry sync tests naturally skip before extension detection and need no fixture change.

- [ ] **Step 10: Run focused sync/output/integration tests**

Run:

```sh
cargo test -p gg-core output::tests::sync_streaming_github_stack -- --nocapture
cargo test -p gg-core commands::sync::tests::github_stack_preflight -- --nocapture
cargo test -p gg-cli --test integration_tests github_stacks -- --nocapture
cargo test -p gg-cli --test integration_tests sync -- --nocapture
```

Expected: all PASS; structured stdout contains no raw extension output.

- [ ] **Step 11: Run crate-wide checks and commit**

Run:

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Expected: all PASS.

Commit:

```sh
git add crates/gg-core/src/output.rs crates/gg-core/src/commands/sync.rs \
  crates/gg-cli/tests/integration_tests/main.rs \
  crates/gg-cli/tests/integration_tests/github_stacks.rs \
  crates/gg-cli/tests/integration_tests/sync.rs
git commit -m "feat(sync): reconcile native GitHub stacks"
```

---

### Task 5: Document and Verify the Public Preview Integration

**Files:**
- Modify: `README.md`
- Modify: `docs/src/configuration.md`
- Modify: `docs/src/commands/setup.md`
- Modify: `docs/src/commands/sync.md`
- Modify: `docs/src/getting-started.md`

**Interfaces:**
- Consumes: the exact configuration names, result fields, eligibility, and recovery behavior implemented in Tasks 1-4.
- Produces: user guidance only; no code or agent-skill behavior changes.

- [ ] **Step 1: Update README feature and prerequisite guidance**

Add native GitHub Stacked PR UI integration to the feature overview, explain that it is a GitHub public preview, and give the optional installation command:

```sh
gh extension install github/gh-stack
```

Add `defaults.github.stacks_integration` to the configuration table with values `off`, `auto`, `force` and default `auto`. State that ordinary GitHub sync remains functional without the extension.

- [ ] **Step 2: Update the mdBook configuration and setup reference**

In `docs/src/configuration.md`, include the nested JSON example and a table row using the exact mode semantics. In `docs/src/commands/setup.md`, add the GitHub full-setup selector and state that it appears only when GitHub is selected.

Use the product term `GitHub Stacked PRs` and command name `gh stack`; do not call git-gud's own stack metadata a GitHub Stack.

- [ ] **Step 3: Update sync behavior and output contracts**

In `docs/src/commands/sync.md`, add sections covering:

- full GitHub sync eligibility and active open/draft PR collection
- create and stack-number append behavior
- merged-prefix preservation
- create/append-only divergence warning and explicit recovery
- `auto` versus `force` capability behavior
- non-fatal failures
- the atomic `sync.github_stack` object
- the JSONL `github_stack` event and repeated summary result

Include one JSON example with all fields and one JSONL event example whose `status` is `warning` for divergence.

- [ ] **Step 4: Update getting-started guidance**

In `docs/src/getting-started.md`, keep `gh` as the required GitHub client and list `github/gh-stack` as optional for the native stack map/UI. Do not imply the extension is required for PR creation or ordinary sync.

- [ ] **Step 5: Build documentation and inspect links**

Run:

```sh
mdbook build docs
rg -n 'stacks_integration|gh extension install github/gh-stack|github_stack' \
  README.md docs/src
```

Expected: mdBook build PASS; each public config/output term appears in the intended guide and reference pages.

- [ ] **Step 6: Verify the official extension version in isolation**

Use a narrowly scoped temporary config directory and perform no repository stack mutation:

```sh
gg_stack_tmp=$(mktemp -d)
GH_CONFIG_DIR="$gg_stack_tmp" gh extension install github/gh-stack
GH_CONFIG_DIR="$gg_stack_tmp" gh stack --version
rm -rf "$gg_stack_tmp"
```

Expected: installation succeeds and version is `0.1.0` or later. The deletion target is the explicit directory returned by `mktemp -d`.

- [ ] **Step 7: Run the complete project verification matrix**

Run in this order:

```sh
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
mdbook build docs
git diff --check
```

Expected: every command exits zero.

- [ ] **Step 8: Review the final diff against the approved spec**

Run:

```sh
git status --short
git diff --stat HEAD~5
git diff HEAD~5 -- crates/gg-core/src/github_stacks.rs \
  crates/gg-core/src/commands/sync.rs crates/gg-core/src/output.rs
```

Confirm all of these directly in the diff:

- no direct REST stack mutation
- no automatic extension installation
- no GitLab call path
- no partial-sync backend call
- only numeric `gh stack link` arguments
- mutation attempt marks remote touch
- JSON stays atomic and JSONL stays line-delimited
- no `skills/gg` changes

- [ ] **Step 9: Commit the documentation deliverable**

```sh
git add README.md docs/src/configuration.md docs/src/commands/setup.md \
  docs/src/commands/sync.md docs/src/getting-started.md
git commit -m "docs: document native GitHub Stacks integration"
```

- [ ] **Step 10: Record final evidence**

Run:

```sh
git status --short --branch
git log --oneline --decorate -7
```

Expected: clean worktree; the design commit, plan commit, and five
implementation commits are visible on `nacho/gh-stacked`.
