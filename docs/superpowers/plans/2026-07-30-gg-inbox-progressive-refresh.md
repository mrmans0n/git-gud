# Progressive `gg inbox` Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `gg inbox` render useful human and machine-readable results immediately while reducing redundant provider requests and preserving a deterministic final snapshot.

**Architecture:** Discover immutable local candidates first, refresh complete provider snapshots with a four-worker bounded scheduler, and feed completions to one coordinator. The coordinator alone renders stable terminal rows or emits completion-order JSONL events, then builds the same deterministic final inbox used by atomic JSON.

**Tech Stack:** Rust 2021, `std::thread` scoped workers, `std::sync::mpsc`, `indicatif`, `serde`/`serde_json`, `git2`, Clap, fake `gh`/`glab` integration executables, mdBook.

## Global Constraints

- Process at most four PRs or MRs concurrently; do not add a concurrency flag or setting.
- GitHub inbox refresh uses one `gh pr view --json` request per candidate.
- GitLab inbox refresh reuses one `glab mr view --output json` response and performs one separate approval request.
- `--json` remains one deterministic aggregate document.
- `--jsonl` emits completion-order entry events and a deterministic final summary.
- Live human rows appear only when both stdout and stderr are terminals; redirected output remains final-only.
- Provider refresh failures remain visible in `refresh_failed`, emit structured errors, and do not make the command exit nonzero.
- Provider detection failure, a missing provider CLI, and repository-wide discovery failure remain fatal.
- Keep `stack_inbox` on atomic `gg inbox --json`; do not add streaming MCP transport.
- Add no persistent cache, provider-wide batch API, job framework, or new dependency.
- Preserve existing successful bucket classification and provider-specific PR/MR labels.
- Every behavior change requires tests, `cargo fmt`, warning-free Clippy, and the all-feature test suite.

---

## File Structure

### New files

- `crates/gg-core/src/commands/inbox/refresh.rs`
  - Fixed-width worker scheduling and completion delivery.
  - Concurrency/order/error-isolation unit tests.
- `crates/gg-core/src/commands/inbox/render.rs`
  - Stable `indicatif::MultiProgress` terminal rows.
  - TTY gating, row-state formatting, clearing, and terminal tests.

### Modified files

- `crates/gg-core/src/gh.rs`
  - Parse PR details, review state, mergeability, and CI from one response.
- `crates/gg-core/src/glab.rs`
  - Parse MR details and CI from one typed response, then query approval.
- `crates/gg-core/src/provider.rs`
  - Provider-neutral `InboxSnapshot` and dispatch method.
- `crates/gg-core/src/commands/inbox.rs`
  - Candidate discovery, coordinator, classification, filtering, and final rendering.
- `crates/gg-core/src/output.rs`
  - Additive atomic inbox schema and inbox streaming event schema.
- `crates/gg-cli/src/main.rs`
  - `--jsonl` flag, `InboxOptions`, dispatch, and generic streaming fatal errors.
- `crates/gg-cli/tests/integration_tests/inbox.rs`
  - Fake-provider request-count, JSONL lifecycle, and partial-failure coverage.
- `README.md`
  - Progressive/streaming inbox command summary.
- `docs/src/commands/inbox.md`
  - Human UX, JSONL events, and failure semantics.
- `docs/src/mcp-server.md`
  - Additive `refresh_failed` atomic response behavior.
- `skills/gg/SKILL.md`
  - Generalize the JSONL guidance from sync-only to supported streaming commands.
- `skills/gg/references/setup-and-inspection.md`
  - Select atomic or streaming inbox inspection based on the caller's need.

No Cargo manifest changes are expected.

---

### Task 1: Add Complete Provider Inbox Snapshots

**Files:**
- Modify: `crates/gg-core/src/gh.rs:23-49,182-230,423-493,680-750`
- Modify: `crates/gg-core/src/glab.rs:20-49,309-350,546-645,1720-1760`
- Modify: `crates/gg-core/src/provider.rs:37-64,189-218,363-383`

**Interfaces:**
- Produces:

```rust
// crates/gg-core/src/provider.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxSnapshot {
    pub state: PrState,
    pub url: String,
    pub approved: bool,
    pub changes_requested: bool,
    pub mergeable: bool,
    pub ci_status: Option<CiStatus>,
}

impl Provider {
    pub fn get_inbox_snapshot(&self, number: u64) -> Result<InboxSnapshot>;
}
```

- `ci_status: None` means the provider returned no checks or pipeline.
- `ci_status: Some(CiStatus::Unknown)` means checks existed but could not be mapped to a known aggregate state.
- Later tasks consume only `Provider::get_inbox_snapshot`; they do not call the three legacy inbox methods.

- [ ] **Step 1: Add failing GitHub combined-snapshot parser tests**

Add focused tests in `gh.rs` for a response containing `reviewDecision`,
`mergeable`, and `statusCheckRollup`:

