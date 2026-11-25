use super::{checked_git, write_remote_prune_config};
use crate::helpers::{create_test_repo_with_remote, run_gg_with_env};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const IDS: [&str; 3] = ["c-1111111", "c-2222222", "c-3333333"];

// Read heads from the bare origin, not the working copy: a successful push must
// change the SHA reported by the forge. A target edit takes effect on the next view.
const GH: &str = r#"#!/bin/sh
set -eu
dir=$(dirname "$0")
case "$1 ${2:-}" in
  '--version ') echo 'gh version 2.0.0';;
  'auth status') exit 0;;
  'pr list')
    for n in 2 3; do
      case "$3" in --head) key=head;; --base) key=base;; *) exit 1;; esac
      [ "$(cat "$dir/$key$n")" != "$4" ] || echo "$n"
    done;;
  'pr edit')
    [ "$4" = --base ] || exit 1
    echo "edit $3 $5" >> "$dir/events"
    [ "${EVIDENCE:-}" = stale-target ] || echo "$5" > "$dir/base$3";;
  'pr view')
    n=$3
    head=$(cat "$dir/head$n"); base=$(cat "$dir/base$n")
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
    echo "view $n $base $mergeable" >> "$dir/events"
    case "$mergeable" in MERGEABLE) detail=CLEAN;; CONFLICTING) detail=DIRTY;; *) detail=UNKNOWN;; esac
    printf '{"number":%s,"title":"Entry %s","state":"%s","url":"https://github.com/test/repo/pull/%s","headRefName":"%s","headRefOid":"%s","baseRefName":"%s","mergeCommit":%s,"isDraft":false,"mergeable":"%s","mergeStateStatus":"%s","reviews":[],"statusCheckRollup":[]}\n' "$n" "$n" "$state" "$n" "$head" "$oid" "$base" "$merged" "$mergeable" "$detail";;
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
        run_gg_with_env(
            &self.repo,
            args,
            &[
                ("PATH", self.path.as_os_str()),
                ("TEST_ORIGIN", self.remote.as_os_str()),
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
