use super::{
    checked_git, operation_record, operation_record_ids, run_gg_bounded_with_env,
    write_remote_prune_config,
};
use crate::helpers::{create_test_repo_with_remote, ProcessGroupGuard};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const IDS: [&str; 3] = ["c-1111111", "c-2222222", "c-3333333"];

// Read heads from the bare origin, not the working copy: a successful push must
// change the SHA reported by the forge. A target edit takes effect on the next view.
const GH: &str = r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
printf '%s\n' "$*" >> "$dir/requests"
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr list')
    for n in 2 3; do
      case "$3" in --head) key=head;; --base) key=base;; *) exit 1;; esac
      [ "$(cat "$dir/$key$n")" != "$4" ] || echo "$n"
    done;;
  'pr edit')
    printf '%s\n' "$*" >> "$dir/mutations"
    if [ "$4" = --base ]; then
      case "${EVIDENCE:-}" in
        block-retarget|ambiguous-retarget)
          attempts=$(cat "$dir/retarget-attempts" 2>/dev/null || echo 0)
          echo $((attempts + 1)) > "$dir/retarget-attempts";;
      esac
      pending=
      for record in "$TEST_GIT_DIR"/gg/operations/*.json; do
        grep -q '"status": "pending"' "$record" || continue
        [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
        pending=$(basename "$record" .json)
      done
      [ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
      printf '%s\n' "$pending" > "$dir/retarget-operation-id"
      if [ "${EVIDENCE:-}" = block-retarget ]; then
        touch "$dir/retarget-started"
        while [ ! -f "$dir/release-retarget" ]; do sleep 0.01; done
      fi
      echo "edit $3 $5" >> "$dir/events"
      [ "${EVIDENCE:-}" = stale-target ] || echo "$5" > "$dir/base$3"
      if [ "${EVIDENCE:-}" = ambiguous-retarget ]; then
        echo 'simulated lost retarget response' >&2
        exit 1
      fi
    elif [ "$4" = --title ] && [ "${EVIDENCE:-}" = block-title ]; then
      attempts=$(cat "$dir/title-attempts" 2>/dev/null || echo 0)
      echo $((attempts + 1)) > "$dir/title-attempts"
      pending=
      for record in "$TEST_GIT_DIR"/gg/operations/*.json; do
        grep -q '"status": "pending"' "$record" || continue
        [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
        pending=$(basename "$record" .json)
      done
      [ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
      printf '%s\n' "$pending" > "$dir/title-operation-id"
      touch "$dir/title-started"
      while [ ! -f "$dir/release-title" ]; do sleep 0.01; done
    elif [ "$4" = --body ] && [ "${EVIDENCE:-}" = block-description ]; then
      attempts=$(cat "$dir/description-attempts" 2>/dev/null || echo 0)
      echo $((attempts + 1)) > "$dir/description-attempts"
      pending=
      for record in "$TEST_GIT_DIR"/gg/operations/*.json; do
        grep -q '"status": "pending"' "$record" || continue
        [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
        pending=$(basename "$record" .json)
      done
      [ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
      printf '%s\n' "$pending" > "$dir/description-operation-id"
      touch "$dir/description-started"
      while [ ! -f "$dir/release-description" ]; do sleep 0.01; done
    fi;;
  'pr view')
    n=$3
    view_count_file="$dir/view$n"
    view_count=$(cat "$view_count_file" 2>/dev/null || echo 0)
    view_count=$((view_count + 1))
    printf '%s\n' "$view_count" > "$view_count_file"
    if [ "${4:-}" = --json ] && [ "${5:-}" = body ]; then
      printf '%s\n' '<!-- gg:managed:start -->' 'old body' '<!-- gg:managed:end -->'
      exit 0
    fi
    head=$(cat "$dir/head$n"); base=$(cat "$dir/base$n"); title="Entry $n"
    if [ "$n" = 2 ] && [ "${EVIDENCE:-}" = block-title ]; then title='Old title'; fi
    if [ "$n" = 2 ] && [ "${EVIDENCE:-}" = fail-main-lookup ] && [ "$base" = main ]; then
      echo 'simulated authoritative lookup failure' >&2
      exit 1
    fi
    oid=$(git --git-dir="$TEST_ORIGIN" rev-parse "refs/heads/$head")
    state=OPEN; mergeable=MERGEABLE; merged=null
    if [ "$n" = 1 ] && [ -f "$dir/landed" ]; then
      state=MERGED
      merged=$(printf '{"oid":"%s"}' "$(cat "$dir/landed")")
    fi
    if [ "$n" = 2 ]; then
      if [ "$base" = main ]; then
        mergeable=${ON_MAIN:-MERGEABLE}
      elif [ "${ON_MAIN:-MERGEABLE}" = MERGEABLE ]; then
        mergeable=CONFLICTING
      fi
      case "${EVIDENCE:-}" in
        wrong-head) head=$(cat "$dir/head3");;
        wrong-sha) oid=$(git --git-dir="$TEST_ORIGIN" rev-parse main);;
      esac
    fi
    case "${EVIDENCE:-}" in
      closed-mapped-review|terminal-closed-expected|terminal-closed-unexpected|consecutive-terminal-reviews) state=CLOSED;;
      terminal-predecessor) [ "$n" != 1 ] || state=CLOSED;;
      terminal-merged-expected|terminal-merged-unexpected) state=MERGED;;
      stale-before-push|stale-before-retarget|stale-before-title)
        [ "$n" != 1 ] || [ "$view_count" -lt 2 ] || state=CLOSED;;
      stale-before-description)
        [ "$n" != 1 ] || [ "$view_count" -lt 3 ] || state=CLOSED;;
    esac
    case "${EVIDENCE:-}" in
      terminal-closed-unexpected|terminal-merged-unexpected) base=unexpected-target;;
    esac
    echo "view $n $base $mergeable" >> "$dir/events"
    case "$mergeable" in MERGEABLE) detail=CLEAN;; CONFLICTING) detail=DIRTY;; *) detail=UNKNOWN;; esac
    printf '{"number":%s,"title":"%s","state":"%s","url":"https://github.com/test/repo/pull/%s","headRefName":"%s","headRefOid":"%s","baseRefName":"%s","mergeCommit":%s,"isDraft":false,"mergeable":"%s","mergeStateStatus":"%s","reviews":[],"statusCheckRollup":[]}\n' "$n" "$title" "$state" "$n" "$head" "$oid" "$base" "$merged" "$mergeable" "$detail";;
  *) echo "unexpected gh $*" >&2; exit 1;;
esac
"#;

fn executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn branch(index: usize) -> String {
    format!("testuser/keep--{}", IDS[index])
}

struct Stack {
    _temp: TempDir,
    repo: PathBuf,
    remote: PathBuf,
    bin: PathBuf,
    path: OsString,
    base: String,
    commits: Vec<String>,
}

impl Stack {
    fn landed(rewritten: bool, forget_mapping: bool) -> Self {
        let (temp, repo, remote) = create_test_repo_with_remote();
        let base = checked_git(&repo, &["rev-parse", "main"]);
        write_remote_prune_config(&repo);
        let bin = repo.join(".git/fake-bin");
        fs::create_dir(&bin).unwrap();
        executable(&bin.join("gh"), GH);
        let mut paths = vec![bin.clone()];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let path = std::env::join_paths(paths).unwrap();
        let config_path = repo.join(".git/gg/config.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        config["defaults"]["sync_update_descriptions"] = json!(false);
        config["defaults"]["lint"] =
            json!(["git rev-parse HEAD >> \"$(git rev-parse --absolute-git-dir)/lint-ran\""]);
        config["stacks"]["keep"] = json!({"base":"main", "mrs":{}});
        checked_git(&repo, &["checkout", "-b", "testuser/keep"]);
        let mut commits = Vec::new();
        for (i, id) in IDS.iter().enumerate() {
            let n = i + 1;
            let mut message = format!("Entry {n}\n\nGG-ID: {id}");
            let target = if i == 0 {
                "main".to_string()
            } else {
                message.push_str(&format!("\nGG-Parent: {}", IDS[i - 1]));
                branch(i - 1)
            };
            fs::write(repo.join(id), id).unwrap();
            checked_git(&repo, &["add", id]);
            checked_git(&repo, &["commit", "-m", &message]);
            commits.push(checked_git(&repo, &["rev-parse", "HEAD"]));
            let head = branch(i);
            checked_git(
                &repo,
                &["push", "origin", &format!("HEAD:refs/heads/{head}")],
            );
            fs::write(bin.join(format!("head{n}")), head).unwrap();
            fs::write(bin.join(format!("base{n}")), target).unwrap();
            if i != 0 || !forget_mapping {
                config["stacks"]["keep"]["mrs"][*id] = json!(n);
            }
        }
        fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        checked_git(&repo, &["checkout", "--detach", &commits[0]]);
        if rewritten {
            checked_git(
                &repo,
                &["commit", "--amend", "-m", "Squashed first entry (#1)"],
            );
        }
        fs::write(
            bin.join("landed"),
            checked_git(&repo, &["rev-parse", "HEAD"]),
        )
        .unwrap();
        fs::write(repo.join("upstream"), "independent upstream change").unwrap();
        checked_git(&repo, &["add", "upstream"]);
        checked_git(&repo, &["commit", "-m", "Advance main independently"]);
        // Bypass the named remote so origin/main stays stale until sync fetches.
        checked_git(
            &repo,
            &["push", remote.to_str().unwrap(), "HEAD:refs/heads/main"],
        );
        checked_git(&repo, &["checkout", "testuser/keep"]);
        checked_git(&repo, &["worktree", "add", "../base-checkout", "main"]);
        executable(
            &repo.join(".git/hooks/pre-push"),
            "#!/bin/sh\necho push >> \"$(git rev-parse --absolute-git-dir)/pushes\"\n",
        );
        Self {
            _temp: temp,
            repo,
            remote,
            bin,
            path,
            base,
            commits,
        }
    }

    fn gg(&self, args: &[&str], on_main: &str, evidence: &str) -> (bool, String, String) {
        let git_dir = self.repo.join(".git");
        run_gg_bounded_with_env(
            &self.repo,
            args,
            &[
                ("PATH", self.path.as_os_str()),
                ("TEST_ORIGIN", self.remote.as_os_str()),
                ("TEST_GIT_DIR", git_dir.as_os_str()),
                ("ON_MAIN", on_main.as_ref()),
                ("EVIDENCE", evidence.as_ref()),
            ],
        )
    }

    fn json(&self, args: &[&str], on_main: &str) -> Value {
        let (ok, out, err) = self.gg(args, on_main, "");
        assert!(ok, "{args:?}: {out}\n{err}");
        serde_json::from_str(&out).unwrap()
    }

    fn assert_base_untouched(&self) {
        let checkout = self.repo.join("../base-checkout");
        assert_eq!(checked_git(&checkout, &["rev-parse", "HEAD"]), self.base);
        assert_eq!(
            checked_git(&checkout, &["branch", "--show-current"]),
            "main"
        );
        assert_eq!(checked_git(&checkout, &["status", "--porcelain"]), "");
    }

    fn assert_untouched(&self) {
        assert_eq!(
            checked_git(&self.repo, &["rev-parse", "HEAD"]),
            self.commits[2]
        );
        assert_eq!(
            checked_git(&self.repo, &["branch", "--show-current"]),
            "testuser/keep"
        );
        self.assert_base_untouched();
        assert_eq!(checked_git(&self.repo, &["status", "--porcelain"]), "");
        for (i, sha) in self.commits.iter().enumerate() {
            assert_eq!(checked_git(&self.remote, &["rev-parse", &branch(i)]), *sha);
        }
        assert!(!self.repo.join(".git/pushes").exists(), "unexpected push");
    }

    fn assert_retargeted_before_view(&self, verdict: &str) {
        let events = fs::read_to_string(self.bin.join("events")).unwrap();
        let (_, after_edit) = events.split_once("edit 2 main\n").expect(&events);
        assert!(
            after_edit
                .lines()
                .any(|line| line == format!("view 2 main {verdict}")),
            "{events}"
        );
    }

    fn retarget_operation_id(&self) -> String {
        fs::read_to_string(self.bin.join("retarget-operation-id"))
            .expect("retarget must capture its operation id")
            .trim()
            .to_string()
    }

    fn advance_origin_with_survivor_conflict(&self) {
        let checkout = self.repo.join("../base-checkout");
        checked_git(&checkout, &["fetch", "origin"]);
        checked_git(&checkout, &["merge", "--ff-only", "origin/main"]);
        fs::write(checkout.join(IDS[1]), "conflicting upstream version").unwrap();
        checked_git(&checkout, &["add", IDS[1]]);
        checked_git(&checkout, &["commit", "-m", "Conflict with survivor"]);
        checked_git(&checkout, &["push", "origin", "main"]);
        fs::write(self.bin.join("base2"), "main").unwrap();
    }

    fn assert_provider_requests(&self, required_ordered: &[&str], expected_mutations: &[&str]) {
        let requests = fs::read_to_string(self.bin.join("requests")).unwrap();
        let requests = requests
            .lines()
            .map(|request| {
                if request.starts_with("pr view ") {
                    request
                        .split_whitespace()
                        .take(3)
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    request.to_string()
                }
            })
            .collect::<Vec<_>>();
        let mut required = required_ordered.iter();
        let mut next = required.next();
        for request in &requests {
            if next.is_some_and(|required| request == required) {
                next = required.next();
            }
        }
        assert!(
            next.is_none(),
            "required provider request order {required_ordered:?} was not present in {requests:?}"
        );
        let mutations = requests
            .iter()
            .filter(|request| {
                request.as_str() != "--version"
                    && request.as_str() != "auth status"
                    && !request.starts_with("pr view ")
                    && !request.starts_with("pr list ")
            })
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            mutations, expected_mutations,
            "unexpected provider mutation sequence in {requests:?}"
        );
    }
}

#[test]
fn preserves_published_shas_with_lint_and_initial_until_position() {
    for forget_mapping in [false, true] {
        let stack = Stack::landed(false, forget_mapping);
        let result = stack.json(
            &[
                "sync",
                "--rebase-if-needed",
                "--lint",
                "--until",
                "2",
                "--json",
            ],
            "MERGEABLE",
        );
        stack.assert_retargeted_before_view("MERGEABLE");
        stack.assert_untouched();
        assert_eq!(result["sync"]["rebased_before_sync"], false);
        let entries = result["sync"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["gg_id"], IDS[1]);
        assert_eq!(entries[0]["pushed"], false);
        let lint = fs::read_to_string(stack.repo.join(".git/lint-ran")).unwrap();
        assert_eq!(lint.trim(), stack.commits[1]);
        let listing = stack.json(&["ls", "--json"], "MERGEABLE");
        let active = listing["stack"]["entries"].as_array().unwrap();
        assert_eq!(active.len(), 2);
        for (entry, i) in active.iter().zip(1..3) {
            assert_eq!(entry["gg_id"], IDS[i]);
            assert!(stack.commits[i].starts_with(entry["sha"].as_str().unwrap()));
            assert_eq!(entry["pr_number"], i + 1);
        }
    }
}

fn assert_fallback(rewritten: bool, verdict: &str) {
    let stack = Stack::landed(rewritten, false);
    let result = stack.json(
        &["sync", "--rebase-if-needed", "--until", "3", "--json"],
        verdict,
    );
    assert_eq!(result["sync"]["rebased_before_sync"], true);
    assert_eq!(
        checked_git(&stack.repo, &["rev-parse", "HEAD~2"]),
        checked_git(&stack.remote, &["rev-parse", "main"])
    );
    stack.assert_base_untouched();
    for (i, &id) in IDS.iter().enumerate().skip(1) {
        let new_sha = checked_git(
            &stack.repo,
            &["rev-parse", if i == 1 { "HEAD~1" } else { "HEAD" }],
        );
        assert_ne!(new_sha, stack.commits[i]);
        assert_eq!(
            checked_git(&stack.remote, &["rev-parse", &branch(i)]),
            new_sha
        );
        assert_eq!(fs::read_to_string(stack.repo.join(id)).unwrap(), id);
    }
    if !rewritten {
        stack.assert_retargeted_before_view("CONFLICTING");
    }
}

#[test]
fn rebases_when_landed_parent_was_rewritten() {
    assert_fallback(true, "MERGEABLE");
}

#[test]
fn rebases_when_new_target_is_conflicting() {
    assert_fallback(false, "CONFLICTING");
}

#[test]
fn refuses_unknown_mergeability_or_mismatched_evidence_without_rewriting() {
    for (verdict, evidence) in [
        ("UNKNOWN", ""),
        ("MERGEABLE", "wrong-head"),
        ("MERGEABLE", "wrong-sha"),
        ("MERGEABLE", "stale-target"),
    ] {
        let stack = Stack::landed(false, false);
        let (ok, out, err) = stack.gg(
            &["sync", "--rebase-if-needed", "--until", "3"],
            verdict,
            evidence,
        );
        assert!(!ok, "accepted {verdict}/{evidence}: {out}\n{err}");
        stack.assert_untouched();
    }
}

#[test]
fn existing_review_lookup_failure_stops_before_main_loop_mutation() {
    let stack = Stack::landed(false, false);
    let (ok, out, err) = stack.gg(
        &["sync", "--rebase-if-needed", "--until", "2"],
        "MERGEABLE",
        "fail-main-lookup",
    );

    assert!(!ok, "lookup failure must stop sync: {out}\n{err}");
    assert!(
        err.contains("simulated authoritative lookup failure"),
        "{out}\n{err}"
    );
    let events = fs::read_to_string(stack.bin.join("events")).unwrap();
    assert_eq!(
        events.lines().filter(|line| *line == "edit 2 main").count(),
        1,
        "the preparatory retarget should complete before classification: {events}"
    );
    assert!(
        !stack.repo.join(".git/pushes").exists(),
        "main-loop push ran after lookup failure"
    );
    assert!(
        !events.lines().any(|line| line == "edit 3 main"),
        "main-loop review update ran after lookup failure: {events}"
    );
}

#[test]
fn changed_entry_mapped_to_closed_review_is_classified_before_push() {
    let stack = Stack::landed(false, false);
    fs::remove_file(stack.bin.join("landed")).unwrap();
    checked_git(
        &stack.repo,
        &[
            "push",
            "--no-verify",
            stack.remote.to_str().unwrap(),
            &format!("+{}:refs/heads/main", stack.base),
        ],
    );
    let config_path = stack.repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["sync_auto_rebase"] = json!(false);
    fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    checked_git(&stack.repo, &["fetch", "origin"]);
    checked_git(&stack.repo, &["reset", "--hard", &stack.commits[0]]);

    fs::write(stack.repo.join(IDS[0]), "changed local content").unwrap();
    checked_git(&stack.repo, &["add", IDS[0]]);
    checked_git(&stack.repo, &["commit", "--amend", "--no-edit"]);

    let (ok, out, err) = stack.gg(
        &["sync", "--rebase-if-needed", "--until", "1", "--json"],
        "MERGEABLE",
        "closed-mapped-review",
    );

    assert!(ok, "closed mapped review must be skipped: {out}\n{err}");
    let payload: Value = serde_json::from_str(&out).expect("valid JSON response");
    let entries = payload["sync"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{payload}");
    assert_eq!(entries[0]["pr_number"], 1, "{payload}");
    assert_eq!(entries[0]["action"], "skipped_closed", "{payload}");
    assert_eq!(entries[0]["pushed"], false, "{payload}");
    assert!(
        !stack.repo.join(".git/pushes").exists(),
        "mapped review classification must precede push"
    );
    assert_eq!(
        checked_git(&stack.remote, &["rev-parse", &branch(0)]),
        stack.commits[0]
    );
}

#[test]
fn terminal_reviews_require_the_expected_target_before_skip() {
    for (state, target_matches) in [
        ("closed", true),
        ("closed", false),
        ("merged", true),
        ("merged", false),
    ] {
        let stack = Stack::landed(false, false);
        fs::remove_file(stack.bin.join("landed")).unwrap();
        checked_git(&stack.repo, &["reset", "--hard", &stack.commits[0]]);
        let evidence = format!(
            "terminal-{state}-{}",
            if target_matches {
                "expected"
            } else {
                "unexpected"
            }
        );

        let (ok, out, err) = stack.gg(
            &["sync", "--until", "1", "--no-rebase-check", "--json"],
            "MERGEABLE",
            &evidence,
        );

        if target_matches {
            assert!(ok, "{state}: {out}\n{err}");
            let payload: Value = serde_json::from_str(&out).unwrap();
            assert_eq!(payload["sync"]["entries"][0]["action"], "skipped_closed");
        } else {
            assert!(!ok, "{state}: unexpected target was skipped: {out}\n{err}");
            assert!(
                format!("{out}{err}").contains("does not have the expected source and target"),
                "{state}: {out}\n{err}"
            );
        }
        assert!(!stack.repo.join(".git/pushes").exists(), "{state}");
        assert!(!stack.bin.join("mutations").exists(), "{state}");
    }
}

#[test]
fn consecutive_terminal_reviews_keep_their_published_predecessor_targets() {
    let stack = Stack::landed(false, false);
    fs::remove_file(stack.bin.join("landed")).unwrap();

    let (ok, out, err) = stack.gg(
        &["sync", "--until", "3", "--no-rebase-check", "--json"],
        "MERGEABLE",
        "consecutive-terminal-reviews",
    );

    assert!(ok, "published terminal topology was rejected: {out}\n{err}");
    let payload: Value = serde_json::from_str(&out).unwrap();
    let entries = payload["sync"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3, "{payload}");
    assert!(
        entries
            .iter()
            .all(|entry| entry["action"] == "skipped_closed"),
        "{payload}"
    );
    assert!(!stack.repo.join(".git/pushes").exists());
    assert!(!stack.bin.join("mutations").exists());
}

#[test]
fn active_successor_of_terminal_review_is_retargeted_from_published_topology() {
    let stack = Stack::landed(false, false);
    fs::remove_file(stack.bin.join("landed")).unwrap();
    checked_git(
        &stack.repo,
        &[
            "push",
            "--no-verify",
            stack.remote.to_str().unwrap(),
            &format!("+{}:refs/heads/main", stack.base),
        ],
    );

    let args = ["sync", "--rebase-if-needed", "--until", "2", "--json"];
    let (ok, out, err) = stack.gg(&args, "MERGEABLE", "terminal-predecessor");

    assert!(
        ok,
        "active successor was rejected before retarget: {out}\n{err}"
    );
    let payload: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(payload["sync"]["entries"][0]["action"], "skipped_closed");
    assert_eq!(
        fs::read_to_string(stack.bin.join("base2")).unwrap(),
        "main\n"
    );

    let (ok, out, err) = stack.gg(&args, "MERGEABLE", "terminal-predecessor");

    assert!(
        ok,
        "already-reconciled successor was rejected on repeated sync: {out}\n{err}"
    );
    let payload: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(payload["sync"]["entries"][0]["action"], "skipped_closed");
    assert_eq!(
        fs::read_to_string(stack.bin.join("base2")).unwrap(),
        "main\n"
    );

    let (ok, out, err) = stack.gg(&args, "MERGEABLE", "consecutive-terminal-reviews");

    assert!(
        ok,
        "terminal successor on the desired walk-back target was rejected: {out}\n{err}"
    );
    let payload: Value = serde_json::from_str(&out).unwrap();
    let entries = payload["sync"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "{payload}");
    assert!(
        entries
            .iter()
            .all(|entry| entry["action"] == "skipped_closed"),
        "{payload}"
    );
    stack.assert_provider_requests(
        &[
            "pr view 1",
            "pr view 2",
            "pr view 2",
            "pr edit 2 --base main",
            "pr view 1",
            "pr view 2",
            "pr view 1",
            "pr view 2",
        ],
        &["pr edit 2 --base main"],
    );
}

#[test]
fn ordinary_sync_revalidates_review_before_push() {
    let stack = Stack::landed(false, false);
    fs::remove_file(stack.bin.join("landed")).unwrap();
    checked_git(&stack.repo, &["reset", "--hard", &stack.commits[0]]);
    fs::write(stack.repo.join(IDS[0]), "changed local content").unwrap();
    checked_git(&stack.repo, &["add", IDS[0]]);
    checked_git(&stack.repo, &["commit", "--amend", "--no-edit"]);

    let (ok, out, err) = stack.gg(
        &["sync", "--until", "1", "--no-rebase-check"],
        "MERGEABLE",
        "stale-before-push",
    );

    assert!(!ok, "stale mapped review allowed a push: {out}\n{err}");
    assert!(
        format!("{out}{err}").contains("not open at the expected source and target"),
        "{out}\n{err}"
    );
    assert_eq!(
        fs::read_to_string(stack.bin.join("view1")).unwrap().trim(),
        "2"
    );
    assert!(
        !stack.repo.join(".git/pushes").exists(),
        "push was attempted after stale authorization"
    );
}

#[test]
fn ordinary_sync_revalidates_before_review_updates() {
    for (name, extra_args, expected_views) in [
        ("retarget", &[][..], "2"),
        ("title", &["--update-title"][..], "2"),
        ("description", &["--update-descriptions"][..], "4"),
    ] {
        let stack = Stack::landed(false, false);
        fs::remove_file(stack.bin.join("landed")).unwrap();
        checked_git(&stack.repo, &["reset", "--hard", &stack.commits[0]]);
        let mut args = vec!["sync", "--until", "1", "--no-rebase-check"];
        args.extend_from_slice(extra_args);
        let evidence = format!("stale-before-{name}");

        let (ok, out, err) = stack.gg(&args, "MERGEABLE", &evidence);

        assert!(!ok, "{name}: stale review allowed mutation: {out}\n{err}");
        assert!(
            format!("{out}{err}").contains("not open at the expected source and target"),
            "{name}: {out}\n{err}"
        );
        assert_eq!(
            fs::read_to_string(stack.bin.join("view1")).unwrap().trim(),
            expected_views,
            "{name}: unexpected authoritative read count"
        );
        assert!(
            !stack.bin.join("mutations").exists(),
            "{name}: mutation attempted after failed authorization"
        );
    }
}

#[test]
fn sync_rebase_conflict_continue_finalizes_exact_pending_operation() {
    let stack = Stack::landed(false, false);
    stack.advance_origin_with_survivor_conflict();
    let operations_before = operation_record_ids(&stack.repo);

    let (ok, out, err) = stack.gg(
        &["sync", "--rebase-if-needed", "--until", "2", "--no-verify"],
        "CONFLICTING",
        "",
    );

    assert!(!ok, "conflicting sync unexpectedly succeeded: {out}\n{err}");
    assert!(format!("{out}{err}").to_lowercase().contains("conflict"));
    let new_ids = operation_record_ids(&stack.repo)
        .difference(&operations_before)
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(new_ids.len(), 1, "conflict must use one sync operation");
    let operation_id = new_ids.into_iter().next().unwrap();
    let record = operation_record(&stack.repo, &operation_id);
    assert_eq!(record["kind"], "sync", "{record}");
    assert_eq!(record["status"], "pending", "{record}");
    let marker_path = stack.repo.join(".git/gg/interrupted-rebase-operation.json");
    let marker: Value = serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
    assert_eq!(marker["operation_id"], operation_id);

    fs::write(stack.repo.join(IDS[1]), IDS[1]).unwrap();
    checked_git(&stack.repo, &["add", IDS[1]]);
    checked_git(&stack.repo, &["config", "core.editor", "true"]);
    let (ok, out, err) = stack.gg(&["continue"], "CONFLICTING", "");
    assert!(ok, "gg continue failed: {out}\n{err}");

    assert_eq!(
        operation_record_ids(&stack.repo)
            .difference(&operations_before)
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        [operation_id.clone()].into_iter().collect(),
        "continue must not create a second operation"
    );
    let record = operation_record(&stack.repo, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert!(!marker_path.exists(), "continue must clear the marker");
}

#[test]
fn sync_rebase_conflict_abort_clears_exact_marker_and_restores_stack() {
    let stack = Stack::landed(false, false);
    stack.advance_origin_with_survivor_conflict();
    let operations_before = operation_record_ids(&stack.repo);

    let (ok, out, err) = stack.gg(
        &["sync", "--rebase-if-needed", "--until", "2", "--no-verify"],
        "CONFLICTING",
        "",
    );
    assert!(!ok, "conflicting sync unexpectedly succeeded: {out}\n{err}");

    let new_ids = operation_record_ids(&stack.repo)
        .difference(&operations_before)
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(new_ids.len(), 1, "conflict must use one sync operation");
    let operation_id = new_ids.into_iter().next().unwrap();
    let marker_path = stack.repo.join(".git/gg/interrupted-rebase-operation.json");
    let marker: Value = serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
    assert_eq!(marker["operation_id"], operation_id);

    let (ok, out, err) = stack.gg(&["abort"], "CONFLICTING", "");
    assert!(ok, "gg abort failed: {out}\n{err}");

    assert!(!marker_path.exists(), "abort must clear the marker");
    assert!(!stack.repo.join(".git/rebase-merge").exists());
    assert!(!stack.repo.join(".git/rebase-apply").exists());
    assert_eq!(
        checked_git(&stack.repo, &["rev-parse", "HEAD"]),
        stack.commits[2],
        "abort must restore the pre-rebase stack tip"
    );
    assert_eq!(
        operation_record_ids(&stack.repo)
            .difference(&operations_before)
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        [operation_id.clone()].into_iter().collect(),
        "abort must not create or discard a different operation"
    );
    let record = operation_record(&stack.repo, &operation_id);
    assert_eq!(record["kind"], "sync", "{record}");
    assert_eq!(
        record["status"], "pending",
        "the existing abort contract leaves the rolled-back record for stale-pending recovery"
    );
}

#[test]
fn sync_rebase_conflict_surfaces_marker_failure_with_conflict_context() {
    let stack = Stack::landed(false, false);
    stack.advance_origin_with_survivor_conflict();
    let operations_before = operation_record_ids(&stack.repo);
    let marker_tmp = stack
        .repo
        .join(".git/gg/interrupted-rebase-operation.json.tmp");
    fs::create_dir(&marker_tmp).unwrap();

    let (ok, out, err) = stack.gg(
        &["sync", "--rebase-if-needed", "--until", "2", "--no-verify"],
        "CONFLICTING",
        "",
    );

    assert!(!ok, "conflicting sync unexpectedly succeeded: {out}\n{err}");
    let error = format!("{out}{err}");
    assert!(error.to_lowercase().contains("conflict"), "{error}");
    assert!(
        error.contains("failed to remember interrupted sync operation"),
        "{error}"
    );
    let new_ids = operation_record_ids(&stack.repo)
        .difference(&operations_before)
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        new_ids.len(),
        1,
        "marker failure must not duplicate the operation"
    );
    let operation_id = new_ids.into_iter().next().unwrap();
    let record = operation_record(&stack.repo, &operation_id);
    assert_eq!(record["kind"], "sync", "{record}");
    assert_eq!(record["status"], "pending", "{record}");
    assert!(
        !stack
            .repo
            .join(".git/gg/interrupted-rebase-operation.json")
            .exists(),
        "failed marker write must not fabricate a usable marker"
    );

    fs::remove_dir(marker_tmp).unwrap();
    checked_git(&stack.repo, &["rebase", "--abort"]);
}

#[test]
fn jsonl_disconnect_after_retarget_finalizes_exact_segment_before_second_lint() {
    let stack = Stack::landed(false, false);
    let lint = stack.repo.join("checkpoint-lint.sh");
    executable(
        &lint,
        "#!/bin/sh\nset -eu\nprintf 'lint\\n' >> \"$(git rev-parse --absolute-git-dir)/checkpoint-lint-runs\"\n",
    );
    let config_path = stack.repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["lint"] = json!(["./checkpoint-lint.sh"]);
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let (read_stream, write_stream) = UnixStream::pair().unwrap();
    read_stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let stdout_fd: std::os::fd::OwnedFd = write_stream.into();
    let stderr_path = stack.repo.join(".git/retarget-disconnect-stderr");
    let mut command = Command::new(env!("CARGO_BIN_EXE_gg"));
    command
        .args([
            "sync",
            "--rebase-if-needed",
            "--lint",
            "--until",
            "2",
            "--jsonl",
        ])
        .current_dir(&stack.repo)
        .env("HOME", stack.repo.join(".git/test-home"))
        .env("PATH", &stack.path)
        .env("TEST_ORIGIN", &stack.remote)
        .env("TEST_GIT_DIR", stack.repo.join(".git"))
        .env("ON_MAIN", "MERGEABLE")
        .env("EVIDENCE", "block-retarget")
        .stdout(Stdio::from(stdout_fd))
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
    let mut child = ProcessGroupGuard::spawn(&mut command).expect("start sync");
    let mut reader = BufReader::new(read_stream);
    let mut first = String::new();
    reader.read_line(&mut first).expect("read start event");
    let first: Value = serde_json::from_str(&first).expect("valid start event");
    assert_eq!(first["event"], "start");

    let readiness_deadline = Instant::now() + Duration::from_secs(10);
    while !stack.bin.join("retarget-started").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "sync exited before blocked retarget ({status}): {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        if Instant::now() >= readiness_deadline {
            let _ = fs::write(stack.bin.join("release-retarget"), "release");
            panic!(
                "sync did not reach blocked retarget: {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let operation_id = stack.retarget_operation_id();
    let pending = operation_record(&stack.repo, &operation_id);
    assert_eq!(pending["status"], "pending", "{pending}");
    assert_eq!(pending["touched_remote"], true, "{pending}");
    reader
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    loop {
        let mut buffered = String::new();
        match reader.read_line(&mut buffered) {
            Ok(0) => panic!("sync closed while retarget was blocked"),
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(error) => panic!("drain pre-retarget JSONL: {error}"),
        }
    }

    fs::write(stack.bin.join("release-retarget"), "release").unwrap();
    reader
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut checkpoint_line = String::new();
    reader
        .read_line(&mut checkpoint_line)
        .expect("read retarget checkpoint event");
    let checkpoint_event: Value =
        serde_json::from_str(&checkpoint_line).expect("valid retarget checkpoint event");
    drop(reader);
    let process_deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll sync") {
            break status;
        }
        if Instant::now() >= process_deadline {
            panic!(
                "sync did not stop after disconnect: {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(!status.success(), "closed JSONL consumer must abort sync");
    assert_eq!(
        fs::read_to_string(stack.bin.join("retarget-attempts"))
            .unwrap()
            .trim(),
        "1"
    );
    let record = operation_record(&stack.repo, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
    assert_eq!(
        checkpoint_event["event"], "pr_updated",
        "{checkpoint_event}"
    );
    assert_eq!(
        checkpoint_event["action"], "retargeted",
        "{checkpoint_event}"
    );
    assert_eq!(
        fs::read_to_string(stack.repo.join(".git/checkpoint-lint-runs"))
            .unwrap()
            .lines()
            .count(),
        1,
        "disconnect after retarget must prevent the second lint phase"
    );
    assert!(!stack.repo.join(".git/pushes").exists(), "unexpected push");
}

#[test]
fn jsonl_disconnect_after_title_update_stops_before_description_update() {
    let stack = Stack::landed(false, false);
    let (read_stream, write_stream) = UnixStream::pair().unwrap();
    read_stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let stdout_fd: std::os::fd::OwnedFd = write_stream.into();
    let stderr_path = stack.repo.join(".git/title-disconnect-stderr");
    let mut command = Command::new(env!("CARGO_BIN_EXE_gg"));
    command
        .args([
            "sync",
            "--rebase-if-needed",
            "--until",
            "2",
            "--update-title",
            "--update-descriptions",
            "--jsonl",
        ])
        .current_dir(&stack.repo)
        .env("HOME", stack.repo.join(".git/test-home"))
        .env("PATH", &stack.path)
        .env("TEST_ORIGIN", &stack.remote)
        .env("TEST_GIT_DIR", stack.repo.join(".git"))
        .env("ON_MAIN", "MERGEABLE")
        .env("EVIDENCE", "block-title")
        .stdout(Stdio::from(stdout_fd))
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
    let mut child = ProcessGroupGuard::spawn(&mut command).expect("start sync");
    let mut reader = BufReader::new(read_stream);
    let readiness_deadline = Instant::now() + Duration::from_secs(10);
    while !stack.bin.join("title-started").exists() {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => panic!("sync closed before title update"),
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("read JSONL before title update: {error}"),
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "sync exited before title update ({status}): {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        if Instant::now() >= readiness_deadline {
            let _ = fs::write(stack.bin.join("release-title"), "release");
            panic!("timed out waiting for title update");
        }
    }
    let operation_id = fs::read_to_string(stack.bin.join("title-operation-id"))
        .expect("title request must capture its operation id")
        .trim()
        .to_string();
    let pending = operation_record(&stack.repo, &operation_id);
    assert_eq!(pending["status"], "pending", "{pending}");
    assert_eq!(pending["touched_remote"], true, "{pending}");

    drop(reader);
    fs::write(stack.bin.join("release-title"), "release").unwrap();
    let process_deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= process_deadline {
            panic!("sync did not stop after title disconnect");
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(!status.success(), "closed JSONL consumer must abort sync");
    assert_eq!(
        fs::read_to_string(stack.bin.join("title-attempts"))
            .unwrap()
            .trim(),
        "1"
    );
    let mutations = fs::read_to_string(stack.bin.join("mutations")).unwrap();
    assert!(
        !mutations.contains("--body"),
        "description update followed disconnected title checkpoint: {mutations}"
    );
    let record = operation_record(&stack.repo, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
}

#[test]
fn jsonl_disconnect_after_description_update_stops_before_next_review_mutation() {
    let stack = Stack::landed(false, false);
    let (read_stream, write_stream) = UnixStream::pair().unwrap();
    read_stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let stdout_fd: std::os::fd::OwnedFd = write_stream.into();
    let stderr_path = stack.repo.join(".git/description-disconnect-stderr");
    let mut command = Command::new(env!("CARGO_BIN_EXE_gg"));
    command
        .args([
            "sync",
            "--rebase-if-needed",
            "--update-descriptions",
            "--jsonl",
        ])
        .current_dir(&stack.repo)
        .env("HOME", stack.repo.join(".git/test-home"))
        .env("PATH", &stack.path)
        .env("TEST_ORIGIN", &stack.remote)
        .env("TEST_GIT_DIR", stack.repo.join(".git"))
        .env("ON_MAIN", "MERGEABLE")
        .env("EVIDENCE", "block-description")
        .stdout(Stdio::from(stdout_fd))
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
    let mut child = ProcessGroupGuard::spawn(&mut command).expect("start sync");
    let mut reader = BufReader::new(read_stream);
    let readiness_deadline = Instant::now() + Duration::from_secs(10);
    while !stack.bin.join("description-started").exists() {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => panic!("sync closed before description update"),
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("read JSONL before description update: {error}"),
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "sync exited before description update ({status}): {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        if Instant::now() >= readiness_deadline {
            let _ = fs::write(stack.bin.join("release-description"), "release");
            panic!("timed out waiting for description update");
        }
    }
    let operation_id = fs::read_to_string(stack.bin.join("description-operation-id"))
        .expect("description request must capture its operation id")
        .trim()
        .to_string();
    let pending = operation_record(&stack.repo, &operation_id);
    assert_eq!(pending["status"], "pending", "{pending}");
    assert_eq!(pending["touched_remote"], true, "{pending}");

    drop(reader);
    fs::write(stack.bin.join("release-description"), "release").unwrap();
    let process_deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= process_deadline {
            panic!("sync did not stop after description disconnect");
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(!status.success(), "closed JSONL consumer must abort sync");
    assert_eq!(
        fs::read_to_string(stack.bin.join("description-attempts"))
            .unwrap()
            .trim(),
        "1"
    );
    let mutations = fs::read_to_string(stack.bin.join("mutations")).unwrap();
    assert!(
        !mutations.lines().any(|line| line.starts_with("pr edit 3 ")),
        "a later review mutation followed disconnected description checkpoint: {mutations}"
    );
    let record = operation_record(&stack.repo, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
}

#[test]
fn jsonl_disconnect_during_second_lint_finalizes_retarget_and_rebase_segments() {
    let stack = Stack::landed(false, false);
    executable(
        &stack.repo.join("two-phase-checkpoint-lint.sh"),
        r#"#!/bin/sh
set -eu
dir=$(git rev-parse --absolute-git-dir)
count=$(cat "$dir/lint-count" 2>/dev/null || echo 0)
count=$((count + 1))
echo "$count" > "$dir/lint-count"
pending=''
for record in "$dir/gg/operations/"*.json; do
  grep -q '"status": "pending"' "$record" || continue
  [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
  pending=$(basename "$record" .json)
done
[ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
if [ "$count" = 2 ]; then
  printf '%s\n' "$pending" > "$dir/second-lint-operation-id"
  touch "$dir/fake-bin/second-lint-started"
  while [ ! -f "$dir/fake-bin/release-second-lint" ]; do sleep 0.01; done
fi
"#,
    );
    let config_path = stack.repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["lint"] = json!(["./two-phase-checkpoint-lint.sh"]);
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let (read_stream, write_stream) = UnixStream::pair().unwrap();
    read_stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let stdout_fd: std::os::fd::OwnedFd = write_stream.into();
    let stderr_path = stack.repo.join(".git/second-lint-disconnect-stderr");
    let mut command = Command::new(env!("CARGO_BIN_EXE_gg"));
    command
        .args([
            "sync",
            "--rebase-if-needed",
            "--lint",
            "--until",
            "2",
            "--jsonl",
        ])
        .current_dir(&stack.repo)
        .env("HOME", stack.repo.join(".git/test-home"))
        .env("PATH", &stack.path)
        .env("TEST_ORIGIN", &stack.remote)
        .env("TEST_GIT_DIR", stack.repo.join(".git"))
        .env("ON_MAIN", "CONFLICTING")
        .env("EVIDENCE", "block-retarget")
        .stdout(Stdio::from(stdout_fd))
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
    let mut child = ProcessGroupGuard::spawn(&mut command).expect("start sync");
    let mut reader = BufReader::new(read_stream);
    let mut first = String::new();
    reader.read_line(&mut first).expect("read start event");
    assert_eq!(
        serde_json::from_str::<Value>(&first).unwrap()["event"],
        "start"
    );

    let retarget_deadline = Instant::now() + Duration::from_secs(10);
    while !stack.bin.join("retarget-started").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "sync exited before retarget ({status}): {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        if Instant::now() >= retarget_deadline {
            let _ = fs::write(stack.bin.join("release-retarget"), "release");
            panic!("sync did not reach retarget");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let retarget_id = stack.retarget_operation_id();
    fs::write(stack.bin.join("release-retarget"), "release").unwrap();

    let second_lint_deadline = Instant::now() + Duration::from_secs(10);
    while !stack.bin.join("second-lint-started").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "sync exited before second lint ({status}): {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        if Instant::now() >= second_lint_deadline {
            let _ = fs::write(stack.bin.join("release-second-lint"), "release");
            panic!("sync did not reach second lint");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let second_lint_id = fs::read_to_string(stack.repo.join(".git/second-lint-operation-id"))
        .unwrap()
        .trim()
        .to_string();
    assert_ne!(second_lint_id, retarget_id);

    drop(reader);
    fs::write(stack.bin.join("release-second-lint"), "release").unwrap();
    let process_deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= process_deadline {
            panic!("sync did not stop after second-lint disconnect");
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(!status.success(), "closed JSONL consumer must abort sync");
    assert_eq!(
        fs::read_to_string(stack.bin.join("retarget-attempts"))
            .unwrap()
            .trim(),
        "1"
    );
    assert!(!stack.repo.join(".git/pushes").exists(), "unexpected push");
    for (id, touched_remote) in [(&retarget_id, true), (&second_lint_id, false)] {
        let record = operation_record(&stack.repo, id);
        assert_eq!(record["status"], "committed", "{record}");
        assert_eq!(record["touched_remote"], touched_remote, "{record}");
    }
}

#[test]
fn ambiguous_retarget_response_finalizes_remote_uncertainty_without_retry() {
    let stack = Stack::landed(false, false);

    let (ok, out, err) = stack.gg(
        &["sync", "--rebase-if-needed", "--until", "2", "--json"],
        "MERGEABLE",
        "ambiguous-retarget",
    );

    assert!(
        !ok,
        "ambiguous retarget unexpectedly succeeded: {out}\n{err}"
    );
    assert!(
        format!("{out}{err}").contains("simulated lost retarget response"),
        "{out}\n{err}"
    );
    assert_eq!(
        fs::read_to_string(stack.bin.join("retarget-attempts"))
            .unwrap()
            .trim(),
        "1"
    );
    let operation_id = stack.retarget_operation_id();
    let record = operation_record(&stack.repo, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
}

#[test]
fn completed_retarget_segment_is_finalized_before_later_validation_error() {
    let stack = Stack::landed(false, false);
    let operations_before = operation_record_ids(&stack.repo);

    let (ok, out, err) = stack.gg(
        &["sync", "--rebase-if-needed", "--until", "2", "--no-verify"],
        "MERGEABLE",
        "stale-target",
    );

    assert!(!ok, "stale retarget unexpectedly succeeded: {out}\n{err}");
    assert!(format!("{out}{err}").contains("does not report main as its target"));
    let retarget_id = stack.retarget_operation_id();
    let new_ids = operation_record_ids(&stack.repo)
        .difference(&operations_before)
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(new_ids, [retarget_id.clone()].into_iter().collect());
    let record = operation_record(&stack.repo, &retarget_id);
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], true, "{record}");
    stack.assert_provider_requests(
        &[
            "--version",
            "auth status",
            "pr view 2",
            "pr view 2",
            "pr edit 2 --base main",
            "pr view 2",
        ],
        &["pr edit 2 --base main"],
    );
}

#[test]
fn no_retarget_keeps_rebase_and_lint_in_original_segment() {
    let stack = Stack::landed(false, false);
    fs::write(stack.bin.join("base2"), "main").unwrap();
    let lint = stack.bin.join("lint-with-operation-id");
    executable(
        &lint,
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
pending=
for record in "$TEST_GIT_DIR"/gg/operations/*.json; do
  grep -q '"status": "pending"' "$record" || continue
  [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
  pending=$(basename "$record" .json)
done
[ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
printf '%s\n' "$pending" >> "$dir/lint-operation-ids"
"#,
    );
    let config_path = stack.repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["lint"] = json!([lint.to_string_lossy()]);
    fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let operations_before = operation_record_ids(&stack.repo);

    let (ok, out, err) = stack.gg(
        &[
            "sync",
            "--rebase-if-needed",
            "--until",
            "2",
            "--lint",
            "--no-verify",
        ],
        "CONFLICTING",
        "",
    );

    assert!(ok, "sync failed without retargeting: {out}\n{err}");
    assert!(
        !stack.bin.join("retarget-operation-id").exists(),
        "the already-correct target must not be retargeted"
    );
    let lint_ids = fs::read_to_string(stack.bin.join("lint-operation-ids"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(lint_ids.len(), 2, "one lint request per phase expected");
    assert_eq!(
        lint_ids[1], lint_ids[0],
        "ordinary rebase/lint work must remain in the preparatory segment"
    );
    let operation_id = lint_ids[0].clone();
    let new_ids = operation_record_ids(&stack.repo)
        .difference(&operations_before)
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(new_ids, [operation_id.clone()].into_iter().collect());
    let record = operation_record(&stack.repo, &operation_id);
    assert_eq!(record["status"], "committed", "{record}");
}

#[test]
fn successor_segment_is_finalized_when_following_lint_fails() {
    let stack = Stack::landed(false, false);
    let lint = stack.bin.join("lint-by-segment");
    executable(
        &lint,
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
pending=
for record in "$TEST_GIT_DIR"/gg/operations/*.json; do
  grep -q '"status": "pending"' "$record" || continue
  [ -z "$pending" ] || { echo 'multiple pending operations' >&2; exit 1; }
  pending=$(basename "$record" .json)
done
[ -n "$pending" ] || { echo 'missing pending operation' >&2; exit 1; }
printf '%s\n' "$pending" >> "$dir/lint-operation-ids"
[ "$(wc -l < "$dir/lint-operation-ids")" -lt 2 ]
"#,
    );
    let config_path = stack.repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["defaults"]["lint"] = json!([lint.to_string_lossy()]);
    fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let operations_before = operation_record_ids(&stack.repo);

    let (ok, out, err) = stack.gg(
        &[
            "sync",
            "--rebase-if-needed",
            "--until",
            "2",
            "--lint",
            "--no-verify",
        ],
        "CONFLICTING",
        "",
    );

    assert!(!ok, "second lint unexpectedly succeeded: {out}\n{err}");
    assert!(format!("{out}{err}").contains("Lint failed"));
    let lint_ids = fs::read_to_string(stack.bin.join("lint-operation-ids"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(lint_ids.len(), 2, "one lint request per phase expected");
    let retarget_id = stack.retarget_operation_id();
    assert_eq!(lint_ids[0], retarget_id);
    assert_ne!(lint_ids[1], retarget_id, "later mutation needs a successor");
    let new_ids = operation_record_ids(&stack.repo)
        .difference(&operations_before)
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        new_ids,
        lint_ids.iter().cloned().collect(),
        "every opened sync segment must be captured"
    );
    for id in &lint_ids {
        let record = operation_record(&stack.repo, id);
        assert_eq!(record["status"], "committed", "segment {id}: {record}");
    }
    assert_eq!(
        operation_record(&stack.repo, &retarget_id)["touched_remote"],
        true
    );
    let successor = operation_record(&stack.repo, &lint_ids[1]);
    assert_eq!(successor["touched_remote"], false, "{successor}");
    assert_eq!(successor["remote_effects"], json!([]), "{successor}");
    stack.assert_provider_requests(
        &[
            "--version",
            "auth status",
            "pr view 2",
            "pr view 2",
            "pr edit 2 --base main",
            "pr view 2",
            "pr view 2",
            "pr view 3",
        ],
        &["pr edit 2 --base main"],
    );
}

#[test]
fn refuses_rewritten_parent_if_merge_result_is_not_on_fetched_base() {
    let stack = Stack::landed(true, false);
    fs::write(stack.bin.join("landed"), &stack.commits[2]).unwrap();
    let (ok, out, err) = stack.gg(
        &["sync", "--rebase-if-needed", "--until", "3"],
        "MERGEABLE",
        "",
    );
    assert!(!ok, "accepted unrelated merge result: {out}\n{err}");
    stack.assert_untouched();
}

#[test]
fn already_landed_until_target_does_not_publish_or_lint_its_successor() {
    let stack = Stack::landed(false, true);
    let result = stack.json(
        &[
            "sync",
            "--rebase-if-needed",
            "--until",
            "1",
            "--lint",
            "--json",
        ],
        "MERGEABLE",
    );
    assert!(result["sync"]["entries"].as_array().unwrap().is_empty());
    stack.assert_untouched();
    assert_eq!(
        fs::read_to_string(stack.bin.join("base2")).unwrap(),
        branch(0)
    );
    assert!(!stack.repo.join(".git/lint-ran").exists());
}

#[test]
fn refuses_to_republish_a_patch_equivalent_parent_with_no_mapping() {
    let stack = Stack::landed(true, true);
    let (ok, out, err) = stack.gg(&["sync", "--rebase-if-needed", "--json"], "MERGEABLE", "");
    assert!(!ok, "{out}\n{err}");
    assert!(format!("{out}{err}").contains("no review mapping"));
    stack.assert_untouched();
}

#[test]
fn unrelated_advance_obeys_auto_rebase_without_moving_local_base() {
    for auto_rebase in [true, false] {
        let stack = Stack::landed(false, false);
        fs::remove_file(stack.bin.join("landed")).unwrap();
        let unrelated = checked_git(
            &stack.repo,
            &[
                "commit-tree",
                "main^{tree}",
                "-p",
                &stack.base,
                "-m",
                "Unrelated upstream commit",
            ],
        );
        checked_git(
            &stack.repo,
            &[
                "push",
                "--no-verify",
                stack.remote.to_str().unwrap(),
                &format!("+{unrelated}:refs/heads/main"),
            ],
        );
        let path = stack.repo.join(".git/gg/config.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        config["defaults"]["sync_auto_rebase"] = json!(auto_rebase);
        config["defaults"]["sync_behind_threshold"] = json!(1);
        fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
        let result = stack.json(
            &["sync", "--rebase-if-needed", "--until", "1", "--json"],
            "MERGEABLE",
        );
        assert_eq!(result["sync"]["rebased_before_sync"], auto_rebase);
        assert_eq!(checked_git(&stack.repo, &["rev-parse", "main"]), stack.base);
        assert_eq!(
            checked_git(&stack.repo, &["rev-parse", "HEAD~3"]),
            if auto_rebase {
                unrelated
            } else {
                stack.base.clone()
            }
        );
    }
}

#[test]
fn recognizes_landed_parent_already_excluded_by_local_base() {
    let stack = Stack::landed(false, true);
    checked_git(&stack.repo, &["fetch", "origin"]);
    checked_git(
        &stack.repo.join("../base-checkout"),
        &["merge", "--ff-only", "origin/main"],
    );
    let result = stack.json(
        &["sync", "--rebase-if-needed", "--until", "1", "--json"],
        "MERGEABLE",
    );
    assert_eq!(result["sync"]["entries"][0]["gg_id"], IDS[1]);
    assert_eq!(result["sync"]["rebased_before_sync"], false);
    assert_eq!(
        checked_git(&stack.repo, &["rev-parse", "HEAD"]),
        stack.commits[2]
    );
    stack.assert_retargeted_before_view("MERGEABLE");
    assert!(!stack.repo.join(".git/pushes").exists());
}

#[test]
fn gitlab_sync_updates_titles_without_removing_draft_prefix() {
    for (remote_title, draft, draft_flag, expected_title) in [
        ("Draft: Old title", true, false, "Draft: Entry 1"),
        ("Draft: Old title", true, true, "Draft: Entry 1"),
        ("[Draft] Old title", true, false, "[Draft] Entry 1"),
        ("(draft) Old title", true, false, "(draft) Entry 1"),
        ("Old title", false, true, "Entry 1"),
    ] {
        let stack = Stack::landed(false, false);
        checked_git(&stack.repo, &["reset", "--hard", &stack.commits[0]]);
        let path = stack.repo.join(".git/gg/config.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        config["defaults"]["provider"] = json!("gitlab");
        fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
        fs::write(stack.bin.join("title"), remote_title).unwrap();
        fs::write(stack.bin.join("draft"), draft.to_string()).unwrap();
        executable(
            &stack.bin.join("glab"),
            r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
case "$1 ${2:-}" in
  '--version ') echo 'glab 1.0.0';;
  'auth status') exit 0;;
  'mr view')
    [ "$3" = 1 ]
    head=$(cat "$dir/head1"); base=$(cat "$dir/base1")
    oid=$(git --git-dir="$TEST_ORIGIN" rev-parse "refs/heads/$head")
    printf '{"iid":1,"title":"%s","draft":%s,"state":"opened","web_url":"https://gitlab.test/repo/-/merge_requests/1","source_branch":"%s","sha":"%s","target_branch":"%s","has_conflicts":false}\n' "$(cat "$dir/title")" "$(cat "$dir/draft")" "$head" "$oid" "$base";;
  'mr update')
    [ "$3" = 1 ]
    case "$4" in
      --title) printf '%s\n' "$5" > "$dir/updated-title";;
      --target-branch) [ "$5" = main ];;
      *) exit 1;;
    esac;;
  *) echo "unexpected glab $*" >&2; exit 1;;
esac
"#,
        );
        let mut args = vec!["sync", "--update-title", "--no-rebase-check", "--json"];
        if draft_flag {
            args.push("--draft");
        }
        let result = stack.json(&args, "");
        assert!(result["sync"]["entries"][0]["error"].is_null(), "{result}");
        assert_eq!(
            fs::read_to_string(stack.bin.join("updated-title"))
                .unwrap()
                .trim(),
            expected_title
        );
    }
}

#[test]
fn gitlab_retargets_without_pushing_or_queueing() {
    let stack = Stack::landed(false, true);
    let path = stack.repo.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    config["defaults"]["provider"] = json!("gitlab");
    fs::write(path, serde_json::to_vec(&config).unwrap()).unwrap();
    executable(
        &stack.bin.join("glab"),
        r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
case "$1 ${2:-}" in
  '--version ') echo 'glab 1.0.0';;
  'auth status') exit 0;;
  'mr update')
    [ "$3 $4 $5" = '2 --target-branch main' ]
    echo 'edit 2 main' >> "$dir/events"
    echo main > "$dir/base2";;
  'mr view')
    n=$3; head=$(cat "$dir/head$n"); base=$(cat "$dir/base$n")
    oid=$(git --git-dir="$TEST_ORIGIN" rev-parse "refs/heads/$head")
    detail=need_rebase
    [ "$base" != main ] || detail=ci_still_running
    echo "view $n $base $detail" >> "$dir/events"
    printf '{"iid":%s,"title":"Entry %s","state":"opened","web_url":"https://gitlab.test/repo/-/merge_requests/%s","source_branch":"%s","sha":"%s","target_branch":"%s","detailed_merge_status":"%s","has_conflicts":false}\n' "$n" "$n" "$n" "$head" "$oid" "$base" "$detail";;
  *) echo "unexpected glab $*" >&2; exit 1;;
esac
"#,
    );
    let result = stack.json(
        &["sync", "--rebase-if-needed", "--until", "2", "--json"],
        "",
    );
    assert_eq!(result["sync"]["rebased_before_sync"], false);
    assert_eq!(result["sync"]["entries"][0]["pushed"], false);
    assert!(result["sync"]["entries"][0]["error"].is_null());
    stack.assert_retargeted_before_view("ci_still_running");
    stack.assert_untouched();
}