```rust
#[test]
fn inbox_snapshot_parses_review_and_ci_from_one_response() {
    let json = br#"{
        "number": 42,
        "title": "Add login",
        "state": "OPEN",
        "url": "https://github.com/acme/app/pull/42",
        "headRefName": "nacho/auth/c-abc1234",
        "isDraft": false,
        "mergeable": "MERGEABLE",
        "reviewDecision": "APPROVED",
        "statusCheckRollup": [
            {"status": "COMPLETED", "conclusion": "SUCCESS"}
        ]
    }"#;

    let snapshot = parse_inbox_snapshot(json).unwrap();
    assert_eq!(snapshot.state, PrState::Open);
    assert!(snapshot.approved);
    assert!(!snapshot.changes_requested);
    assert!(snapshot.mergeable);
    assert_eq!(snapshot.ci_status, Some(CiStatus::Success));
}

#[test]
fn inbox_snapshot_treats_empty_rollup_as_no_ci() {
    let json = br#"{
        "number": 43,
        "title": "No checks",
        "state": "OPEN",
        "url": "https://github.com/acme/app/pull/43",
        "isDraft": false,
        "mergeable": "MERGEABLE",
        "reviewDecision": "CHANGES_REQUESTED",
        "statusCheckRollup": []
    }"#;

    let snapshot = parse_inbox_snapshot(json).unwrap();
    assert!(snapshot.changes_requested);
    assert_eq!(snapshot.ci_status, None);
}
```

Add table cases proving failure/cancellation wins over success, an in-progress
check maps to `Pending`, and an unmapped non-empty rollup maps to `Unknown`.

- [ ] **Step 2: Run the GitHub parser tests and verify they fail**

Run:

```bash
rtk cargo test -p gg-core --all-features gh::tests::inbox_snapshot
```

Expected: compilation fails because `parse_inbox_snapshot` and the rollup
fields do not exist.

- [ ] **Step 3: Implement the GitHub combined snapshot**

Add private response types and a public low-level snapshot:

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhStatusCheck {
    conclusion: Option<String>,
    status: Option<String>,
    state: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InboxPrSnapshot {
    pub state: PrState,
    pub url: String,
    pub approved: bool,
    pub changes_requested: bool,
    pub mergeable: bool,
    pub ci_status: Option<CiStatus>,
}

fn parse_inbox_snapshot(bytes: &[u8]) -> Result<InboxPrSnapshot>;
pub fn get_inbox_snapshot(pr_number: u64) -> Result<InboxPrSnapshot>;
```

Extend `GhPrJson` with a defaulted `status_check_rollup`. Have
`get_inbox_snapshot` execute exactly:

```rust
Command::new("gh").args([
    "pr",
    "view",
    &pr_number.to_string(),
    "--json",
    "number,title,state,url,headRefName,isDraft,mergeable,reviewDecision,statusCheckRollup",
])
```

Aggregate checks in this priority order:

1. `FAILURE`, `FAILED`, `TIMED_OUT`, or `ACTION_REQUIRED` -> `Failed`
2. `CANCELLED` or `CANCELED` -> `Canceled`
3. non-completed status, empty conclusion, `PENDING`, `QUEUED`, or
   `IN_PROGRESS` -> `Pending`
4. at least one `SUCCESS`, `NEUTRAL`, or `SKIPPED` and no earlier state ->
   `Success`
5. non-empty but unmapped -> `Unknown`
6. empty rollup -> `None`

Keep the legacy methods for other callers; only inbox switches to the new
combined method.

- [ ] **Step 4: Run the GitHub tests and verify they pass**

Run:

```bash
rtk cargo test -p gg-core --all-features gh::tests::inbox_snapshot
```

Expected: all combined-snapshot parser tests pass.

- [ ] **Step 5: Add failing GitLab typed-snapshot tests**

Extend `GlabMrJson` test fixtures with `head_pipeline` and add:

```rust
#[test]
fn inbox_snapshot_parses_typed_pipeline_status() {
    let json = br#"{
        "iid": 52,
        "title": "Add login",
        "state": "opened",
        "web_url": "https://gitlab.com/acme/app/-/merge_requests/52",
        "source_branch": "nacho/auth/c-abc1234",
        "draft": false,
        "detailed_merge_status": "mergeable",
        "head_pipeline": {"status": "running"}
    }"#;

    let details = parse_inbox_mr_details(json).unwrap();
    assert_eq!(details.state, MrState::Open);
    assert_eq!(details.ci_status, Some(CiStatus::Running));
}
```

Add cases for `success`, `failed`, `pending`, `canceled`, a missing pipeline,
and an unknown non-empty pipeline status.

- [ ] **Step 6: Run the GitLab parser tests and verify they fail**

Run:

```bash
rtk cargo test -p gg-core --all-features glab::tests::inbox_snapshot
```

Expected: compilation fails because typed pipeline parsing does not exist.

- [ ] **Step 7: Implement GitLab details reuse and provider-neutral dispatch**

Add:

```rust
#[derive(Debug, Deserialize)]
struct GlabPipelineJson {
    status: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InboxMrDetails {
    pub state: MrState,
    pub web_url: String,
    pub mergeable: bool,
    pub changes_requested: bool,
    pub ci_status: Option<CiStatus>,
}

fn parse_inbox_mr_details(bytes: &[u8]) -> Result<InboxMrDetails>;
pub fn get_inbox_snapshot(mr_number: u64) -> Result<(InboxMrDetails, bool)>;
```

`glab::get_inbox_snapshot` runs `glab mr view ... --output json` once, parses
details and CI, then calls `check_mr_approved` once. A failure from either
subprocess returns `Err`.

Add the provider-neutral `InboxSnapshot` and map the GitHub/GitLab low-level
types in `Provider::get_inbox_snapshot`.

- [ ] **Step 8: Run provider tests**

Run:

```bash
rtk cargo test -p gg-core --all-features gh::tests::inbox_snapshot
rtk cargo test -p gg-core --all-features glab::tests::inbox_snapshot
rtk cargo test -p gg-core --all-features provider::tests
```

Expected: all pass.

- [ ] **Step 9: Commit the provider snapshot boundary**

```bash
rtk cargo fmt --all
rtk git add crates/gg-core/src/gh.rs crates/gg-core/src/glab.rs crates/gg-core/src/provider.rs
rtk git commit -m "feat(core): add inbox provider snapshots"
```

---

### Task 2: Separate Local Discovery and Adopt Snapshot Classification

**Files:**
- Modify: `crates/gg-core/src/commands/inbox.rs:16-389,391-555,562-890`
- Modify: `crates/gg-core/src/output.rs:387-430,525-585`
- Modify: `crates/gg-cli/tests/integration_tests/inbox.rs:1-243`

**Interfaces:**
- Consumes: `Provider::get_inbox_snapshot(u64) -> Result<InboxSnapshot>`.
- Produces:

```rust
// crates/gg-core/src/commands/inbox.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InboxCandidate {
    pub discovery_index: usize,
    pub stack_name: String,
    pub position: usize,
    pub short_sha: String,
    pub title: String,
    pub pr_number: u64,
    pub behind_base: Option<usize>,
}

pub(super) struct InboxDiscovery {
    pub candidates: Vec<InboxCandidate>,
    pub stack_errors: Vec<StackLoadError>,
}

fn discover_candidates(
    repo: &git2::Repository,
    config: &Config,
) -> Result<InboxDiscovery>;
```

- `InboxItem` gains `remote_state: Option<PrState>` and
  `refresh_error: Option<String>`.
- `InboxItem` provides:

```rust
fn from_snapshot(
    candidate: InboxCandidate,
    snapshot: InboxSnapshot,
    provider: Provider,
) -> Self;

fn from_refresh_error(
    candidate: InboxCandidate,
    error: String,
    provider: Provider,
) -> Self;
```

- Atomic `InboxResponse` gains `buckets.refresh_failed`; successful entries
  omit `refresh_error`.

- [ ] **Step 1: Add failing refresh-failure bucketing and JSON tests**

Add `refresh_failed` to `BucketInput` test construction and assert it wins over
every remote state:

```rust
#[test]
fn refresh_failure_has_highest_priority() {
    let input = BucketInput {
        refresh_failed: true,
        mr_state: PrState::Merged,
        ci_status: Some(CiStatus::Success),
        approved: true,
        changes_requested: true,
        mergeable: true,
        behind_base: true,
    };

    assert_eq!(bucket(&input), Some(ActionBucket::RefreshFailed));
}
```

Extend `output.rs` serialization coverage:

```rust
assert_eq!(
    json["buckets"]["refresh_failed"][0]["refresh_error"],
    "failed to refresh PR #42"
);
assert!(json["buckets"]["ready_to_land"][0]
    .get("refresh_error")
    .is_none());
```

- [ ] **Step 2: Run the focused tests and verify they fail**

Run:

```bash
rtk cargo test -p gg-core --all-features commands::inbox::tests::refresh_failure
rtk cargo test -p gg-core --all-features output::tests::inbox_response_serializes
```

Expected: compilation or assertions fail because the new bucket and field are
absent.

- [ ] **Step 3: Add the refresh-failure model and additive atomic schema**

Add `ActionBucket::RefreshFailed` first in display order. Add
`refresh_failed: bool` to `BucketInput` and return `RefreshFailed` before
checking `mr_state`.

Change the JSON types to:

```rust
#[derive(Serialize, Clone)]
pub struct InboxBucketsJson {
    pub refresh_failed: Vec<InboxEntryJson>,
    // existing buckets remain in their current schema order
}

#[derive(Serialize, Clone)]
pub struct InboxEntryJson {
    // existing fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_error: Option<String>,
}
```

Render `Refresh failed` before the existing groups. Keep successful bucketing
rules unchanged.

- [ ] **Step 4: Extract local candidate discovery**

Move username inference, stack/base loading, MR-number mapping, and
behind-base calculation into `discover_candidates`. Preserve:

- duplicate `(stack_name, full_branch)` suppression
- invalid username filtering
- stale configured-stack skipping
- same-name stacks under different usernames
- stack-specific `StackLoadError` collection

Assign `discovery_index` only when a mapped PR/MR becomes a candidate. Do not
perform provider detection or provider subprocess work inside discovery.

- [ ] **Step 5: Adopt complete snapshots serially before adding concurrency**

Keep this task serial so request reduction and classification can be reviewed
separately from scheduling. After discovery:

```rust
if discovery.candidates.is_empty() {
    // Reuse the existing empty human/JSON branches, including stack_errors,
    // and return Ok(()) without detecting or invoking a provider.
}

let provider = Provider::detect(&repo)?;
provider.check_installed()?;

for candidate in discovery.candidates {
    let completion = match provider.get_inbox_snapshot(candidate.pr_number) {
        Ok(snapshot) => InboxItem::from_snapshot(candidate, snapshot, provider),
        Err(error) => InboxItem::from_refresh_error(candidate, error.to_string(), provider),
    };
    items.push(completion);
}
```

Do not call `get_pr_info`, `get_pr_ci_status`, or `check_pr_approved` directly
from `inbox.rs`. A refresh error remains included even without `--all`.
Successful merged entries remain controlled by `--all`; closed entries remain
excluded.

- [ ] **Step 6: Add fake-provider request-count and failure integration tests**

Add a local helper that writes an executable `gh` script and uses
`run_gg_with_env` with an isolated `PATH`. The script must handle `--version`
without counting it as a PR request:

```rust
fs::write(
    fake_bin.join("gh"),
    r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "gh version test"
  exit 0
fi
printf '%s\n' "$*" >> "$GG_FAKE_LOG"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  printf '%s\n' '{"number":42,"title":"Inbox item","state":"OPEN","url":"https://github.com/acme/app/pull/42","headRefName":"testuser/inbox-copy/c-abc1234","isDraft":false,"mergeable":"MERGEABLE","reviewDecision":"APPROVED","statusCheckRollup":[{"status":"COMPLETED","conclusion":"SUCCESS"}]}'
  exit 0
fi
exit 1
"#,
)?;
```

Assert one candidate produces exactly one logged `pr view` line and lands in
`ready_to_land`.

Add a failing-script case and assert:

- process exits zero
- `buckets.refresh_failed` contains the entry
- `refresh_error` is present
- the entry does not appear in `awaiting_review`

- [ ] **Step 7: Run inbox unit and integration tests**

Run:

```bash
rtk cargo test -p gg-core --all-features commands::inbox
rtk cargo test -p gg-core --all-features output::tests::inbox
rtk cargo test -p gg-cli --all-features --test integration_tests inbox
```

Expected: all pass and the fake log contains one provider snapshot request per
GitHub candidate.

- [ ] **Step 8: Commit discovery and classification**

```bash
rtk cargo fmt --all
rtk git add crates/gg-core/src/commands/inbox.rs crates/gg-core/src/output.rs crates/gg-cli/tests/integration_tests/inbox.rs
rtk git commit -m "refactor(inbox): separate discovery from refresh"
```

---

### Task 3: Refresh Candidates with Four Bounded Workers

**Files:**
- Create: `crates/gg-core/src/commands/inbox/refresh.rs`
- Modify: `crates/gg-core/src/commands/inbox.rs:1-20,149-389`

**Interfaces:**
- Consumes: `InboxCandidate`, `InboxSnapshot`.
- Produces:

```rust
pub(super) const MAX_INBOX_REFRESH_WORKERS: usize = 4;

#[derive(Debug)]
pub(super) struct InboxCompletion {
    pub candidate: InboxCandidate,
    pub result: std::result::Result<InboxSnapshot, String>,
}

pub(super) fn refresh_candidates<F, C>(
    candidates: &[InboxCandidate],
    refresh: F,
    mut on_completion: C,
)
where
    F: Fn(&InboxCandidate) -> std::result::Result<InboxSnapshot, String> + Sync,
    C: FnMut(InboxCompletion);
```

- `refresh_candidates` calls the callback on its caller/coordinator thread, not
  on worker threads.

- [ ] **Step 1: Create failing bounded-concurrency tests**

In `refresh.rs`, construct eight candidates and use atomics plus a four-party
barrier:

```rust
#[test]
fn refreshes_at_most_four_candidates_at_once() {
    let active = AtomicUsize::new(0);
    let maximum = AtomicUsize::new(0);
    let first_wave = Barrier::new(4);
    let candidates = candidates(8);

    refresh_candidates(
        &candidates,
        |candidate| {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            maximum.fetch_max(now, Ordering::SeqCst);
            if candidate.discovery_index < 4 {
                first_wave.wait();
            }
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(snapshot())
        },
        |_| {},
    );

    assert_eq!(maximum.load(Ordering::SeqCst), 4);
}
```

Add tests for zero candidates, fewer than four candidates, every candidate
exactly once, and worker errors not canceling later candidates.

- [ ] **Step 2: Add a failing completion-order test without sleeps**

Use a `Condvar` so candidate zero waits until candidate one sets a flag:

```rust
#[test]
fn reports_fast_later_candidate_before_blocked_first_candidate() {
    let gate = (Mutex::new(false), Condvar::new());
    let mut order = Vec::new();

    refresh_candidates(
        &candidates(2),
        |candidate| {
            if candidate.discovery_index == 0 {
                let (lock, ready) = &gate;
                let released = lock.lock().unwrap();
                drop(ready.wait_while(released, |value| !*value).unwrap());
            } else {
                let (lock, ready) = &gate;
                *lock.lock().unwrap() = true;
                ready.notify_one();
            }
            Ok(snapshot())
        },
        |completion| order.push(completion.candidate.discovery_index),
    );

    assert_eq!(order, vec![1, 0]);
}
```

- [ ] **Step 3: Run refresh tests and verify they fail**

Run:

```bash
rtk cargo test -p gg-core --all-features commands::inbox::refresh::tests
```

Expected: compilation fails because the refresh module and scheduler do not
exist.

- [ ] **Step 4: Implement the scoped worker scheduler**

Use `std::thread::scope`, an `AtomicUsize` next-index counter, and one
`mpsc::channel`:

```rust
let worker_count = candidates.len().min(MAX_INBOX_REFRESH_WORKERS);
if worker_count == 0 {
    return;
}

thread::scope(|scope| {
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();

    for _ in 0..worker_count {
        let sender = sender.clone();
        let next = &next;
        let refresh = &refresh;
        scope.spawn(move || loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(candidate) = candidates.get(index).cloned() else {
                break;
            };
            let result = refresh(&candidate);
            if sender.send(InboxCompletion { candidate, result }).is_err() {
                break;
            }
        });
    }

    drop(sender);
    for completion in receiver {
        on_completion(completion);
    }
});
```

Do not spawn provider subprocesses outside this function and do not print from
workers.

- [ ] **Step 5: Integrate the scheduler into inbox coordination**

Replace the serial snapshot loop with:

```rust
let mut completions: Vec<Option<InboxCompletion>> =
    (0..discovery.candidates.len()).map(|_| None).collect();
refresh_candidates(
    &discovery.candidates,
    |candidate| {
        provider
            .get_inbox_snapshot(candidate.pr_number)
            .map_err(|error| error.to_string())
    },
    |completion| {
        let index = completion.candidate.discovery_index;
        completions[index] = Some(completion);
    },
);
```

Build final items by iterating `completions` in index order. Treat a missing
slot as an internal `GgError::Other` rather than silently dropping a candidate.

- [ ] **Step 6: Run concurrency and inbox regressions**

Run:

```bash
rtk cargo test -p gg-core --all-features commands::inbox::refresh::tests
rtk cargo test -p gg-core --all-features commands::inbox
rtk cargo test -p gg-cli --all-features --test integration_tests inbox
```

Expected: all pass; the controlled order test reports `[1, 0]`, while atomic
JSON remains in discovery order.

- [ ] **Step 7: Commit bounded refresh**

```bash
rtk cargo fmt --all
rtk git add crates/gg-core/src/commands/inbox.rs crates/gg-core/src/commands/inbox/refresh.rs
rtk git commit -m "perf(inbox): refresh snapshots concurrently"
```

---

### Task 4: Stream Inbox JSONL Events

**Files:**
- Modify: `crates/gg-core/src/output.rs:1-85,176-255,387-430,610-720`
- Modify: `crates/gg-core/src/commands/inbox.rs:149-555`
- Modify: `crates/gg-cli/src/main.rs:515-528,560-575,871-875,905-930`
- Modify: `crates/gg-cli/tests/integration_tests/inbox.rs:1-243`

**Interfaces:**
- Consumes: completion callbacks from `refresh_candidates`.
- Produces:

```rust
#[derive(Debug, Clone, Copy)]
pub struct InboxOptions {
    pub all: bool,
    pub json: bool,
    pub jsonl: bool,
}

pub struct InboxStreamingResponse {
    pub version: u32,
    pub command: String,
    pub event: InboxStreamingEvent,
}

#[derive(Serialize)]
pub struct InboxCandidateJson {
    pub stack_name: String,
    pub position: usize,
    pub sha: String,
    pub title: String,
    pub pr_number: u64,
    pub behind_base: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum InboxStreamingEvent {
    Start { total_candidates: usize, total_stack_errors: usize },
    StackError { stack_name: String, error: String },
    Entry {
        completed: usize,
        total_candidates: usize,
        included: bool,
        bucket: Option<String>,
        remote_state: String,
        entry: InboxEntryJson,
    },
    EntryError {
        completed: usize,
        total_candidates: usize,
        included: bool,
        bucket: String,
        entry: InboxCandidateJson,
        error: String,
    },
    Summary {
        total_items: usize,
        buckets: InboxBucketsJson,
        stack_errors: Vec<InboxStackErrorJson>,
    },
}
```

- `InboxStreamingResponse` serializes `version`, `command`, and the tagged event
  into one flat object, matching `SyncStreamingResponse`.
- `StreamingErrorResponse` is a generic flat fatal-error event used by both
  sync and inbox in CLI dispatch.

- [ ] **Step 1: Add failing streaming serialization tests**

In `output.rs`, add tests that serialize start, included entry, excluded merged
entry, entry error, and summary. Assert:

```rust
assert_eq!(json["version"], 1);
assert_eq!(json["command"], "inbox");
assert_eq!(json["event"], "entry");
assert_eq!(json["included"], true);
assert_eq!(json["bucket"], "ready_to_land");
```

For an excluded entry assert `included == false` and `bucket.is_null()`.
For `EntryError`, assert `bucket == "refresh_failed"`.

- [ ] **Step 2: Run serialization tests and verify they fail**

Run:

```bash
rtk cargo test -p gg-core --all-features output::tests::inbox_streaming
```

Expected: compilation fails because inbox streaming types do not exist.

- [ ] **Step 3: Implement inbox streaming and generic fatal-error envelopes**

Implement `InboxStreamingResponse::serialize` using the same
`serde_json::to_value` flattening pattern as sync.

Add:

```rust
#[derive(Serialize)]
pub struct StreamingErrorResponse<'a> {
    pub version: u32,
    pub command: &'a str,
    pub status: &'static str,
    pub event: &'static str,
    pub message: String,
}
```

Construct it with `status: "error"` and `event: "error"`. Replace the
sync-specific fatal error object in `main.rs` with this generic envelope so
inbox fatal errors report `command: "inbox"`.

- [ ] **Step 4: Add the CLI flag and option object**

Change the Clap variant to:

```rust
Inbox {
    #[arg(short, long)]
    all: bool,
    #[arg(long)]
    json: bool,
    #[arg(long = "jsonl", conflicts_with = "json")]
    jsonl: bool,
}
```

Dispatch through `InboxOptions`. Track the active streaming command before the
match result is handled:

```rust
let mut streaming_command: Option<&'static str> = None;
```

Set it to `Some("sync")` or `Some("inbox")` only when that command's `jsonl`
flag is true. Use it to emit `StreamingErrorResponse` on a fatal `Err`.
The inbox dispatch tuple is:

```rust
(
    gg_core::commands::inbox::run(InboxOptions { all, json, jsonl }),
    json || jsonl,
    jsonl,
)
```

- [ ] **Step 5: Emit lifecycle events from the coordinator**

When `jsonl` is selected:

1. Emit `Start` after discovery.
2. Emit each `StackError` in discovery order.
3. In the refresh completion callback, increment `completed` and emit `Entry`
   or `EntryError` immediately.
4. After collecting and sorting final items, emit `Summary` last.

Every successful candidate emits `Entry`, including merged/closed candidates
that will be excluded. Set:

- merged without `--all`: `included: false`, `bucket: None`
- closed: `included: false`, `bucket: None`
- refresh failure: `included: true`, `bucket: "refresh_failed"`

For an empty inbox, emit `Start` followed immediately by `Summary`.
Suppress the static human refresh line in both structured modes.

- [ ] **Step 6: Add JSONL CLI integration tests**

Add tests for:

- help contains `--jsonl`
- `--json --jsonl` is rejected by Clap
- empty inbox emits exactly `start`, `summary`
- fake-provider success emits `start`, `entry`, `summary`
- fake-provider failure emits `start`, `entry_error`, `summary` and exits zero
- every line parses independently and stderr is empty
- final summary matches an atomic `--json` run after removing the streaming
  envelope fields

Parse lines with:

```rust
let events: Vec<Value> = stdout
    .lines()
    .map(|line| serde_json::from_str(line).expect("every JSONL line must parse"))
    .collect();
```

- [ ] **Step 7: Run streaming tests**

Run:

```bash
rtk cargo test -p gg-core --all-features output::tests::inbox_streaming
rtk cargo test -p gg-cli --all-features --test integration_tests inbox
rtk cargo test -p gg-cli --all-features --test integration_tests sync::test_gg_sync_jsonl
```

Expected: all inbox JSONL tests pass and sync fatal-error compatibility remains
green.

- [ ] **Step 8: Commit streaming output**

```bash
rtk cargo fmt --all
rtk git add crates/gg-core/src/output.rs crates/gg-core/src/commands/inbox.rs crates/gg-cli/src/main.rs crates/gg-cli/tests/integration_tests/inbox.rs
rtk git commit -m "feat(inbox): stream refresh events"
```

---

### Task 5: Render Stable Live Human Rows

**Files:**
- Create: `crates/gg-core/src/commands/inbox/render.rs`
- Modify: `crates/gg-core/src/commands/inbox.rs:1-20,149-555`

**Interfaces:**
- Consumes: ordered candidates and `InboxCompletion` values.
- Produces:

```rust
pub(super) enum InboxRowState<'a> {
    Refreshing,
    Bucket(ActionBucket),
    MergedHidden,
    ClosedHidden,
    RefreshFailed(&'a str),
}

pub(super) struct LiveInboxRenderer {
    // MultiProgress, one row per candidate, and one aggregate counter
}

impl LiveInboxRenderer {
    pub fn stderr_if_interactive(
        candidates: &[InboxCandidate],
        provider_label: &str,
    ) -> Option<Self>;

    fn with_draw_target(
        candidates: &[InboxCandidate],
        provider_label: &str,
        draw_target: ProgressDrawTarget,
    ) -> Self;

    pub fn update(&mut self, discovery_index: usize, state: InboxRowState<'_>);
    pub fn finish_and_clear(&mut self);
}

pub(super) fn live_rendering_enabled(
    stdout_is_terminal: bool,
    stderr_is_terminal: bool,
) -> bool;
```

- `Drop` clears a live renderer so provider-preflight and command-level errors
  do not leave stale terminal rows.

- [ ] **Step 1: Add failing TTY-gating and row-format tests**

Test all four TTY combinations:

```rust
#[test]
fn live_rendering_requires_both_output_streams_to_be_terminals() {
    assert!(live_rendering_enabled(true, true));
    assert!(!live_rendering_enabled(true, false));
    assert!(!live_rendering_enabled(false, true));
    assert!(!live_rendering_enabled(false, false));
}
```

Add pure row-format assertions for:

- `refreshing`
- each final bucket label
- `merged (hidden; use --all)`
- `closed (hidden)`
- `refresh failed`

- [ ] **Step 2: Add a failing in-memory terminal stability test**

Create a `RecordingTerm` implementing `indicatif::TermLike` and recording
written strings behind `Arc<Mutex<Vec<String>>>`. Construct three rows with
`with_draw_target`, update discovery index 1, then assert:

- all three initial candidate labels were rendered
- index 1 contains its new status
- index 0 and index 2 retain their original identity/order
- the aggregate message advances from `refreshed 0/3` to `refreshed 1/3`

Clear the renderer and assert a terminal clear operation was recorded.

- [ ] **Step 3: Run renderer tests and verify they fail**

Run:

```bash
rtk cargo test -p gg-core --all-features commands::inbox::render::tests
```

Expected: compilation fails because the renderer module does not exist.

- [ ] **Step 4: Implement stable `MultiProgress` rows**

Use `MultiProgress::with_draw_target`, one spinner `ProgressBar` per candidate,
and one aggregate progress bar. Add rows in discovery order and never remove or
reinsert them.

Use a row template equivalent to:

```text
{spinner:.green} {msg}
```

Format the message from immutable candidate identity plus the current status.
Enable a 100 ms steady tick while a row is refreshing. Stop its spinner when
the row reaches a completed state, but leave the row visible until the entire
renderer clears.

Use `std::io::IsTerminal` in `stderr_if_interactive`. Production draw target is
`ProgressDrawTarget::stderr()`.

- [ ] **Step 5: Wire live updates into the coordinator**

Detect the provider after discovery, use it to obtain `mr_label`, create the
renderer, and then check CLI availability. This puts the stable rows on screen
before the `--version` subprocess while still keeping provider detection fatal:

```rust
let provider = Provider::detect(&repo)?;
let mut renderer = if !options.json && !options.jsonl {
    LiveInboxRenderer::stderr_if_interactive(
        &discovery.candidates,
        provider.pr_label(),
    )
} else {
    None
};
provider.check_installed()?;
```

In the completion callback, compute the visible row state without changing
final ordering:

- successful included entry -> `Bucket(bucket)`
- merged without `--all` -> `MergedHidden`
- closed -> `ClosedHidden`
- provider error -> `RefreshFailed(error)`

Call `finish_and_clear` before printing the final grouped human inbox. Remove
the old static `Refreshing ... done` stderr output. Non-TTY human mode performs
no progress writes.

- [ ] **Step 6: Run renderer and human-output regressions**

Run:

```bash
rtk cargo test -p gg-core --all-features commands::inbox::render::tests
rtk cargo test -p gg-core --all-features commands::inbox
rtk cargo test -p gg-cli --all-features --test integration_tests inbox
```

Update the existing human provider-label tests: because `Command::output`
creates non-TTY streams, they should assert clean final stdout and no static
`Refreshing PR/MR status` stderr line while preserving `PR #N` and `MR !N`.

- [ ] **Step 7: Commit live rendering**

```bash
rtk cargo fmt --all
rtk git add crates/gg-core/src/commands/inbox.rs crates/gg-core/src/commands/inbox/render.rs crates/gg-cli/tests/integration_tests/inbox.rs
rtk git commit -m "feat(inbox): render live refresh progress"
```

---

### Task 6: Document, Validate, and Finish the Feature

**Files:**
- Modify: `README.md:183`
- Modify: `docs/src/commands/inbox.md:1-132`
- Modify: `docs/src/mcp-server.md:66-70`
- Modify: `skills/gg/SKILL.md:45-58`
- Modify: `skills/gg/references/setup-and-inspection.md:20-31`

**Interfaces:**
- Consumes: final CLI, JSON, JSONL, failure, and TTY contracts from Tasks 1-5.
- Produces: human documentation and agent routing that exactly match the
  implemented command.

- [ ] **Step 1: Update the command documentation**

Add these sections to `docs/src/commands/inbox.md`:

- `Live refresh` with stable rows, the `refreshed N/M` counter, final clearing,
  and final-only redirected output.
- `Streaming NDJSON (--jsonl)` with exact start, stack-error, entry,
  entry-error, summary, and fatal-error examples captured from tests.
- `Partial failures` explaining exit zero plus inspection of
  `refresh_failed`/`stack_errors`.
- `--json vs --jsonl` selection guidance.
- `refresh_failed` first in the bucket list.

Update the flags list with:

```text
--jsonl: emit flushed NDJSON events as refreshes complete; conflicts with --json
```

- [ ] **Step 2: Update README and MCP documentation**

Change the README command-table description to mention progressive refresh and
streaming output without turning the table into a flag catalog.

In `docs/src/mcp-server.md`, state that `stack_inbox` remains atomic and may
return `refresh_failed` entries with `refresh_error`; MCP callers inspect those
fields even when the tool call itself succeeds.

- [ ] **Step 3: Update the agent skill**

In `skills/gg/SKILL.md`, replace:

```text
Use JSON for decisions and JSONL for streaming sync.
```

with:

```text
Use JSON for final structured snapshots and JSONL when a supported long-running command must be monitored incrementally.
```

In `setup-and-inspection.md`, make step 5 explicit:

```text
Use `gg inbox --json` for one final cross-stack snapshot. Use
`gg inbox --jsonl` when the caller benefits from results as each PR or MR
refresh completes, and consume the final `summary` event before making a
complete-inbox claim.
```

Do not copy the event schema into the skill.

- [ ] **Step 4: Run focused documentation and contract validation**

Run:

```bash
rtk mdbook build docs
rtk skills-ref validate skills/gg
rtk cargo test -p gg-cli --all-features --test skill_contract
```

Expected: mdBook builds, skill references validate, and the structural skill
contract passes.

- [ ] **Step 5: Run formatting and inspect the diff**

Run:

```bash
rtk cargo fmt --all
rtk git diff --check
rtk git status --short
rtk git diff --stat
```

Expected: no formatting or whitespace failures. Only intended inbox,
provider, output, CLI, test, documentation, and skill files are modified.
Do not stage `.superpowers/` visual-companion artifacts.

- [ ] **Step 6: Run warning-free Clippy**

Run:

```bash
rtk cargo clippy --all-targets --all-features -- -D warnings
```

Expected: exit zero with no warnings.

- [ ] **Step 7: Run the complete test suite**

Run:

```bash
rtk cargo test --all-features
```

Expected: all workspace tests pass.

- [ ] **Step 8: Commit documentation and any final formatting**

Review `rtk git diff` before staging, then:

```bash
rtk git add README.md docs/src/commands/inbox.md docs/src/mcp-server.md skills/gg/SKILL.md skills/gg/references/setup-and-inspection.md
rtk git commit -m "docs(inbox): document progressive refresh"
```

`cargo fmt` should be a no-op because every Rust task formats before its commit.
If it is not, stop and repair the task that left an unformatted Rust file
before creating the documentation commit.

- [ ] **Step 9: Verify the final committed tree**

Run:

```bash
rtk git status --short
rtk git log --oneline --decorate -8
rtk git diff origin/main...HEAD --check
```

Expected: only `.superpowers/` companion artifacts may remain untracked; all
feature and documentation changes are committed.

---

## Completion Criteria

The implementation is complete only when all of the following are true:

- Human TTY mode renders stable rows before provider refresh completes.
- Human redirected output contains only the final grouped inbox.
- GitHub performs one snapshot request per candidate.
- GitLab performs one details request and one approval request per candidate.
- No more than four candidates refresh concurrently.
- `--json` remains atomic and deterministic.
- `--jsonl` emits completion-order candidate events and a deterministic final
  summary.
- Partial and all-entry refresh failures exit zero and remain visible.
- Fatal discovery/provider-preflight failures exit nonzero with the correct
  structured error envelope.
- MCP remains on atomic JSON.
- Focused tests, mdBook, skill validation, format, Clippy, and all-feature tests
  pass.
