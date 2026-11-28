use crate::helpers::{
    create_test_repo, create_test_repo_with_remote, run_gg, run_git, ProcessGroupGuard,
};

use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
fn write_waiting_fake_gh(path: &Path) {
    fs::write(
        path,
        r#"#!/bin/sh
set -eu

if [ "$1" = "--version" ]; then
  echo "gh version 2.97.0"
  exit 0
fi

if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
  num="${3:-41}"
  calls=0
  if [ -f "$GG_FAKE_PR_VIEW_CALLS" ]; then calls=$(cat "$GG_FAKE_PR_VIEW_CALLS"); fi
  calls=$((calls + 1))
  echo "$calls" > "$GG_FAKE_PR_VIEW_CALLS"
  if [ -f "$GG_FAKE_CLEAN_PROVIDER_FAIL" ] && [ "$calls" -gt 1 ]; then
    echo "provider unavailable" >&2
    exit 1
  fi
  case "$*" in
    *statusCheckRollup*)
      touch "$GG_FAKE_POLLED"
      calls=0
      if [ -f "$GG_FAKE_CI_CALLS" ]; then calls=$(cat "$GG_FAKE_CI_CALLS"); fi
      calls=$((calls + 1))
      echo "$calls" > "$GG_FAKE_CI_CALLS"
      if [ -f "$GG_FAKE_REGRESS" ] && [ "$calls" -eq 3 ]; then
        echo PENDING
      elif [ -f "$GG_FAKE_READY" ]; then
        echo SUCCESS
      else
        echo PENDING
      fi
      exit 0
      ;;
    *"--jq .reviewDecision"*)
      approval=APPROVED
      if [ -f "$GG_FAKE_APPROVAL_REGRESS" ]; then
        calls=0; [ ! -f "$GG_FAKE_APPROVAL_REGRESS.calls" ] || calls=$(cat "$GG_FAKE_APPROVAL_REGRESS.calls")
        calls=$((calls + 1)); echo "$calls" > "$GG_FAKE_APPROVAL_REGRESS.calls"
        [ "$calls" -eq 1 ] || approval=REVIEW_REQUIRED
      elif [ -f "$GG_FAKE_UNAPPROVED" ]; then approval=REVIEW_REQUIRED; fi
      echo "$approval"
      exit 0
      ;;
  esac

  if [ "$num" = "42" ]; then
    if [ -f "$GG_FAKE_MERGED" ]; then state=MERGED; else state=OPEN; fi
    printf '{"number":42,"title":"Second entry","state":"%s","url":"https://github.com/test/repo/pull/42","headRefName":"testuser/land-wait--c-2222222","isDraft":false,"mergeable":"MERGEABLE","reviews":[],"reviewDecision":"APPROVED"}\n' "$state"
    exit 0
  fi

  if [ -f "$GG_FAKE_CLOSED" ]; then state=CLOSED; elif [ -f "$GG_FAKE_MERGED" ] || { [ -f "$GG_FAKE_FIRST_TERMINAL_AFTER_REFRESH" ] && [ "$calls" -gt 2 ]; }; then state=MERGED; else state=OPEN; fi
  if [ -f "$GG_FAKE_DRAFT" ]; then draft=true; else draft=false; fi
  printf '{"number":41,"title":"Land entry","state":"%s","url":"https://github.com/test/repo/pull/41","headRefName":"testuser/land-wait--c-1111111","isDraft":%s,"mergeable":"MERGEABLE","reviews":[],"reviewDecision":"APPROVED"}\n' "$state" "$draft"
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
  touch "$GG_FAKE_MERGED"
  if [ -n "${GG_FAKE_EFFECT_SAVE_BLOCK:-}" ]; then
    for record in .git/gg/operations/*.json; do
      if [ -f "$record" ] && grep -q '"status": "pending"' "$record"; then
        mkdir "$record.tmp"
        printf '%s\n' "$record.tmp" > "$GG_FAKE_EFFECT_SAVE_BLOCK"
        break
      fi
    done
  fi
  exit 0
fi

if [ "$1" = "pr" ] && [ "$2" = "edit" ]; then
  if [ -n "${GG_FAKE_EFFECT_SAVE_BLOCK:-}" ] && [ -f "$GG_FAKE_EFFECT_SAVE_BLOCK" ]; then
    rmdir "$(cat "$GG_FAKE_EFFECT_SAVE_BLOCK")"
    touch "$GG_FAKE_EFFECT_SAVE_BLOCK.released"
  fi
  if [ -f "$GG_FAKE_RETARGET_FAIL" ]; then
    echo "retarget failed" >&2
    exit 1
  fi
  exit 0
fi

echo "unexpected gh invocation: $*" >&2
exit 1
"#,
    )
    .expect("write fake gh");
    let mut permissions = fs::metadata(path).expect("stat fake gh").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make fake gh executable");
}

#[cfg(unix)]
fn write_fake_land_glab(path: &Path) {
    fs::write(
        path,
        r#"#!/bin/sh
set -eu

if [ -f "$GG_FAKE_MERGE_SETTINGS_FAIL" ] && [ "$(cat "$GG_FAKE_MERGE_SETTINGS_FAIL")" = "forbid-all" ]; then touch "$GG_FAKE_MERGE_SETTINGS_FAIL.accessed"; exit 1; fi

if [ "$1" = "--version" ]; then
  echo "glab version 1.0.0"
  exit 0
fi

# MR pipeline status; with auto-merge the MR joins the train only once it passes.
ci=success
if [ -f "$GG_FAKE_GLAB_CI" ]; then ci=$(cat "$GG_FAKE_GLAB_CI"); fi
retarget_pipeline=
if [ -f "$GG_FAKE_RETARGET_PIPELINE" ]; then retarget_pipeline=$(cat "$GG_FAKE_RETARGET_PIPELINE"); fi
raw_api=false

if [ "$1" = "auth" ] && [ "$2" = "status" ]; then
  if [ -f "$GG_FAKE_AUTH_NETWORK_FAIL" ]; then
    echo "Could not resolve host: gitlab.com" >&2
    exit 1
  fi
  exit 0
fi

if [ "$1" = "api" ] && [ "$2" = "projects/:id" ]; then
  if [ -f "$GG_FAKE_MERGE_SETTINGS_FAIL" ]; then
    merge_settings_mode=$(cat "$GG_FAKE_MERGE_SETTINGS_FAIL")
    if [ "$merge_settings_mode" = "invalid" ]; then
      echo "not json"
      exit 0
    fi
    if [ "$merge_settings_mode" = "missing" ]; then
      echo '{"merge_trains_enabled":false}'
      exit 0
    fi
    echo "project settings unavailable" >&2
    exit 1
  fi
  if [ -f "$GG_FAKE_MERGE_TRAINS" ]; then trains=true; else trains=false; fi
  squash_option=default_off
  if [ -f "$GG_FAKE_SQUASH_OPTION" ]; then squash_option=$(cat "$GG_FAKE_SQUASH_OPTION"); fi
  printf '{"merge_trains_enabled":%s,"squash_option":"%s"}\n' "$trains" "$squash_option"
  exit 0
fi

if [ "$1" = "api" ] && [ "$2" = "--method" ] && [ "$3" = "PUT" ] && [ "$4" = "projects/:id/merge_requests/41" ]; then
  printf '%s\n' "$*" > "$GG_FAKE_SQUASH_DISABLED"
  printf 'attempt\n' >> "$GG_FAKE_SQUASH_DISABLED.attempts"
  squash_mode=ok
  if [ -f "$GG_FAKE_LANDING_REQUEST_FAIL" ]; then squash_mode=$(cat "$GG_FAKE_LANDING_REQUEST_FAIL"); fi
  if [ "$squash_mode" = "squash-unapplied-response-fail" ]; then
    echo "connection lost before squash update" >&2
    exit 1
  fi
  if [ "$squash_mode" = "squash-block-success" ]; then
    touch "$GG_FAKE_LANDING_REQUEST_FAIL.started"
    wait_count=0
    while [ ! -f "$GG_FAKE_LANDING_REQUEST_FAIL.release" ] && [ "$wait_count" -lt 400 ]; do
      sleep 0.05; wait_count=$((wait_count + 1))
    done
    [ -f "$GG_FAKE_LANDING_REQUEST_FAIL.release" ] || exit 1
  fi
  rm -f "$GG_FAKE_MR_SQUASH"
  if [ "$squash_mode" = "squash-response-fail" ]; then
    echo "connection lost after squash update" >&2
    exit 1
  fi
  printf '{"iid":41,"source_project_id":7,"target_project_id":7,"state":"opened","source_branch":"testuser/land-wait--c-1111111","target_branch":"main","sha":"1111111111111111111111111111111111111111","draft":false,"work_in_progress":false,"squash":false}\n'
  exit 0
fi

if [ "$1" = "api" ] && [ "$2" = "-X" ] && [ "$3" = "POST" ] && [ "$4" = "projects/7/merge_requests/42/pipelines" ]; then
  printf 'attempt\n' >> "$GG_FAKE_RETARGET_PIPELINE.attempts"
  if [ "$retarget_pipeline" = "rejected" ]; then
    echo "Pipeline filtered out by workflow rules" >&2
    exit 1
  fi
  if [ "$retarget_pipeline" = "applied-response-block-fail" ]; then
    touch "$GG_FAKE_RETARGET_PIPELINE.started"
    touch "$GG_FAKE_LANDING_REQUEST_FAIL.started"
    wait_count=0
    while [ ! -f "$GG_FAKE_LANDING_REQUEST_FAIL.release" ] && [ "$wait_count" -lt 400 ]; do
      sleep 0.05
      wait_count=$((wait_count + 1))
    done
    if [ ! -f "$GG_FAKE_LANDING_REQUEST_FAIL.release" ]; then
      echo "timed out waiting to release pipeline response" >&2
      exit 1
    fi
    echo "connection lost after pipeline creation" >&2
    exit 1
  fi
  if [ "$retarget_pipeline" = "applied-response-block-success" ]; then
    touch "$GG_FAKE_RETARGET_PIPELINE.started"
    wait_count=0
    while [ ! -f "$GG_FAKE_LANDING_REQUEST_FAIL.release" ] && [ "$wait_count" -lt 400 ]; do
      sleep 0.05
      wait_count=$((wait_count + 1))
    done
    [ -f "$GG_FAKE_LANDING_REQUEST_FAIL.release" ] || exit 1
    echo '{"id":4242,"sha":"merge-result","ref":"refs/merge-requests/42/merge","status":"pending"}'
    exit 0
  fi
  if [ "$retarget_pipeline" = "applied-response-fail" ]; then
    touch "$GG_FAKE_RETARGET_PIPELINE.started"
    echo "connection lost after pipeline creation" >&2
    exit 1
  fi
  if { [ "$retarget_pipeline" != "missing" ] && [ "$retarget_pipeline" != "missing-after-pipeline" ] && [ "$retarget_pipeline" != "missing-draft" ] && [ "$retarget_pipeline" != "missing-work-in-progress" ]; } || [ ! -f "$GG_FAKE_MERGED" ] || [ -f "$GG_FAKE_RETARGET_PIPELINE.started" ]; then
    echo "unexpected pipeline creation" >&2
    exit 1
  fi
  touch "$GG_FAKE_RETARGET_PIPELINE.started"
  echo '{"id":4242,"sha":"merge-result","ref":"refs/merge-requests/42/merge","status":"pending"}'
  exit 0
fi

is_landing_request=false
if [ "$1" = "api" ] && [ "$2" = "-X" ] && [ "$3" = "POST" ]; then
  is_landing_request=true
fi
if [ "$1" = "api" ] && [ "$2" = "--method" ] && [ "$3" = "PUT" ] && [ "$4" = "projects/:id/merge_requests/41/merge" ]; then
  is_landing_request=true
fi
if [ "$is_landing_request" = "true" ]; then
  case "$*" in
    *merge_when_pipeline_succeeds=true*) printf 'attempt\n' >> "$GG_FAKE_QUEUED.attempts" ;;
  esac
  if [ -n "$retarget_pipeline" ]; then
    case "$4" in
      projects/:id/merge_trains/merge_requests/41)
        merge=$(git commit-tree 'HEAD~1^{tree}' -p HEAD~2 -p HEAD~1 -m 'Merge MR !41')
        git push --quiet "$(git remote get-url origin)" "$merge:refs/heads/main"
        touch "$GG_FAKE_MERGED"
        ;;
      projects/:id/merge_trains/merge_requests/42)
        if [ "$retarget_pipeline" != "running" ] && [ ! -f "$GG_FAKE_RETARGET_PIPELINE.started" ] && [ ! -f "$GG_FAKE_RETARGET_PIPELINE.attempts" ]; then
          echo "MR !42 still needs a pipeline for the new target" >&2
          exit 1
        fi
        ;;
    esac
  fi
  landing_request_mode=ok
  if [ -f "$GG_FAKE_LANDING_REQUEST_FAIL" ]; then
    landing_request_mode=$(cat "$GG_FAKE_LANDING_REQUEST_FAIL")
  fi
  if [ -f "$GG_FAKE_AUTO_MERGE_DROPPED" ]; then
    printf 'attempt\n' >> "$GG_FAKE_LANDING_REQUEST_FAIL.attempts"
    if [ "$landing_request_mode" = "requeue-head-race" ]; then
      printf 'post:3333333333333333333333333333333333333333\n' >> "$GG_FAKE_LANDING_REQUEST_FAIL.order"
    fi
  fi
  if [ "$landing_request_mode" = "block-fail" ] || [ "$landing_request_mode" = "block-success" ] || { { [ "$landing_request_mode" = "block-requeue-success" ] || [ "$landing_request_mode" = "block-requeue-fail" ]; } && [ -f "$GG_FAKE_AUTO_MERGE_DROPPED" ]; } || { [ "$landing_request_mode" = "block-second-fail" ] && [ "$4" = "projects/:id/merge_trains/merge_requests/42" ]; } || { [ "$landing_request_mode" = "block-second-success" ] && [ "$4" = "projects/:id/merge_trains/merge_requests/42" ]; }; then
    touch "$GG_FAKE_LANDING_REQUEST_FAIL.started"
    wait_count=0
    while [ ! -f "$GG_FAKE_LANDING_REQUEST_FAIL.release" ] && [ "$wait_count" -lt 400 ]; do
      sleep 0.05
      wait_count=$((wait_count + 1))
    done
    if [ ! -f "$GG_FAKE_LANDING_REQUEST_FAIL.release" ]; then
      echo "timed out waiting to release landing request" >&2
      exit 1
    fi
  fi
  if [ -f "$GG_FAKE_AUTO_MERGE_DROPPED" ]; then
    case "$landing_request_mode" in
      requeue-success-finalization-fail|requeue-fail-finalization-fail)
        for record in .git/gg/operations/*.json; do
          if [ -f "$record" ] && grep -q 'gitlab:requeue_merge_train' "$record"; then mkdir "$record.tmp"; break; fi
        done
        ;;
      requeue-success-marker-clear-fail) mkdir .git/gg/config.tmp ;;
    esac
  fi
  if [ "$landing_request_mode" = "block-fail" ] || { { [ "$landing_request_mode" = "block-requeue-fail" ] || [ "$landing_request_mode" = "requeue-fail-finalization-fail" ]; } && [ -f "$GG_FAKE_AUTO_MERGE_DROPPED" ]; } || { [ "$landing_request_mode" = "block-second-fail" ] && [ "$4" = "projects/:id/merge_trains/merge_requests/42" ]; }; then
    echo "landing request failed" >&2
    exit 1
  fi
  if [ "$landing_request_mode" = "fail-second" ] && [ "$4" = "projects/:id/merge_trains/merge_requests/42" ]; then
    echo "second landing request failed" >&2
    exit 1
  fi
  if [ "$landing_request_mode" = "fail" ]; then
    echo "landing request failed" >&2
    exit 1
  fi
  if [ "$landing_request_mode" = "already-auto-merge" ]; then echo "409 already set to Auto-Merge" >&2; exit 1; fi
  if [ "$landing_request_mode" = "auto-merge-applied-response-fail" ]; then
    printf 'attempt\n' >> "$GG_FAKE_LANDING_REQUEST_FAIL.attempts"
    printf '%s\n' "$*" > "$GG_FAKE_QUEUED"
    echo "connection lost after enabling auto-merge" >&2
    exit 1
  fi
  if [ "$landing_request_mode" = "merge-train-applied-response-fail" ]; then
    printf 'attempt\n' >> "$GG_FAKE_QUEUED.attempts"
    printf '%s\n' "$*" > "$GG_FAKE_QUEUED"
    echo "connection lost after adding MR to merge train" >&2
    exit 1
  fi
  if [ "$landing_request_mode" = "requeue-merge" ] && [ -f "$GG_FAKE_AUTO_MERGE_DROPPED" ]; then touch "$GG_FAKE_MERGED"; fi
  if [ "$landing_request_mode" = "requeue-head-race" ] && [ "$(wc -l < "$GG_FAKE_LANDING_REQUEST_FAIL.attempts")" -ge 2 ]; then touch "$GG_FAKE_MERGED"; fi
  if [ -f "$GG_FAKE_AUTO_MERGE_DROPPED" ]; then
    printf '%s\n' "$*" > "$GG_FAKE_REQUEUED"
    if [ "$landing_request_mode" != "requeue-head-race" ] || [ "$(wc -l < "$GG_FAKE_LANDING_REQUEST_FAIL.attempts")" -ge 2 ]; then
      rm -f "$GG_FAKE_AUTO_MERGE_DROPPED"
    fi
  fi
  printf '%s\n' "$*" > "$GG_FAKE_QUEUED"
  exit 0
fi

if [ "$1" = "api" ]; then
  if [ "$2" = "projects/:id/merge_requests/42" ] && [ "$retarget_pipeline" = "lookup-fail" ]; then
    echo "MR lookup failed" >&2
    exit 1
  fi
  case "$2" in
    projects/:id/merge_requests/*/approvals)
      approved=true
      if [ -f "$GG_FAKE_APPROVAL_REGRESS" ]; then
        approval_mode=$(cat "$GG_FAKE_APPROVAL_REGRESS")
        if [ "$approval_mode" = "fail-squash-segment" ]; then
          if [ -f "$GG_FAKE_APPROVAL_REGRESS.forbid" ]; then touch "$GG_FAKE_APPROVAL_REGRESS.failed"; echo "approval lookup failed" >&2; exit 1; fi
          for record in .git/gg/operations/*.json; do
            if [ -f "$record" ] && grep -q 'gitlab:squash_disable' "$record" && grep -q '"status": "pending"' "$record"; then
              touch "$GG_FAKE_APPROVAL_REGRESS.failed"; echo "approval lookup failed" >&2; exit 1
            fi
          done
        elif [ "$approval_mode" = "after-validation" ]; then
          reads=0; [ ! -f "$GG_FAKE_APPROVAL_REGRESS.raw-reads" ] || reads=$(cat "$GG_FAKE_APPROVAL_REGRESS.raw-reads")
          printf 'approval:%s\n' "$reads" >> "$GG_FAKE_APPROVAL_REGRESS.order"
          [ "$reads" -lt 3 ] || approved=false
        elif [ "$approval_mode" = "after-requeue-validation" ]; then
          if [ -f "$GG_FAKE_AUTO_MERGE_DROPPED" ]; then
            reads=0; [ ! -f "$GG_FAKE_APPROVAL_REGRESS.raw-reads" ] || reads=$(cat "$GG_FAKE_APPROVAL_REGRESS.raw-reads")
            printf 'approval:%s\n' "$reads" >> "$GG_FAKE_APPROVAL_REGRESS.order"; [ ! -f "$GG_FAKE_APPROVAL_REGRESS.requeue-validated" ] || approved=false
          fi
        elif [ "$approval_mode" = "pipeline-operation" ]; then
          grep -q 'gitlab:pipeline_start' .git/gg/operations/*.json 2>/dev/null && approved=false
        else
          calls=0; [ ! -f "$GG_FAKE_APPROVAL_REGRESS.calls" ] || calls=$(cat "$GG_FAKE_APPROVAL_REGRESS.calls")
          calls=$((calls + 1)); echo "$calls" > "$GG_FAKE_APPROVAL_REGRESS.calls"
          [ "$calls" -le "$approval_mode" ] || approved=false
        fi
      elif [ -f "$GG_FAKE_UNAPPROVED" ]; then approved=false; fi
      if [ -f "$GG_FAKE_READINESS_POLL.enabled" ]; then touch "$GG_FAKE_READINESS_POLL.armed"; fi
      printf '{"approved":%s}\n' "$approved"
      exit 0
      ;;
    projects/:id/merge_trains/merge_requests/*)
      train_mr="${2##*/}"
      if [ -f "$GG_FAKE_TRAIN_MERGED" ] && [ "$train_mr" = "41" ]; then
        train_merged_mode=$(cat "$GG_FAKE_TRAIN_MERGED")
        if [ "$train_merged_mode" = "block" ]; then
          touch "$GG_FAKE_TRAIN_MERGED.started"
          wait_count=0
          while [ ! -f "$GG_FAKE_TRAIN_MERGED.release" ] && [ "$wait_count" -lt 400 ]; do
            sleep 0.05
            wait_count=$((wait_count + 1))
          done
          if [ ! -f "$GG_FAKE_TRAIN_MERGED.release" ]; then
            echo "timed out waiting to release merge-train status" >&2
            exit 1
          fi
        fi
        touch "$GG_FAKE_MERGED"
        printf '{"merge_request":{"iid":%s},"status":"merged","pipeline":{"status":"success"}}\n' "$train_mr"
        exit 0
      fi
      if [ -f "$GG_FAKE_MERGED" ] && { [ "$train_mr" = "41" ] || { [ -f "$GG_FAKE_QUEUED" ] && grep -q "merge_requests/$train_mr" "$GG_FAKE_QUEUED"; }; }; then
        printf '{"merge_request":{"iid":%s},"status":"merged","pipeline":{"status":"success"}}\n' "$train_mr"
        exit 0
      fi
      if [ -f "$GG_FAKE_QUEUED" ] && grep -q "merge_requests/$train_mr" "$GG_FAKE_QUEUED" && [ "$ci" = success ]; then
        touch "$GG_FAKE_POLLED"
        printf '{"merge_request":{"iid":%s},"status":"fresh","pipeline":{"status":"success"}}\n' "$train_mr"
        exit 0
      fi
      echo "404 Not Found" >&2
      exit 1
      ;;
    projects/:id/merge_trains/*scope=active*)
      if [ ! -f "$GG_FAKE_QUEUED" ] || [ "$ci" != success ]; then
        echo '[]'
        exit 0
      fi
      touch "$GG_FAKE_POLLED"
      if [ -f "$GG_FAKE_MERGED" ] || [ -f "$GG_FAKE_TRAIN_MERGED" ]; then
        echo '[]'
      else
        echo '[{"merge_request":{"iid":41},"status":"fresh","pipeline":{"status":"success"}}]'
      fi
      exit 0
      ;;
    projects/:id/merge_trains/*scope=complete*)
      if [ -f "$GG_FAKE_MERGED" ] || [ -f "$GG_FAKE_TRAIN_MERGED" ]; then
        echo '[{"merge_request":{"iid":41},"status":"merged","pipeline":{"status":"success"}}]'
      else
        echo '[]'
      fi
      exit 0
      ;;
    projects/:id/merge_requests/41|projects/:id/merge_requests/42)
      raw_api=true
      set -- mr view "${2##*/}" --output json
      ;;
  esac
fi

if [ "$1" = "mr" ] && [ "$2" = "view" ]; then
  num="${3:-41}"
  if [ "$raw_api" = "false" ] && [ -f "$GG_FAKE_READINESS_POLL.enabled" ]; then
    readiness_views=0; [ ! -f "$GG_FAKE_READINESS_POLL.view-calls" ] || readiness_views=$(cat "$GG_FAKE_READINESS_POLL.view-calls")
    readiness_views=$((readiness_views + 1)); echo "$readiness_views" > "$GG_FAKE_READINESS_POLL.view-calls"
    if [ -f "$GG_FAKE_READINESS_POLL.armed" ] && [ ! -f "$GG_FAKE_READINESS_POLL" ]; then
      touch "$GG_FAKE_READINESS_POLL"
      wait_count=0
      while [ ! -f "$GG_FAKE_READINESS_POLL.release" ] && [ "$wait_count" -lt 400 ]; do sleep 0.05; wait_count=$((wait_count + 1)); done
      [ -f "$GG_FAKE_READINESS_POLL.release" ] || { echo "timed out waiting to release readiness poll" >&2; exit 1; }
      ci=$(cat "$GG_FAKE_GLAB_CI")
    fi
  fi
  target_branch=main
  detailed_merge_status=mergeable
  head_pipeline="{\"status\":\"$ci\"}"
  if [ "$num" = "42" ]; then
    state=opened
    iid=42
    title="Second entry"
    source_branch="testuser/land-wait--c-2222222"
    sha=2222222222222222222222222222222222222222
    if [ -n "$retarget_pipeline" ]; then
      if [ ! -f "$GG_FAKE_MERGED" ]; then
        target_branch="testuser/land-wait--c-1111111"
      elif [ -f "$GG_FAKE_RETARGET_PIPELINE.started" ]; then
        head_pipeline='{"id":4242,"status":"pending"}'
      elif [ "$retarget_pipeline" = "running" ]; then
        head_pipeline='{"id":4241,"status":"running"}'
      else
        detailed_merge_status=ci_must_pass
        head_pipeline=null
      fi
    fi
  else
    if [ -f "$GG_FAKE_CLOSED" ]; then state=closed; elif [ -f "$GG_FAKE_MERGED" ]; then state=merged; else state=opened; fi
    iid=41
    title="Land entry"
    source_branch="testuser/land-wait--c-1111111"
    # A push to the source branch moves the first MR head.
    if [ -f "$GG_FAKE_HEAD_MOVED" ]; then sha=2222222222222222222222222222222222222222; else sha=1111111111111111111111111111111111111111; fi
    if [ "$(cat "$GG_FAKE_LANDING_REQUEST_FAIL" 2>/dev/null)" = "requeue-head-race" ] && [ -f "$GG_FAKE_AUTO_MERGE_DROPPED" ]; then
      if [ "$raw_api" = "true" ] || [ -f "$GG_FAKE_LANDING_REQUEST_FAIL.attempts" ]; then sha=3333333333333333333333333333333333333333; fi
      if [ "$raw_api" = "true" ]; then
        printf 'validation:%s\n' "$sha" >> "$GG_FAKE_LANDING_REQUEST_FAIL.order"
      elif [ ! -f "$GG_FAKE_LANDING_REQUEST_FAIL.poll-$sha" ]; then
        touch "$GG_FAKE_LANDING_REQUEST_FAIL.poll-$sha"
        printf 'poll:%s\n' "$sha" >> "$GG_FAKE_LANDING_REQUEST_FAIL.order"
      fi
    fi
  fi
  # GitLab reports queued MRs (auto-merge or merge train) through this flag.
  if [ -f "$GG_FAKE_QUEUED" ] && [ ! -f "$GG_FAKE_AUTO_MERGE_DROPPED" ]; then auto=true; else auto=false; fi
  if [ "$raw_api" = "false" ] && [ "$num" = "41" ] && [ "$(cat "$GG_FAKE_APPROVAL_REGRESS" 2>/dev/null)" = "after-requeue-validation" ] && [ -f "$GG_FAKE_AUTO_MERGE_DROPPED" ] && [ ! -f "$GG_FAKE_APPROVAL_REGRESS.poll-seen" ]; then touch "$GG_FAKE_APPROVAL_REGRESS.poll-seen"; printf 'poll\n' >> "$GG_FAKE_APPROVAL_REGRESS.order"; fi
  if [ -f "$GG_FAKE_MR_SQUASH" ]; then squash=true; else squash=false; fi
  if [ "$num" = "41" ] && [ "$squash" = "false" ] && [ "$(cat "$GG_FAKE_APPROVAL_REGRESS" 2>/dev/null)" = "fail-squash-segment" ]; then
    if [ "$raw_api" = "true" ]; then
      reads=0; [ ! -f "$GG_FAKE_APPROVAL_REGRESS.raw-reads" ] || reads=$(cat "$GG_FAKE_APPROVAL_REGRESS.raw-reads")
      reads=$((reads + 1)); echo "$reads" > "$GG_FAKE_APPROVAL_REGRESS.raw-reads"; [ "$reads" -ne 2 ] || touch "$GG_FAKE_APPROVAL_REGRESS.forbid"
    else rm -f "$GG_FAKE_APPROVAL_REGRESS.forbid"; fi
  fi
  if [ "$raw_api" = "true" ] && [ "$num" = "42" ]; then
    reads=0
    if [ -f "$GG_FAKE_RETARGET_PIPELINE.reads" ]; then reads=$(cat "$GG_FAKE_RETARGET_PIPELINE.reads"); fi
    reads=$((reads + 1)); echo "$reads" > "$GG_FAKE_RETARGET_PIPELINE.reads"
    if [ "$retarget_pipeline" = "identity-change" ] && grep -Eq 'gitlab:(pipeline_start|squash_disable)' .git/gg/operations/*.json 2>/dev/null; then
      sha=3333333333333333333333333333333333333333
    fi
    if [ "$retarget_pipeline" = "recovery-order" ]; then
      for record in .git/gg/operations/*.json; do
        if [ -f "$record" ] && grep -q 'gitlab:pipeline_start' "$record" && grep -q '"status": "pending"' "$record"; then
          touch "$GG_FAKE_RETARGET_PIPELINE.early"; echo "pipeline recovery not finalized" >&2; exit 1
        fi
      done
    fi
  fi
  if [ "$raw_api" = "true" ] && [ "$num" = "41" ] && [ "$retarget_pipeline" = "identity-change" ] && grep -q 'gitlab:squash_disable' .git/gg/operations/*.json 2>/dev/null; then
    sha=3333333333333333333333333333333333333333
  fi
  if [ "$num" = "42" ] && [ "$retarget_pipeline" = "preceding-finalize-fail" ]; then
    for record in .git/gg/operations/*.json; do
      if [ -f "$record" ] && grep -q '"status": "pending"' "$record"; then mkdir "$record.tmp"; break; fi
    done
  fi
  if [ "$raw_api" = "true" ] && [ "$retarget_pipeline" = "malformed-squash-view" ]; then
    if [ -f "$GG_FAKE_RETARGET_PIPELINE.raw-seen" ]; then
      echo 'not json'
      exit 0
    fi
    touch "$GG_FAKE_RETARGET_PIPELINE.raw-seen"
  fi
  if [ "$raw_api" = "true" ] && [ "$retarget_pipeline" = "closed-on-landing" ]; then
    state=closed
  fi
  if [ "$raw_api" = "true" ] && [ "$retarget_pipeline" = "target-mismatch-on-landing" ]; then
    target_branch=other-base
  fi
  if [ "$raw_api" = "true" ] && { [ "$retarget_pipeline" = "draft-on-landing" ] || { [ "$retarget_pipeline" = "draft-on-requeue" ] && [ -f "$GG_FAKE_AUTO_MERGE_DROPPED" ]; }; }; then
    printf '{"iid":%s,"title":"%s","state":"%s","web_url":"https://gitlab.example/test/repo/-/merge_requests/%s","source_branch":"%s","target_branch":"%s","sha":"%s","draft":true,"work_in_progress":false,"detailed_merge_status":"%s","has_conflicts":false,"squash":%s,"merge_when_pipeline_succeeds":%s,"head_pipeline":%s}\n' "$iid" "$title" "$state" "$iid" "$source_branch" "$target_branch" "$sha" "$detailed_merge_status" "$squash" "$auto" "$head_pipeline"
    exit 0
  fi
  if [ "$raw_api" = "true" ] && [ "$retarget_pipeline" = "malformed-draft-on-landing" ]; then
    printf '{"iid":%s,"title":"%s","state":"%s","web_url":"https://gitlab.example/test/repo/-/merge_requests/%s","source_branch":"%s","target_branch":"%s","sha":"%s","draft":"unknown","work_in_progress":false,"detailed_merge_status":"%s","has_conflicts":false,"squash":%s,"merge_when_pipeline_succeeds":%s,"head_pipeline":%s}\n' "$iid" "$title" "$state" "$iid" "$source_branch" "$target_branch" "$sha" "$detailed_merge_status" "$squash" "$auto" "$head_pipeline"
    exit 0
  fi
  if [ "$raw_api" = "true" ] && { [ "$retarget_pipeline" = "missing-draft-on-landing" ] || { [ "$retarget_pipeline" = "missing-after-pipeline" ] && [ -f "$GG_FAKE_RETARGET_PIPELINE.started" ]; } || { [ "$retarget_pipeline" = "missing-after-squash" ] && [ -f "$GG_FAKE_SQUASH_DISABLED" ]; }; }; then
    printf '{"iid":%s,"title":"%s","state":"%s","web_url":"https://gitlab.example/test/repo/-/merge_requests/%s","source_branch":"%s","target_branch":"%s","sha":"%s","work_in_progress":false,"detailed_merge_status":"%s","has_conflicts":false,"squash":%s,"merge_when_pipeline_succeeds":%s,"head_pipeline":%s}\n' "$iid" "$title" "$state" "$iid" "$source_branch" "$target_branch" "$sha" "$detailed_merge_status" "$squash" "$auto" "$head_pipeline"
    exit 0
  fi
  if [ "$num" = "42" ] && [ "$retarget_pipeline" = "missing-draft" ]; then
    printf '{"iid":%s,"title":"%s","state":"%s","web_url":"https://gitlab.example/test/repo/-/merge_requests/%s","source_branch":"%s","target_branch":"%s","sha":"%s","work_in_progress":false,"detailed_merge_status":"%s","has_conflicts":false,"squash":%s,"merge_when_pipeline_succeeds":%s,"head_pipeline":%s}\n' "$iid" "$title" "$state" "$iid" "$source_branch" "$target_branch" "$sha" "$detailed_merge_status" "$squash" "$auto" "$head_pipeline"
    exit 0
  fi
  if [ "$num" = "42" ] && [ "$retarget_pipeline" = "missing-work-in-progress" ]; then
    printf '{"iid":%s,"title":"%s","state":"%s","web_url":"https://gitlab.example/test/repo/-/merge_requests/%s","source_branch":"%s","target_branch":"%s","sha":"%s","draft":false,"detailed_merge_status":"%s","has_conflicts":false,"squash":%s,"merge_when_pipeline_succeeds":%s,"head_pipeline":%s}\n' "$iid" "$title" "$state" "$iid" "$source_branch" "$target_branch" "$sha" "$detailed_merge_status" "$squash" "$auto" "$head_pipeline"
    exit 0
  fi
  if [ "$raw_api" = "true" ] && [ "$num" = "41" ] && [ "$(cat "$GG_FAKE_APPROVAL_REGRESS" 2>/dev/null)" = "after-validation" ]; then
    reads=0; [ ! -f "$GG_FAKE_APPROVAL_REGRESS.raw-reads" ] || reads=$(cat "$GG_FAKE_APPROVAL_REGRESS.raw-reads")
    reads=$((reads + 1)); echo "$reads" > "$GG_FAKE_APPROVAL_REGRESS.raw-reads"
    printf 'validation:%s\n' "$reads" >> "$GG_FAKE_APPROVAL_REGRESS.order"
  elif [ "$raw_api" = "true" ] && [ "$num" = "41" ] && [ "$(cat "$GG_FAKE_APPROVAL_REGRESS" 2>/dev/null)" = "after-requeue-validation" ] && [ -f "$GG_FAKE_AUTO_MERGE_DROPPED" ]; then
    reads=0; [ ! -f "$GG_FAKE_APPROVAL_REGRESS.raw-reads" ] || reads=$(cat "$GG_FAKE_APPROVAL_REGRESS.raw-reads")
    reads=$((reads + 1)); echo "$reads" > "$GG_FAKE_APPROVAL_REGRESS.raw-reads"
    touch "$GG_FAKE_APPROVAL_REGRESS.requeue-validated"; printf 'validation:%s\n' "$reads" >> "$GG_FAKE_APPROVAL_REGRESS.order"
  fi
  if [ "$raw_api" = "false" ] && [ -f "$GG_FAKE_QUEUED" ]; then touch "$GG_FAKE_LANDING_REQUEST_FAIL.waiter-polled"; fi
  printf '{"iid":%s,"source_project_id":7,"target_project_id":7,"title":"%s","state":"%s","web_url":"https://gitlab.example/test/repo/-/merge_requests/%s","source_branch":"%s","target_branch":"%s","sha":"%s","draft":false,"work_in_progress":false,"detailed_merge_status":"%s","has_conflicts":false,"squash":%s,"merge_when_pipeline_succeeds":%s,"head_pipeline":%s}\n' "$iid" "$title" "$state" "$iid" "$source_branch" "$target_branch" "$sha" "$detailed_merge_status" "$squash" "$auto" "$head_pipeline"
  exit 0
fi

if [ "$1" = "mr" ] && [ "$2" = "update" ]; then
  exit 0
fi

if [ "$1" = "mr" ] && [ "$2" = "merge" ]; then
  landing_request_mode=ok
  if [ -f "$GG_FAKE_LANDING_REQUEST_FAIL" ]; then
    landing_request_mode=$(cat "$GG_FAKE_LANDING_REQUEST_FAIL")
  fi
  if [ "$landing_request_mode" = "block-fail" ] || [ "$landing_request_mode" = "block-success" ]; then
    touch "$GG_FAKE_LANDING_REQUEST_FAIL.started"
    wait_count=0
    while [ ! -f "$GG_FAKE_LANDING_REQUEST_FAIL.release" ] && [ "$wait_count" -lt 400 ]; do
      sleep 0.05
      wait_count=$((wait_count + 1))
    done
    if [ ! -f "$GG_FAKE_LANDING_REQUEST_FAIL.release" ]; then
      echo "timed out waiting to release landing request" >&2
      exit 1
    fi
  fi
  if [ "$landing_request_mode" = "block-fail" ] || [ "$landing_request_mode" = "fail" ]; then
    echo "landing request failed" >&2
    exit 1
  fi
  touch "$GG_FAKE_MERGED"
  touch "$GG_FAKE_MERGED.glab"
  exit 0
fi

echo "unexpected glab invocation: $*" >&2
exit 1
"#,
    )
    .expect("write fake glab");
    let mut permissions = fs::metadata(path).expect("stat fake glab").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make fake glab executable");
}

#[cfg(unix)]
struct WaitingLandFixture {
    _temp_dir: tempfile::TempDir,
    repo_path: std::path::PathBuf,
    remote_path: std::path::PathBuf,
    other_worktree: std::path::PathBuf,
    path: OsString,
    polled: std::path::PathBuf,
    readiness_poll: std::path::PathBuf,
    ready: std::path::PathBuf,
    merged: std::path::PathBuf,
    closed: std::path::PathBuf,
    draft: std::path::PathBuf,
    unapproved: std::path::PathBuf,
    approval_regress: std::path::PathBuf,
    ci_calls: std::path::PathBuf,
    regress: std::path::PathBuf,
    merge_trains: std::path::PathBuf,
    train_merged: std::path::PathBuf,
    merge_settings_fail: std::path::PathBuf,
    landing_request_fail: std::path::PathBuf,
    auth_network_fail: std::path::PathBuf,
    queued: std::path::PathBuf,
    retarget_fail: std::path::PathBuf,
    clean_provider_fail: std::path::PathBuf,
    pr_view_calls: std::path::PathBuf,
    glab_ci: std::path::PathBuf,
    first_terminal_after_refresh: std::path::PathBuf,
    mr_squash: std::path::PathBuf,
    squash_disabled: std::path::PathBuf,
    squash_option: std::path::PathBuf,
    auto_merge_dropped: std::path::PathBuf,
    requeued: std::path::PathBuf,
    head_moved: std::path::PathBuf,
    test_home: std::path::PathBuf,
}

#[cfg(unix)]
impl WaitingLandFixture {
    fn new() -> Self {
        let (temp_dir, repo_path, remote_path) = create_test_repo_with_remote();
        let gg_dir = repo_path.join(".git/gg");
        fs::create_dir_all(&gg_dir).expect("create gg dir");
        fs::write(
            gg_dir.join("config.json"),
            r#"{
  "defaults": {"branch_username":"testuser","provider":"github","base":"main"},
  "stacks": {"land-wait":{"base":"main","mrs":{"c-1111111":41}}}
}"#,
        )
        .expect("write config");

        run_git(&repo_path, &["checkout", "-b", "testuser/land-wait"]);
        fs::write(repo_path.join("land.txt"), "land\n").expect("write land entry");
        run_git(&repo_path, &["add", "land.txt"]);
        run_git(
            &repo_path,
            &["commit", "-m", "Land entry\n\nGG-ID: c-1111111"],
        );

        let other_worktree = repo_path.parent().unwrap().join("other-worktree");
        let worktree_output = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "testuser/other",
                other_worktree.to_str().unwrap(),
                "main",
            ])
            .current_dir(&repo_path)
            .output()
            .expect("create other worktree");
        assert!(worktree_output.status.success());

        let fake_bin = repo_path.join("fake-bin-land-wait");
        fs::create_dir_all(&fake_bin).expect("create fake bin");
        write_waiting_fake_gh(&fake_bin.join("gh"));
        let mut path = OsString::from(fake_bin.as_os_str());
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());

        let polled = repo_path.join("fake-polled");
        let readiness_poll = repo_path.join("fake-readiness-poll");
        let ready = repo_path.join("fake-ready");
        let merged = repo_path.join("fake-merged");
        let closed = repo_path.join("fake-closed");
        let draft = repo_path.join("fake-draft");
        let unapproved = repo_path.join("fake-unapproved");
        let approval_regress = repo_path.join("fake-approval-regress");
        let ci_calls = repo_path.join("fake-ci-calls");
        let regress = repo_path.join("fake-regress");
        let merge_trains = repo_path.join("fake-merge-trains");
        let train_merged = repo_path.join("fake-train-merged");
        let merge_settings_fail = repo_path.join("fake-merge-settings-fail");
        let landing_request_fail = repo_path.join("fake-landing-request-fail");
        let auth_network_fail = repo_path.join("fake-auth-network-fail");
        let queued = repo_path.join("fake-queued");
        let retarget_fail = repo_path.join("fake-retarget-fail");
        let clean_provider_fail = repo_path.join("fake-clean-provider-fail");
        let pr_view_calls = repo_path.join("fake-pr-view-calls");
        let glab_ci = repo_path.join("fake-glab-ci");
        let first_terminal_after_refresh = repo_path.join("fake-first-terminal-after-refresh");
        let mr_squash = repo_path.join("fake-mr-squash");
        let squash_disabled = repo_path.join("fake-squash-disabled");
        let squash_option = repo_path.join("fake-squash-option");
        let auto_merge_dropped = repo_path.join("fake-auto-merge-dropped");
        let requeued = repo_path.join("fake-requeued");
        let head_moved = repo_path.join("fake-head-moved");
        let test_home = repo_path.join(".test-home");
        fs::create_dir_all(&test_home).expect("create test home");

        Self {
            _temp_dir: temp_dir,
            repo_path,
            remote_path,
            other_worktree,
            path,
            polled,
            readiness_poll,
            ready,
            merged,
            closed,
            draft,
            unapproved,
            approval_regress,
            ci_calls,
            regress,
            merge_trains,
            train_merged,
            merge_settings_fail,
            landing_request_fail,
            auth_network_fail,
            queued,
            retarget_fail,
            clean_provider_fail,
            pr_view_calls,
            glab_ci,
            first_terminal_after_refresh,
            mr_squash,
            squash_disabled,
            squash_option,
            auto_merge_dropped,
            requeued,
            head_moved,
            test_home,
        }
    }

    fn start_land(&self) -> std::process::Child {
        self.start_land_with_args(&["land", "--all", "--wait", "--no-clean"])
    }

    fn land_command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_gg"));
        command
            .args(args)
            .current_dir(&self.repo_path)
            .env("HOME", &self.test_home)
            .env("PATH", &self.path)
            .env("GG_FAKE_POLLED", &self.polled)
            .env("GG_FAKE_READINESS_POLL", &self.readiness_poll)
            .env("GG_FAKE_READY", &self.ready)
            .env("GG_FAKE_MERGED", &self.merged)
            .env("GG_FAKE_CLOSED", &self.closed)
            .env("GG_FAKE_DRAFT", &self.draft)
            .env("GG_FAKE_UNAPPROVED", &self.unapproved)
            .env("GG_FAKE_APPROVAL_REGRESS", &self.approval_regress)
            .env("GG_FAKE_CI_CALLS", &self.ci_calls)
            .env("GG_FAKE_REGRESS", &self.regress)
            .env("GG_FAKE_MERGE_TRAINS", &self.merge_trains)
            .env("GG_FAKE_TRAIN_MERGED", &self.train_merged)
            .env("GG_FAKE_MERGE_SETTINGS_FAIL", &self.merge_settings_fail)
            .env("GG_FAKE_LANDING_REQUEST_FAIL", &self.landing_request_fail)
            .env("GG_FAKE_AUTH_NETWORK_FAIL", &self.auth_network_fail)
            .env("GG_FAKE_QUEUED", &self.queued)
            .env("GG_FAKE_RETARGET_FAIL", &self.retarget_fail)
            .env(
                "GG_FAKE_RETARGET_PIPELINE",
                self.repo_path.join("fake-retarget-pipeline"),
            )
            .env("GG_FAKE_CLEAN_PROVIDER_FAIL", &self.clean_provider_fail)
            .env("GG_FAKE_PR_VIEW_CALLS", &self.pr_view_calls)
            .env("GG_FAKE_GLAB_CI", &self.glab_ci)
            .env(
                "GG_FAKE_FIRST_TERMINAL_AFTER_REFRESH",
                &self.first_terminal_after_refresh,
            )
            .env("GG_FAKE_MR_SQUASH", &self.mr_squash)
            .env("GG_FAKE_SQUASH_DISABLED", &self.squash_disabled)
            .env("GG_FAKE_SQUASH_OPTION", &self.squash_option)
            .env("GG_FAKE_AUTO_MERGE_DROPPED", &self.auto_merge_dropped)
            .env("GG_FAKE_REQUEUED", &self.requeued)
            .env("GG_FAKE_HEAD_MOVED", &self.head_moved)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn start_land_with_args(&self, args: &[&str]) -> std::process::Child {
        self.land_command(args).spawn().expect("start waiting land")
    }

    fn wait_until_polling(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.polled.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(self.polled.exists(), "land never started polling CI");
    }

    fn wait_until_queued(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.queued.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(self.queued.exists(), "land never queued the MR");
    }

    fn wait_until_waiter_snapshots_queued_head(&self) {
        wait_for_file(&self.landing_request_fail.with_extension("waiter-polled"));
    }

    fn release_ci(&self) {
        fs::write(&self.ready, "ready\n").expect("release fake CI");
    }

    fn add_second_entry(&self) {
        fs::write(self.repo_path.join("second.txt"), "second\n").expect("write second entry");
        run_git(&self.repo_path, &["add", "second.txt"]);
        run_git(
            &self.repo_path,
            &["commit", "-m", "Second entry\n\nGG-ID: c-2222222"],
        );
        run_git(
            &self.repo_path,
            &["branch", "testuser/land-wait--c-2222222", "HEAD"],
        );
        run_git(
            &self.repo_path,
            &[
                "push",
                "origin",
                "testuser/land-wait--c-2222222:testuser/land-wait--c-2222222",
            ],
        );

        let config_path = self.repo_path.join(".git/gg/config.json");
        let mut config: Value =
            serde_json::from_slice(&fs::read(&config_path).expect("read config"))
                .expect("parse config");
        config["stacks"]["land-wait"]["mrs"]["c-2222222"] = Value::from(42);
        fs::write(
            &config_path,
            serde_json::to_vec_pretty(&config).expect("serialize config"),
        )
        .expect("write config");
    }

    fn add_unsynced_entry(&self) {
        fs::write(self.repo_path.join("unsynced.txt"), "unsynced\n").expect("write unsynced entry");
        run_git(&self.repo_path, &["add", "unsynced.txt"]);
        run_git(
            &self.repo_path,
            &["commit", "-m", "Unsynced entry\n\nGG-ID: c-2222222"],
        );
    }

    fn use_gitlab_provider(&self) {
        let config_path = self.repo_path.join(".git/gg/config.json");
        let mut config: Value =
            serde_json::from_slice(&fs::read(&config_path).expect("read config"))
                .expect("parse config");
        config["defaults"]["provider"] = Value::String("gitlab".to_string());
        fs::write(
            &config_path,
            serde_json::to_vec_pretty(&config).expect("serialize config"),
        )
        .expect("write config");
        write_fake_land_glab(&self.repo_path.join("fake-bin-land-wait").join("glab"));
    }

    fn enable_gitlab_merge_trains(&self) {
        fs::write(&self.merge_trains, "enabled\n").expect("enable fake merge trains");
    }

    fn complete_merge_train_status(&self) {
        fs::write(&self.train_merged, "merged\n").expect("complete fake merge train status");
    }

    fn fail_gitlab_merge_settings_lookup(&self) {
        fs::write(&self.merge_settings_fail, "fail\n").expect("fail fake merge settings lookup");
    }

    fn return_invalid_gitlab_merge_settings(&self) {
        fs::write(&self.merge_settings_fail, "invalid\n")
            .expect("return invalid fake merge settings");
    }

    fn omit_gitlab_squash_option(&self) {
        fs::write(&self.merge_settings_fail, "missing\n").expect("omit fake squash option");
    }

    fn fail_gitlab_landing_request(&self) {
        fs::write(&self.landing_request_fail, "fail\n").expect("fail fake landing request");
    }

    fn block_second_gitlab_landing_request(&self, succeeds: bool) {
        let mode = if succeeds {
            "block-second-success\n"
        } else {
            "block-second-fail\n"
        };
        fs::write(&self.landing_request_fail, mode).expect("block second fake landing request");
    }

    fn block_then_succeed_gitlab_landing_request(&self) {
        fs::write(&self.landing_request_fail, "block-success\n")
            .expect("block then succeed fake landing request");
    }

    fn landing_request_started(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(format!("{}.started", self.landing_request_fail.display()))
    }

    fn release_landing_request(&self) {
        fs::write(
            format!("{}.release", self.landing_request_fail.display()),
            "release\n",
        )
        .expect("release fake landing request");
    }

    fn fail_gitlab_squash_update_response(&self) {
        fs::write(&self.landing_request_fail, "squash-response-fail\n")
            .expect("fail fake squash update response");
    }

    fn set_glab_pipeline_status(&self, status: &str) {
        fs::write(&self.glab_ci, status).expect("set fake GitLab pipeline status");
    }

    fn queued_request(&self) -> String {
        fs::read_to_string(&self.queued).expect("read fake merge-train request")
    }

    fn fail_auth_with_network_error(&self) {
        fs::write(&self.auth_network_fail, "fail\n").expect("enable fake auth failure");
    }

    fn fail_retargeting(&self) {
        fs::write(&self.retarget_fail, "fail\n").expect("enable fake retarget failure");
    }

    fn fail_cleanup_provider_after_first_view(&self) {
        fs::write(&self.clean_provider_fail, "fail\n")
            .expect("enable fake cleanup provider failure");
    }

    fn make_first_entry_terminal_after_refresh(&self) {
        fs::write(&self.first_terminal_after_refresh, "enabled\n")
            .expect("enable fake terminal race");
    }

    fn mark_closed(&self) {
        fs::write(&self.closed, "closed\n").expect("mark fake PR closed");
    }

    fn mark_draft(&self) {
        fs::write(&self.draft, "draft\n").expect("mark fake PR draft");
    }

    fn mark_unapproved(&self) {
        fs::write(&self.unapproved, "unapproved\n").expect("mark fake PR unapproved");
    }

    fn fail_downstream_push(&self) {
        fs::write(
            self.repo_path.join("fake-bin-land-wait").join("git"),
            r#"#!/bin/sh
if [ "$1" = "push" ] && [ "$2" = "--force-with-lease" ]; then
  echo "force-with-lease rejected" >&2
  exit 1
fi
exec /usr/bin/git "$@"
"#,
        )
        .expect("write fake git");
        let path = self.repo_path.join("fake-bin-land-wait").join("git");
        let mut permissions = fs::metadata(&path).expect("stat fake git").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("make fake git executable");
    }
}

#[cfg(unix)]
fn land_operation_records(repo_path: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(repo_path.join(".git/gg/operations")) else {
        return Vec::new();
    };
    let mut records: Vec<Value> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("json")
        })
        .map(|entry| {
            serde_json::from_slice(&fs::read(entry.path()).expect("read operation record"))
                .expect("parse operation record")
        })
        .filter(|record: &Value| record["kind"] == "land")
        .collect();
    records.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    records
}

#[cfg(unix)]
const PIPELINE_LAND_ARGS: [&str; 8] = [
    "land",
    "--until",
    "2",
    "--wait",
    "--jsonl",
    "--no-clean",
    "--no-squash",
    "--rebase-if-needed",
];

#[cfg(unix)]
fn gitlab_landing_attempt(repo: &Path) -> Option<Value> {
    let config: Value =
        serde_json::from_slice(&fs::read(repo.join(".git/gg/config.json")).unwrap()).unwrap();
    config["stacks"]["land-wait"]["gitlab_landing_attempt"]
        .as_object()
        .cloned()
        .map(Value::Object)
}

#[cfg(unix)]
fn pipeline_operation(repo: &Path) -> (std::path::PathBuf, Value) {
    let record = land_operation_records(repo)
        .into_iter()
        .find(|record| {
            record["args"]
                .as_array()
                .is_some_and(|args| args.iter().any(|arg| arg == "gitlab:pipeline_start"))
        })
        .expect("pipeline operation");
    let path = repo.join(format!(
        ".git/gg/operations/{}.json",
        record["id"].as_str().unwrap()
    ));
    (path, record)
}

#[cfg(unix)]
fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(path.exists(), "timed out waiting for {}", path.display());
}

#[cfg(unix)]
fn pause_requeue_after_operation_creation(
    fixture: &WaitingLandFixture,
    args: &[&str],
) -> (std::process::Child, String, std::path::PathBuf) {
    let ready = fixture.repo_path.join("requeue-operation-ready");
    let release = fixture.repo_path.join("requeue-operation-release");
    let child = fixture
        .land_command(args)
        .env("GG_TEST_REQUEUE_OPERATION_READY", &ready)
        .env("GG_TEST_REQUEUE_OPERATION_RELEASE", &release)
        .spawn()
        .expect("start Land at the requeue operation boundary");
    fixture.wait_until_queued();
    fs::write(&fixture.head_moved, "moved\n").expect("move fake MR head");
    fs::write(&fixture.auto_merge_dropped, "dropped\n").expect("drop fake auto-merge");
    wait_for_file(&ready);
    let operation_id = fs::read_to_string(ready).expect("read requeue operation ID");
    (child, operation_id, release)
}

#[cfg(unix)]
fn assert_requeue_restart_proceeds(fixture: &WaitingLandFixture, args: &[&str]) {
    let restart = fixture.land_command(args).output().expect("restart Land");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&restart.stdout),
        String::from_utf8_lossy(&restart.stderr)
    );
    assert!(restart.status.success(), "restart failed: {text}");
    assert!(!text.contains("Inconsistent merge-train recovery evidence"));
    assert!(
        fixture.requeued.exists(),
        "restart never reached the requeue POST"
    );
}

#[cfg(unix)]
fn completed_pipeline_fixture() -> (WaitingLandFixture, std::process::Output) {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();
    let state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&state, "applied-response-block-success").unwrap();
    let mut command = fixture.land_command(&PIPELINE_LAND_ARGS);
    let mut guard = ProcessGroupGuard::spawn(&mut command).unwrap();
    let mut reader = BufReader::new(guard.take_stdout().expect("pipeline stdout"));
    let mut start = String::new();
    reader.read_line(&mut start).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&start).unwrap()["event"],
        "start"
    );
    wait_for_file(&std::path::PathBuf::from(format!(
        "{}.started",
        state.display()
    )));
    drop(reader);
    fixture.release_landing_request();
    let output = guard
        .wait_with_output_timeout(Duration::from_secs(30))
        .unwrap();
    (fixture, output)
}

#[cfg(unix)]
fn repoint_completed_pipeline_handoff_to_first_entry(fixture: &WaitingLandFixture) {
    let config_path = fixture.repo_path.join(".git/gg/config.json");
    let (operation_path, mut operation) = pipeline_operation(&fixture.repo_path);
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    let attempt = &mut config["stacks"]["land-wait"]["gitlab_landing_attempt"];
    attempt["mr_number"] = 41.into();
    attempt["source_branch"] = "testuser/land-wait--c-1111111".into();
    attempt["source_oid"] = "1111111111111111111111111111111111111111".into();
    config["stacks"]["land-wait"]["mrs"]["c-1111111"] = 41.into();
    for marker in operation["args"].as_array_mut().unwrap() {
        let Some(value) = marker.as_str() else {
            continue;
        };
        if value.starts_with("gitlab:mr=") {
            *marker = "gitlab:mr=41".into();
        } else if value.starts_with("gitlab:source_branch=") {
            *marker = "gitlab:source_branch=testuser/land-wait--c-1111111".into();
        } else if value.starts_with("gitlab:source_oid=") {
            *marker = "gitlab:source_oid=1111111111111111111111111111111111111111".into();
        }
    }
    fs::write(config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    fs::write(
        operation_path,
        serde_json::to_vec_pretty(&operation).unwrap(),
    )
    .unwrap();
    let state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&state, "missing").unwrap();
    fs::remove_file(format!("{}.started", state.display())).unwrap();
}

#[cfg(unix)]
#[test]
fn test_land_provider_preflight_failure_leaves_no_operation() {
    let fixture = WaitingLandFixture::new();
    let empty_path = fixture.repo_path.join("empty-path");
    fs::create_dir(&empty_path).expect("create empty PATH directory");

    let output = fixture
        .land_command(&["land", "--json", "--no-clean"])
        .env("PATH", empty_path)
        .output()
        .expect("run land without provider CLI");

    assert!(!output.status.success(), "provider preflight must fail");
    assert!(
        land_operation_records(&fixture.repo_path).is_empty(),
        "provider preflight failure must not create a Land operation"
    );
    assert!(!fixture.merged.exists(), "provider must not be mutated");
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_disconnect_before_start_leaves_no_operation() {
    let fixture = WaitingLandFixture::new();
    let (read_stream, write_stream) =
        std::os::unix::net::UnixStream::pair().expect("create disconnected JSONL stream");
    drop(read_stream);
    let stdout_fd: std::os::fd::OwnedFd = write_stream.into();

    let output = fixture
        .land_command(&["land", "--jsonl", "--no-clean"])
        .stdout(Stdio::from(stdout_fd))
        .spawn()
        .expect("start land")
        .wait_with_output()
        .expect("wait for land");

    assert!(
        !output.status.success(),
        "closed JSONL stream must abort land"
    );
    assert!(
        land_operation_records(&fixture.repo_path).is_empty(),
        "a failed start event must not create a Land operation"
    );
    assert!(!fixture.merged.exists(), "provider must not be mutated");
}

#[cfg(unix)]
#[test]
fn test_land_surfaces_exact_effect_save_failure_before_success() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    let save_block = fixture.repo_path.join("fake-effect-save-block");

    let output = fixture
        .land_command(&["land", "--json", "--no-clean"])
        .env("GG_FAKE_EFFECT_SAVE_BLOCK", &save_block)
        .output()
        .expect("run land with blocked exact-effect save");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(fixture.merged.exists(), "fake provider must merge the PR");
    assert!(
        !output.status.success(),
        "an exact-effect save failure must fail land: stdout={stdout} stderr={stderr}"
    );
    assert!(
        !std::path::PathBuf::from(format!("{}.released", save_block.display())).exists(),
        "land must stop before the later provider call can hide the save failure"
    );
    let payload: Value = serde_json::from_str(&stdout).expect("parse structured error payload");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|error| error.contains("durably record merge")),
        "land must report the exact-effect durability failure without a success result: {payload}"
    );

    let pending: Vec<Value> = land_operation_records(&fixture.repo_path)
        .into_iter()
        .filter(|record| record["status"] == "pending")
        .collect();
    assert_eq!(pending.len(), 1, "expected the uncertain Land operation");
    assert_eq!(pending[0]["touched_remote"], true);

    if let Ok(blocked_path) = fs::read_to_string(&save_block) {
        let _ = fs::remove_dir(blocked_path.trim());
    }
}

#[cfg(unix)]
fn disconnect_jsonl_after_landing_request_started(
    fixture: &WaitingLandFixture,
    args: &[&str],
) -> std::process::Output {
    let mut land = fixture.start_land_with_args(args);
    let stdout = land.stdout.take().expect("capture land stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read land start event");
    let start: Value = serde_json::from_str(line.trim()).expect("parse land start event");
    assert_eq!(start["event"], "start");

    let deadline = Instant::now() + Duration::from_secs(60);
    let request_started = fixture.landing_request_started();
    while !request_started.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    if !request_started.exists() {
        let _ = land.kill();
        panic!("final landing request never started");
    }

    drop(reader);
    fixture.release_landing_request();
    land.wait_with_output().expect("wait for land")
}

#[cfg(unix)]
fn disconnect_jsonl_and_capture_pending_landing(
    fixture: &WaitingLandFixture,
    args: &[&str],
) -> (std::process::Output, String) {
    let mut land = fixture.start_land_with_args(args);
    let stdout = land.stdout.take().expect("capture land stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read land start event");
    let start: Value = serde_json::from_str(line.trim()).expect("parse land start event");
    assert_eq!(start["event"], "start");

    let deadline = Instant::now() + Duration::from_secs(60);
    let request_started = fixture.landing_request_started();
    while !request_started.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    if !request_started.exists() {
        let _ = land.kill();
        panic!("final landing request never started");
    }

    let pending: Vec<Value> = land_operation_records(&fixture.repo_path)
        .into_iter()
        .filter(|record| record["status"] == "pending")
        .collect();
    assert_eq!(
        pending.len(),
        1,
        "the blocked request must own one pending operation: {pending:?}"
    );
    let operation_id = pending[0]["id"]
        .as_str()
        .expect("pending operation id")
        .to_string();
    assert_eq!(
        pending[0]["touched_remote"], true,
        "remote uncertainty must be persisted before the blocked request"
    );

    drop(reader);
    fixture.release_landing_request();
    let output = land.wait_with_output().expect("wait for land");
    (output, operation_id)
}

#[cfg(unix)]
fn run_blocked_landing_and_capture_pending(
    fixture: &WaitingLandFixture,
    args: &[&str],
) -> (std::process::Output, String) {
    let mut land = fixture.start_land_with_args(args);
    let deadline = Instant::now() + Duration::from_secs(60);
    let request_started = fixture.landing_request_started();
    while !request_started.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    if !request_started.exists() {
        let _ = land.kill();
        let output = land.wait_with_output().expect("collect timed-out land");
        panic!(
            "landing request never started: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let pending: Vec<Value> = land_operation_records(&fixture.repo_path)
        .into_iter()
        .filter(|record| record["status"] == "pending")
        .collect();
    assert_eq!(
        pending.len(),
        1,
        "expected one pending segment: {pending:?}"
    );
    let operation_id = pending[0]["id"]
        .as_str()
        .expect("pending operation id")
        .to_string();
    assert_eq!(pending[0]["touched_remote"], true);

    fixture.release_landing_request();
    let output = land.wait_with_output().expect("wait for land");
    (output, operation_id)
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_streams_wait_entry_and_summary() {
    let fixture = WaitingLandFixture::new();
    let mut land = fixture.start_land_with_args(&["land", "--wait", "--no-clean", "--jsonl"]);
    let stdout = land.stdout.take().expect("capture land stdout");
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut events = Vec::new();
        loop {
            let mut line = String::new();
            assert!(
                reader.read_line(&mut line).expect("read JSONL event") > 0,
                "land exited before emitting a wait heartbeat"
            );
            let event: Value = serde_json::from_str(line.trim_end()).expect("parse JSONL event");
            let is_wait = event["event"] == "wait";
            events.push(event);
            if is_wait {
                sender.send((reader, events)).expect("return JSONL reader");
                break;
            }
        }
    });
    let (mut reader, mut events) = match receiver.recv_timeout(Duration::from_secs(20)) {
        Ok(result) => result,
        Err(error) => {
            let _ = land.kill();
            panic!("land did not flush a wait heartbeat: {error}");
        }
    };

    // The command cannot finish until this file exists, so observing the wait
    // event first proves StreamingJson flushed the heartbeat immediately.
    fixture.release_ci();
    let mut remaining_stdout = String::new();
    reader
        .read_to_string(&mut remaining_stdout)
        .expect("read remaining JSONL events");

    let output = land.wait_with_output().expect("wait for land");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(
        output.status.success(),
        "land --jsonl failed: stdout={remaining_stdout} stderr={stderr}"
    );
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    events.extend(
        remaining_stdout
            .lines()
            .map(|line| serde_json::from_str(line).expect("every JSONL line must parse")),
    );
    assert_eq!(events.first().unwrap()["event"], "start");
    assert_eq!(events.first().unwrap()["total_entries"], 1);

    let wait = events
        .iter()
        .find(|event| event["event"] == "wait")
        .expect("pending CI should emit a wait heartbeat");
    assert_eq!(wait["phase"], "readiness");
    assert_eq!(wait["position"], 1);
    assert_eq!(wait["pr_number"], 41);
    assert_eq!(wait["poll"], 1);
    assert_eq!(wait["ci_status"], "pending");
    assert_eq!(wait["approved"], true);
    assert!(wait["elapsed_seconds"].is_number());

    let entry = events
        .iter()
        .find(|event| event["event"] == "entry")
        .expect("merge should emit an entry event");
    assert_eq!(entry["action"], "merged");
    assert_eq!(entry["pr_number"], 41);

    let summary = events.last().unwrap();
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["landed"][0]["action"], "merged");
    assert_eq!(summary["remaining"], 0);
}

#[cfg(unix)]
#[test]
fn test_land_all_wait_rechecks_approval_before_direct_merge() {
    let fixture = WaitingLandFixture::new();
    let land = fixture.start_land_with_args(&["land", "--all", "--wait", "--jsonl", "--no-clean"]);
    fixture.wait_until_polling();
    fixture.mark_unapproved();
    fixture.release_ci();

    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");

    assert!(!fixture.merged.exists(), "merge marker exists: {summary}");
    assert_eq!(summary["error"], "PR #41 is not approved", "{summary}");
    assert!(
        land_operation_records(&fixture.repo_path).is_empty(),
        "approval refusal must not create a Land operation"
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_admin_emits_no_human_warning() {
    let fixture = WaitingLandFixture::new();
    fixture.release_ci();

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()),
        "every JSONL line must parse: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_index_lock_wait_emits_no_human_warning() {
    let fixture = WaitingLandFixture::new();
    fixture.release_ci();
    let index_lock = fixture.repo_path.join(".git/index.lock");
    fs::write(&index_lock, "locked\n").expect("create index lock");

    let land = fixture.start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"]);
    std::thread::sleep(Duration::from_millis(250));
    fs::remove_file(&index_lock).expect("release index lock");

    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()),
        "every JSONL line must parse: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_default_scope_reports_one_total_entry() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    fixture.release_ci();

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");

    let start: Value =
        serde_json::from_str(stdout.lines().next().expect("start event")).expect("parse start");
    assert_eq!(start["event"], "start");
    assert_eq!(start["total_entries"], 1);

    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["remaining"], 1);
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_summary_reports_unsynced_remaining_entry() {
    let fixture = WaitingLandFixture::new();
    fixture.add_unsynced_entry();
    fixture.release_ci();

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["status"], "warning");
    assert_eq!(summary["remaining"], 1);
    assert!(summary["warnings"]
        .as_array()
        .expect("warnings must be an array")
        .iter()
        .any(|warning| warning
            .as_str()
            .is_some_and(|warning| warning.contains("gg sync"))));
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_default_scope_counts_terminal_prefix_entries() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    fs::write(&fixture.merged, "already merged\n").expect("mark fake PR merged");

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("parse JSONL event"))
        .collect::<Vec<_>>();
    assert_eq!(events.first().unwrap()["event"], "start");
    assert_eq!(events.first().unwrap()["total_entries"], 2);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event"] == "entry")
            .count(),
        2
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_default_scope_uses_refreshed_state_for_selection() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    fixture.make_first_entry_terminal_after_refresh();

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("parse JSONL event"))
        .collect::<Vec<_>>();
    let entries = events
        .iter()
        .filter(|event| event["event"] == "entry")
        .collect::<Vec<_>>();
    assert_eq!(events.first().unwrap()["total_entries"], 1);
    assert_eq!(
        entries.len(),
        1,
        "default land emitted too many entries: {stdout}"
    );
    assert_eq!(entries[0]["action"], "merged");
    assert_eq!(entries[0]["pr_number"], 41);
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_unapproved_entry_emits_error_outcome() {
    let fixture = WaitingLandFixture::new();
    fixture.mark_unapproved();

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("parse JSONL event"))
        .collect::<Vec<_>>();
    let entry = events
        .iter()
        .find(|event| event["event"] == "entry")
        .expect("unapproved PR should emit an entry outcome");
    assert_eq!(entry["status"], "error");
    assert_eq!(entry["action"], "error");
    assert!(entry["error"]
        .as_str()
        .is_some_and(|error| error.contains("is not approved")));
    assert_eq!(events.last().unwrap()["status"], "error");
}

#[cfg(unix)]
#[test]
fn test_land_all_rechecks_approval_before_each_remote_mutation() {
    for (variant, merge_train, extra_args) in [
        ("merge train", true, &[][..]),
        ("auto-merge", false, &["--auto-merge"][..]),
        ("direct merge with GitLab admin", false, &["--admin"][..]),
    ] {
        let fixture = WaitingLandFixture::new();
        fixture.use_gitlab_provider();
        if merge_train {
            fixture.enable_gitlab_merge_trains();
        }
        fs::write(&fixture.approval_regress, "1\n").expect("regress approval");

        let mut args = vec!["land", "--all", "--jsonl", "--no-clean"];
        args.extend_from_slice(extra_args);
        let output = fixture
            .start_land_with_args(&args)
            .wait_with_output()
            .expect("wait for land");
        let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
        let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
        assert!(output.status.success(), "{variant}: stderr={stderr}");
        assert!(
            stderr.trim().is_empty(),
            "{variant}: unexpected stderr: {stderr}"
        );

        let events = stdout
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("parse JSONL event"))
            .collect::<Vec<_>>();
        let entry = events
            .iter()
            .find(|event| event["event"] == "entry")
            .expect("approval regression should emit an entry");
        assert_eq!(entry["action"], "error", "{variant}: {stdout}");
        assert!(
            entry["error"]
                .as_str()
                .is_some_and(|error| error.contains("is not approved")),
            "{variant}: {stdout}"
        );
        assert_eq!(events.last().unwrap()["status"], "error", "{variant}");
        assert_eq!(
            fs::read_to_string(format!("{}.calls", fixture.approval_regress.display()))
                .expect("approval call count")
                .trim(),
            "2",
            "{variant}: initial refresh and final seam must each check approval"
        );
        assert!(
            !fixture.queued.exists(),
            "{variant}: remote queue was mutated"
        );
        assert!(
            !fixture.merged.exists(),
            "{variant}: remote merge was mutated"
        );
        assert!(
            land_operation_records(&fixture.repo_path).is_empty(),
            "{variant}: approval refusal must not touch a Land operation"
        );
    }
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_auto_merge_all_wait_rechecks_approval_before_queue() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fs::write(&fixture.approval_regress, "1\n").expect("regress second approval lookup");

    let output = fixture
        .land_command(&[
            "land",
            "--all",
            "--wait",
            "--auto-merge",
            "--jsonl",
            "--no-clean",
        ])
        .output()
        .expect("run land");
    let stdout = String::from_utf8(output.stdout).expect("land stdout");
    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");

    assert!(!fixture.queued.exists(), "queued marker exists: {summary}");
    assert_eq!(summary["error"], "MR !41 is not approved", "{summary}");
    assert_eq!(
        fs::read_to_string(format!("{}.calls", fixture.approval_regress.display()))
            .expect("approval call count")
            .trim(),
        "2"
    );
    assert!(land_operation_records(&fixture.repo_path).is_empty());
    assert!(gitlab_landing_attempt(&fixture.repo_path).is_none());
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_merge_train_wait_rechecks_approval_after_validation_before_initial_queue() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fs::write(&fixture.approval_regress, "after-validation\n").expect("regress approval");
    fs::write(
        &fixture.landing_request_fail,
        "merge-train-applied-response-fail\n",
    )
    .expect("terminate a would-be queue request");

    let output = fixture
        .land_command(&["land", "--wait", "--jsonl", "--no-clean"])
        .output()
        .expect("run land");
    let stdout = String::from_utf8(output.stdout).expect("land stdout");
    let stderr = String::from_utf8(output.stderr).expect("land stderr");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary", "{summary}");
    assert_eq!(summary["status"], "error", "{summary}");
    let config: Value = serde_json::from_slice(
        &fs::read(fixture.repo_path.join(".git/gg/config.json")).expect("read config"),
    )
    .expect("parse config");
    let stack = &config["stacks"]["land-wait"];

    assert_eq!(
        serde_json::json!({
            "error": summary["error"],
            "queue_post_marker": fixture.queued.exists(),
            "queue_post_attempt": std::path::PathBuf::from(format!("{}.attempts", fixture.queued.display())).exists(),
            "gitlab_landing_attempt": stack["gitlab_landing_attempt"],
            "gitlab_merge_train_queue_attempt": stack["gitlab_merge_train_queue_attempt"],
            "land_operation_records": land_operation_records(&fixture.repo_path),
            "validation_order": fs::read_to_string(format!("{}.order", fixture.approval_regress.display())).expect("validation order"),
            "validation_reads": fs::read_to_string(format!("{}.raw-reads", fixture.approval_regress.display())).expect("validation reads"),
        }),
        serde_json::json!({
            "error": "MR !41 is not approved",
            "queue_post_marker": false,
            "queue_post_attempt": false,
            "gitlab_landing_attempt": null,
            "gitlab_merge_train_queue_attempt": null,
            "land_operation_records": [],
            "validation_order": "approval:0\napproval:0\napproval:0\nvalidation:1\nvalidation:2\nvalidation:3\napproval:3\n",
            "validation_reads": "3\n",
        })
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_draft_entry_emits_error_status() {
    let fixture = WaitingLandFixture::new();
    fixture.mark_draft();

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("parse JSONL event"))
        .collect::<Vec<_>>();
    let entry = events
        .iter()
        .find(|event| event["event"] == "entry")
        .expect("draft PR should emit an entry outcome");
    assert_eq!(entry["status"], "error");
    assert_eq!(entry["action"], "skipped_draft");
    assert!(entry["error"]
        .as_str()
        .is_some_and(|error| error.contains("is a draft")));
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_closed_entry_marks_warning_status() {
    let fixture = WaitingLandFixture::new();
    fixture.mark_closed();

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("parse JSONL event"))
        .collect::<Vec<_>>();
    let entry = events
        .iter()
        .find(|event| event["event"] == "entry")
        .expect("closed PR should emit an entry outcome");
    assert_eq!(entry["status"], "warning");
    assert_eq!(entry["action"], "skipped_closed");

    let summary = events.last().unwrap();
    assert_eq!(summary["status"], "warning");
    assert!(summary["warnings"]
        .as_array()
        .expect("warnings must be an array")
        .iter()
        .any(|warning| warning
            .as_str()
            .is_some_and(|warning| warning.contains("is closed"))));
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_reports_retarget_failure_warning() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    fixture.fail_retargeting();
    fixture.release_ci();

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert!(
        summary["warnings"]
            .as_array()
            .expect("warnings must be an array")
            .iter()
            .any(|warning| warning
                .as_str()
                .is_some_and(|warning| warning.contains("Failed to update PR #42 base"))),
        "summary should include retarget failure warning: {summary}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_no_rebase_skips_downstream_branch_rebase() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    fixture.release_ci();
    fixture.fail_downstream_push();

    let output = fixture
        .start_land_with_args(&[
            "land",
            "--all",
            "--jsonl",
            "--admin",
            "--no-clean",
            "--no-rebase",
        ])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");

    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert!(
        !summary["warnings"]
            .as_array()
            .expect("warnings must be an array")
            .iter()
            .any(|warning| warning
                .as_str()
                .is_some_and(|warning| warning.contains("Failed to push"))),
        "--no-rebase must not push downstream branches: {summary}"
    );
}

#[test]
fn test_gg_land_help_has_no_rebase() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success);
    assert!(stdout.contains("--no-rebase"));
}

#[test]
fn test_gg_land_help_has_rebase_if_needed() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success);
    assert!(stdout.contains("--rebase-if-needed"));
}

#[test]
fn test_gg_land_rejects_no_rebase_with_rebase_if_needed() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, _stdout, stderr) =
        run_gg(&repo_path, &["land", "--no-rebase", "--rebase-if-needed"]);

    assert!(
        !success,
        "--no-rebase and --rebase-if-needed should conflict"
    );
    assert!(stderr.contains("cannot be used with"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_reports_downstream_push_failure_warning() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    fixture.release_ci();
    fixture.fail_downstream_push();

    let output = fixture
        .start_land_with_args(&["land", "--all", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert!(
        summary["warnings"]
            .as_array()
            .expect("warnings must be an array")
            .iter()
            .any(|warning| warning.as_str().is_some_and(
                |warning| warning.contains("Failed to push testuser/land-wait--c-2222222")
            )),
        "summary should include downstream push failure warning: {summary}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_reports_provider_scan_failure() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    fixture.fail_cleanup_provider_after_first_view();
    fs::write(&fixture.merged, "already merged\n").expect("mark first PR merged");

    let output = fixture
        .start_land_with_args(&["land", "--all", "--jsonl", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["status"], "error");
    assert!(
        summary["error"]
            .as_str()
            .is_some_and(|error| error.contains("Failed to fetch PR #")),
        "summary should include provider scan failure: {summary}"
    );
    assert!(
        stdout.lines().any(|line| {
            let event: Value = serde_json::from_str(line).expect("parse JSONL event");
            event["event"] == "entry" && event["status"] == "error" && event["pr_number"] == 42
        }),
        "provider scan failure should emit an entry outcome: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_jsonl_admin_emits_no_human_warning() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();

    let output = fixture
        .start_land_with_args(&["land", "--all", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(
        fixture.merged.with_extension("glab").exists(),
        "fake GitLab MR should be merged"
    );
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()),
        "every JSONL line must parse: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_auto_merge_jsonl_reports_single_entry_total() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.add_second_entry();

    let output = fixture
        .start_land_with_args(&["land", "--all", "--jsonl", "--auto-merge", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let start: Value =
        serde_json::from_str(stdout.lines().next().expect("start event")).expect("parse start");
    assert_eq!(start["event"], "start");
    assert_eq!(start["total_entries"], 1);

    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["remaining"], 2);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_merge_train_jsonl_reports_single_entry_total_without_wait() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();

    let output = fixture
        .start_land_with_args(&["land", "--all", "--jsonl", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let start: Value =
        serde_json::from_str(stdout.lines().next().expect("start event")).expect("parse start");
    assert_eq!(start["event"], "start");
    assert_eq!(start["total_entries"], 1);

    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["remaining"], 2);
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_clean_silences_provider_lookup_debug() {
    let fixture = WaitingLandFixture::new();
    fs::write(&fixture.merged, "already merged\n").expect("mark fake PR merged");
    fixture.fail_cleanup_provider_after_first_view();

    let output = fixture
        .start_land_with_args(&["land", "--all", "--jsonl", "--clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()),
        "every JSONL line must parse: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_jsonl_network_auth_fallback_emits_no_human_warning() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.fail_auth_with_network_error();

    let output = fixture
        .start_land_with_args(&["land", "--all", "--jsonl", "--admin", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()),
        "every JSONL line must parse: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_jsonl_records_merge_train_merge_before_stale_stack_error() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();

    let land = fixture.start_land_with_args(&["land", "--all", "--wait", "--jsonl", "--no-clean"]);
    fixture.wait_until_queued();
    fixture.wait_until_polling();

    fs::write(fixture.repo_path.join("new.txt"), "new\n").expect("write new entry");
    run_git(&fixture.repo_path, &["add", "new.txt"]);
    run_git(
        &fixture.repo_path,
        &["commit", "-m", "New entry\n\nGG-ID: c-2222222"],
    );
    fs::write(&fixture.merged, "merged\n").expect("complete fake merge train");

    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(
        output.status.success(),
        "structured land reports the error in JSONL: stderr={stderr}"
    );
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");

    let events: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("every JSONL line must parse"))
        .collect();
    assert!(
        events.iter().any(|event| event["event"] == "entry"
            && event["pr_number"] == 41
            && event["action"] == "merged"),
        "merged entry should be emitted before stale-stack error: {stdout}"
    );
    let summary = events.last().expect("summary event");
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["landed"][0]["action"], "merged");
    assert_eq!(summary["remaining"], 0);
    assert!(
        summary["error"]
            .as_str()
            .is_some_and(|error| error.contains("Stack changed while gg land was waiting")),
        "summary should still report the stale-stack error: {summary}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_merge_train_wait_queues_auto_merge_before_pipeline_passes() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");

    let land = fixture.start_land_with_args(&["land", "--all", "--wait", "--jsonl", "--no-clean"]);
    fixture.wait_until_queued();
    let request = fixture.queued_request();
    assert!(
        request.contains("auto_merge=true"),
        "merge-train request must ask GitLab to add the MR when checks pass: {request}"
    );
    assert!(
        request.contains("squash=true"),
        "merge-train request must forward the default squash: {request}"
    );

    fs::write(&fixture.merged, "merged\n").expect("complete fake merge train");
    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["landed"][0]["action"], "merged", "{summary}");
    assert!(summary["error"].is_null(), "{summary}");
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_merge_train_wait_all_persists_cleanup_between_entries() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();
    fixture.set_glab_pipeline_status("running");

    let land = fixture.start_land_with_args(&["land", "--all", "--wait", "--jsonl", "--no-clean"]);
    fixture.wait_until_queued();
    fs::write(&fixture.merged, "merged\n").expect("complete fake merge train");

    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    let second_request = fixture.queued_request();
    assert!(
        second_request.contains("merge_trains/merge_requests/42"),
        "the continuation must submit a landing request for the second MR: {second_request}\nstdout: {stdout}\nstderr: {stderr}"
    );

    let config: Value = serde_json::from_slice(
        &fs::read(fixture.repo_path.join(".git/gg/config.json")).expect("read persisted config"),
    )
    .expect("parse persisted config");
    assert!(
        config["stacks"]["land-wait"]["mrs"]
            .get("c-1111111")
            .is_none(),
        "first merged MR mapping must be persisted before continuing: {config}"
    );
    assert!(
        config["stacks"]["land-wait"]["mrs"]
            .get("c-2222222")
            .is_none(),
        "second merged MR mapping must be removed after its own landing: {config}"
    );

    let events: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse JSONL event"))
        .collect();
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event"] == "entry" && event["action"] == "merged")
            .count(),
        2,
        "both merge-train landings must emit a merged entry: {stdout}"
    );

    let records = land_operation_records(&fixture.repo_path);
    assert!(
        records.len() >= 2,
        "expected multiple operation segments: {records:?}"
    );
    assert!(
        records.iter().all(|record| record["status"] == "committed"),
        "all continuation segments must be finalized: {records:?}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_starts_missing_pipeline_after_retarget_without_rebasing() {
    for pipeline in ["missing", "running"] {
        let fixture = WaitingLandFixture::new();
        fixture.use_gitlab_provider();
        fixture.enable_gitlab_merge_trains();
        fixture.add_second_entry();
        fs::write(fixture.repo_path.join("fake-retarget-pipeline"), pipeline)
            .expect("set pipeline behavior after the retarget");
        let (ok, original_head) = run_git(&fixture.repo_path, &["rev-parse", "HEAD"]);
        assert!(ok);

        let output = fixture
            .start_land_with_args(&[
                "land",
                "--until",
                "2",
                "--wait",
                "--jsonl",
                "--no-clean",
                "--no-squash",
                "--rebase-if-needed",
            ])
            .wait_with_output()
            .expect("wait for land");
        let stdout = String::from_utf8(output.stdout).expect("land stdout");
        let stderr = String::from_utf8(output.stderr).expect("land stderr");
        let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
            .expect("parse summary event");
        assert!(output.status.success(), "{pipeline}: {summary}\n{stderr}");
        assert_eq!(summary["landed"][1]["pr_number"], 42, "{summary}");
        assert_eq!(summary["landed"][1]["action"], "merged", "{summary}");
        let (ok, downstream_head) = run_git(
            &fixture.repo_path,
            &["rev-parse", "testuser/land-wait--c-2222222"],
        );
        assert!(ok);
        assert_eq!(downstream_head, original_head, "downstream MR was rebased");
    }
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_refuses_unknown_draft_state_before_starting_pipeline() {
    for pipeline in ["missing-draft", "missing-work-in-progress"] {
        let fixture = WaitingLandFixture::new();
        fixture.use_gitlab_provider();
        fixture.enable_gitlab_merge_trains();
        fixture.add_second_entry();
        let pipeline_state = fixture.repo_path.join("fake-retarget-pipeline");
        fs::write(&pipeline_state, pipeline).expect("set incomplete MR response");

        let output = fixture
            .start_land_with_args(&[
                "land",
                "--until",
                "2",
                "--wait",
                "--jsonl",
                "--no-clean",
                "--no-squash",
                "--rebase-if-needed",
            ])
            .wait_with_output()
            .expect("wait for land");
        let stdout = String::from_utf8(output.stdout).expect("land stdout");
        let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
            .expect("parse summary event");
        let started = std::path::PathBuf::from(format!("{}.started", pipeline_state.display()));

        assert_eq!(summary["status"], "error", "{pipeline}: {summary}");
        assert!(
            summary["error"]
                .as_str()
                .or_else(|| summary["message"].as_str())
                .is_some_and(|error| error.contains("Failed to parse MR !42")),
            "{pipeline}: {summary}"
        );
        assert!(
            !started.exists(),
            "{pipeline}: pipeline must not be started"
        );
        assert!(
            !fixture.queued_request().contains("merge_requests/42"),
            "{pipeline}: MR !42 must not be queued"
        );
    }
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_revalidates_draft_state_before_every_remote_mutation_path() {
    for mode in ["direct", "merge-train", "no-squash"] {
        let fixture = WaitingLandFixture::new();
        fixture.use_gitlab_provider();
        let landing_state = fixture.repo_path.join("fake-retarget-pipeline");
        fs::write(&landing_state, "missing-draft-on-landing")
            .expect("omit draft state during the final pre-mutation lookup");

        if mode == "merge-train" {
            fixture.enable_gitlab_merge_trains();
        }
        if mode == "no-squash" {
            fs::write(&fixture.squash_option, "default_on").expect("set fake squash option");
            fs::write(&fixture.mr_squash, "on\n").expect("enable fake MR squash");
        }
        let args = if mode == "no-squash" {
            vec!["land", "--json", "--no-clean", "--no-squash"]
        } else {
            vec!["land", "--json", "--no-clean"]
        };

        let output = fixture
            .start_land_with_args(&args)
            .wait_with_output()
            .expect("wait for land");
        let stdout = String::from_utf8(output.stdout).expect("land stdout");
        let stderr = String::from_utf8(output.stderr).expect("land stderr");

        assert!(!output.status.success(), "{mode}: {stdout}");
        assert!(
            format!("{stdout}{stderr}").contains("missing field `draft`"),
            "{mode}: the failure must report unknown draft state: stdout={stdout} stderr={stderr}"
        );
        assert!(!fixture.queued.exists(), "{mode}: MR must not be queued");
        assert!(!fixture.merged.exists(), "{mode}: MR must not be merged");
        assert!(
            !fixture.squash_disabled.exists(),
            "{mode}: MR squash must not be changed"
        );
    }
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_rejects_unsafe_state_transitions_before_remote_mutation() {
    for (mode, expected_error) in [
        ("draft-on-landing", "draft or work in progress"),
        ("closed-on-landing", "is not open"),
        ("target-mismatch-on-landing", "targets 'other-base'"),
    ] {
        let fixture = WaitingLandFixture::new();
        fixture.use_gitlab_provider();
        fs::write(fixture.repo_path.join("fake-retarget-pipeline"), mode)
            .expect("change MR state during final pre-mutation lookup");

        let output = fixture
            .start_land_with_args(&["land", "--json", "--no-clean"])
            .wait_with_output()
            .expect("wait for land");
        let stdout = String::from_utf8(output.stdout).expect("land stdout");
        let stderr = String::from_utf8(output.stderr).expect("land stderr");

        assert!(!output.status.success(), "{mode}: {stdout}");
        assert!(
            format!("{stdout}{stderr}").contains(expected_error),
            "{mode}: stdout={stdout} stderr={stderr}"
        );
        assert!(!fixture.queued.exists(), "{mode}: MR must not be queued");
        assert!(!fixture.merged.exists(), "{mode}: MR must not be merged");
        assert!(
            !fixture.squash_disabled.exists(),
            "{mode}: MR squash must not be changed"
        );
    }
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_auto_merge_rejects_unknown_or_unsafe_state_before_mutation() {
    for (mode, expected_error) in [
        ("draft-on-landing", "draft or work in progress"),
        ("closed-on-landing", "is not open"),
        ("target-mismatch-on-landing", "targets 'other-base'"),
        ("missing-draft-on-landing", "missing field `draft`"),
        ("malformed-draft-on-landing", "invalid type"),
    ] {
        let fixture = WaitingLandFixture::new();
        fixture.use_gitlab_provider();
        fs::write(fixture.repo_path.join("fake-retarget-pipeline"), mode)
            .expect("change MR state during final pre-mutation lookup");

        let output = fixture
            .start_land_with_args(&["land", "--json", "--auto-merge", "--no-clean"])
            .wait_with_output()
            .expect("wait for land");
        let stdout = String::from_utf8(output.stdout).expect("land stdout");
        let stderr = String::from_utf8(output.stderr).expect("land stderr");

        assert!(!output.status.success(), "{mode}: {stdout}");
        assert!(
            format!("{stdout}{stderr}").contains(expected_error),
            "{mode}: stdout={stdout} stderr={stderr}"
        );
        assert!(!fixture.queued.exists(), "{mode}: MR must not be queued");
        assert!(
            land_operation_records(&fixture.repo_path).is_empty(),
            "{mode}: validation failure must not create an operation"
        );
    }
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_wait_validation_failure_leaves_no_pending_operation() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fs::write(
        fixture.repo_path.join("fake-retarget-pipeline"),
        "missing-draft-on-landing",
    )
    .expect("omit draft during the authoritative post-readiness lookup");

    let output = fixture
        .start_land_with_args(&["land", "--wait", "--jsonl", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("land stdout");
    assert!(
        stdout.contains("missing field `draft`"),
        "validation failure must be reported: {stdout}"
    );
    assert!(!fixture.queued.exists(), "MR must not be queued");
    assert!(
        land_operation_records(&fixture.repo_path).is_empty(),
        "post-readiness validation failure must not create a pending operation"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_no_squash_view_failure_leaves_no_pending_operation() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fs::write(&fixture.squash_option, "default_on").expect("set fake squash option");
    fs::write(&fixture.mr_squash, "on\n").expect("enable fake MR squash");
    fs::write(
        fixture.repo_path.join("fake-retarget-pipeline"),
        "malformed-squash-view",
    )
    .expect("malform the permissive squash-state lookup");

    let output = fixture
        .start_land_with_args(&["land", "--json", "--no-clean", "--no-squash"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("land stdout");
    let stderr = String::from_utf8(output.stderr).expect("land stderr");

    let summary: Value = serde_json::from_str(stdout.trim()).expect("land summary");
    assert!(
        summary["land"]["error"]
            .as_str()
            .or_else(|| summary["error"].as_str())
            .is_some_and(|error| error.contains("Failed to parse MR !41")),
        "{summary}"
    );
    assert!(format!("{stdout}{stderr}").contains("Failed to parse MR !41"));
    assert!(!fixture.squash_disabled.exists());
    assert!(!fixture.queued.exists());
    assert!(land_operation_records(&fixture.repo_path).is_empty());
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_requeue_revalidates_state_and_creates_no_pending_operation() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");

    let land = fixture.start_land_with_args(&["land", "--all", "--wait", "--jsonl", "--no-clean"]);
    fixture.wait_until_queued();
    fs::write(
        fixture.repo_path.join("fake-retarget-pipeline"),
        "draft-on-requeue",
    )
    .expect("make the MR draft before requeue");
    fs::write(&fixture.head_moved, "moved\n").expect("move fake MR head");
    fs::write(&fixture.auto_merge_dropped, "dropped\n").expect("drop fake auto-merge");

    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("land stdout");
    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert!(
        summary["error"]
            .as_str()
            .is_some_and(|error| error.contains("draft or work in progress")),
        "{summary}"
    );
    assert!(!fixture.requeued.exists(), "unsafe MR must not be requeued");
    let records = land_operation_records(&fixture.repo_path);
    assert_eq!(
        records.len(),
        1,
        "only the initial queue may be recorded: {records:?}"
    );
    assert_eq!(records[0]["status"], "committed");
    assert_eq!(records[0]["touched_remote"], true);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_merge_train_requeue_rechecks_approval_after_final_validation() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");
    fs::write(&fixture.approval_regress, "after-requeue-validation\n").unwrap();
    fs::write(&fixture.landing_request_fail, "requeue-merge\n").unwrap();
    let args = ["land", "--wait", "--admin", "--jsonl", "--no-clean"];
    let land = fixture.start_land_with_args(&args);
    fixture.wait_until_queued();
    fs::write(&fixture.head_moved, "moved\n").unwrap();
    fs::write(&fixture.auto_merge_dropped, "dropped\n").unwrap();
    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("land stdout");
    let stderr = String::from_utf8(output.stderr).expect("land stderr");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    let summary: Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
    let config_bytes = fs::read(fixture.repo_path.join(".git/gg/config.json")).unwrap();
    let config: Value = serde_json::from_slice(&config_bytes).unwrap();
    let records = land_operation_records(&fixture.repo_path);
    let post_attempt = format!("{}.attempts", fixture.landing_request_fail.display());
    assert!(
        !Path::new(&post_attempt).exists(),
        "requeue POST occurred: config={config} records={records:?}"
    );
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["status"], "error");
    assert_eq!(summary["error"], "MR !41 is not approved");
    assert!(!fixture.requeued.exists(), "requeue POST marker exists");
    assert!(config["stacks"]["land-wait"]["gitlab_merge_train_queue_attempt"].is_null());
    assert_eq!(records.len(), 1, "{records:?}");
    let args = records[0]["args"].as_array().expect("initial queue args");
    assert!(args.iter().any(|arg| arg == "gitlab:queue_merge_train"));
    assert!(!args.iter().any(|arg| arg == "gitlab:requeue_merge_train"));
    let order =
        fs::read_to_string(format!("{}.order", fixture.approval_regress.display())).unwrap();
    assert_eq!(order, "poll\nvalidation:1\napproval:1\n");
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_requeue_tracks_the_final_validated_head() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");
    fs::write(&fixture.landing_request_fail, "requeue-head-race\n").unwrap();

    let land = fixture.start_land_with_args(&["land", "--wait", "--jsonl", "--no-clean"]);
    fixture.wait_until_queued();
    fs::write(&fixture.head_moved, "moved\n").unwrap();
    fs::write(&fixture.auto_merge_dropped, "dropped\n").unwrap();

    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("land stdout");
    let summary: Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
    let attempts = fs::read_to_string(format!(
        "{}.attempts",
        fixture.landing_request_fail.display()
    ))
    .unwrap();
    let order =
        fs::read_to_string(format!("{}.order", fixture.landing_request_fail.display())).unwrap();

    assert_eq!(attempts.lines().count(), 1, "second requeue POST: {order}");
    assert_eq!(summary["status"], "error");
    assert!(summary["error"]
        .as_str()
        .is_some_and(|error| error.contains("auto-merge was cancelled without a new push")));
    assert_eq!(order, "poll:2222222222222222222222222222222222222222\nvalidation:3333333333333333333333333333333333333333\npost:3333333333333333333333333333333333333333\npoll:3333333333333333333333333333333333333333\n");
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_requeue_durability_boundaries_precede_post_and_allow_restart() {
    let args = ["land", "--all", "--wait", "--jsonl", "--no-clean"];
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");
    fs::write(&fixture.landing_request_fail, "requeue-merge\n").unwrap();

    let (land, operation_id, release) = pause_requeue_after_operation_creation(&fixture, &args);
    let operation_block = fixture
        .repo_path
        .join(format!(".git/gg/operations/{operation_id}.json.tmp"));
    fs::create_dir(&operation_block).expect("block touched_remote save");
    fs::write(release, "release\n").expect("release requeue operation");
    let output = land.wait_with_output().expect("wait for failed requeue");
    fs::remove_dir(operation_block).expect("restore operation store");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let records = land_operation_records(&fixture.repo_path);
    let record = records
        .iter()
        .find(|record| record["id"] == operation_id)
        .expect("requeue operation record");
    let config: Value = serde_json::from_slice(
        &fs::read(fixture.repo_path.join(".git/gg/config.json")).expect("read config"),
    )
    .expect("parse config");
    let attempt = &config["stacks"]["land-wait"]["gitlab_merge_train_queue_attempt"];
    let post_attempts = std::path::PathBuf::from(format!(
        "{}.attempts",
        fixture.landing_request_fail.display()
    ));
    assert!(
        attempt.is_null(),
        "stale requeue marker survived failed touched_remote barrier: attempt={attempt} touched_remote={} requeue_post={} post_attempt_log={} output={text}",
        record["touched_remote"],
        fixture.requeued.exists(),
        post_attempts.exists()
    );
    assert_eq!(record["status"], "pending", "{record}");
    assert_eq!(record["touched_remote"], false, "{record}");
    assert_eq!(record["remote_effects"], serde_json::json!([]));
    assert!(
        !fixture.requeued.exists(),
        "requeue POST escaped the barrier"
    );
    assert!(!post_attempts.exists(), "requeue POST reached the provider");
    assert_requeue_restart_proceeds(&fixture, &args);

    let marker_fixture = WaitingLandFixture::new();
    marker_fixture.use_gitlab_provider();
    marker_fixture.enable_gitlab_merge_trains();
    marker_fixture.set_glab_pipeline_status("running");
    fs::write(&marker_fixture.landing_request_fail, "requeue-merge\n").unwrap();
    let (land, operation_id, release) =
        pause_requeue_after_operation_creation(&marker_fixture, &args);
    let config_block = marker_fixture.repo_path.join(".git/gg/config.tmp");
    fs::create_dir(&config_block).expect("block requeue marker save");
    fs::write(release, "release\n").expect("release requeue operation");
    let output = land
        .wait_with_output()
        .expect("wait for failed marker save");
    fs::remove_dir(config_block).expect("restore config store");
    let records = land_operation_records(&marker_fixture.repo_path);
    let record = records
        .iter()
        .find(|record| record["id"] == operation_id)
        .expect("requeue operation record");
    let config: Value = serde_json::from_slice(
        &fs::read(marker_fixture.repo_path.join(".git/gg/config.json")).expect("read config"),
    )
    .expect("parse config");
    assert_eq!(record["touched_remote"], true, "{record}");
    assert!(config["stacks"]["land-wait"]["gitlab_merge_train_queue_attempt"].is_null());
    assert!(
        !marker_fixture.requeued.exists(),
        "requeue POST escaped marker save"
    );
    assert!(!format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .contains("Inconsistent merge-train recovery evidence"));
    assert_requeue_restart_proceeds(&marker_fixture, &args);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_requeue_checkpoint_failures_preserve_remote_outcome() {
    let args = ["land", "--all", "--wait", "--jsonl", "--no-clean"];
    for (mode, provider_succeeded, expected_status) in [
        ("requeue-success-finalization-fail", true, "pending"),
        ("requeue-fail-finalization-fail", false, "pending"),
        ("requeue-success-marker-clear-fail", true, "committed"),
    ] {
        let fixture = WaitingLandFixture::new();
        fixture.use_gitlab_provider();
        fixture.enable_gitlab_merge_trains();
        fixture.set_glab_pipeline_status("running");
        fs::write(&fixture.landing_request_fail, format!("{mode}\n")).unwrap();

        let mut land = fixture.start_land_with_args(&args);
        fixture.wait_until_queued();
        fixture.wait_until_waiter_snapshots_queued_head();
        fs::write(&fixture.head_moved, "moved\n").unwrap();
        fs::write(&fixture.auto_merge_dropped, "dropped\n").unwrap();
        let (stdout, stderr) = if mode == "requeue-success-marker-clear-fail" {
            let mut reader = BufReader::new(land.stdout.take().expect("land stdout pipe"));
            let mut stdout = String::new();
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0, "{stdout}");
                stdout.push_str(&line);
                if line.contains("marker-clear checkpoint failed") {
                    break;
                }
            }
            fs::remove_dir(fixture.repo_path.join(".git/gg/config.tmp"))
                .expect("restore config store after marker-clear failure");
            reader.read_to_string(&mut stdout).unwrap();
            let output = land.wait_with_output().expect("wait for Land");
            (
                stdout,
                String::from_utf8(output.stderr).expect("land stderr"),
            )
        } else {
            let output = land
                .wait_with_output()
                .expect("wait for requeue checkpoint failure");
            (
                String::from_utf8(output.stdout).expect("land stdout"),
                String::from_utf8(output.stderr).expect("land stderr"),
            )
        };
        let text = format!("{stdout}{stderr}");
        let summary: Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
        let error = summary["message"]
            .as_str()
            .or_else(|| summary["error"].as_str())
            .expect("terminal error message");

        let records = land_operation_records(&fixture.repo_path);
        let record = records
            .iter()
            .find(|record| {
                record["args"]
                    .as_array()
                    .is_some_and(|args| args.iter().any(|arg| arg == "gitlab:requeue_merge_train"))
            })
            .expect("requeue operation record");
        let operation_id = record["id"].as_str().expect("operation ID");
        let operation_tmp = fixture
            .repo_path
            .join(format!(".git/gg/operations/{operation_id}.json.tmp"));
        if operation_tmp.exists() {
            fs::remove_dir(&operation_tmp).expect("remove finalization blocker");
        }
        let config_tmp = fixture.repo_path.join(".git/gg/config.tmp");
        if config_tmp.exists() {
            fs::remove_dir(&config_tmp).expect("remove marker-clear blocker");
        }
        let config: Value = serde_json::from_slice(
            &fs::read(fixture.repo_path.join(".git/gg/config.json")).unwrap(),
        )
        .unwrap();
        let attempt = &config["stacks"]["land-wait"]["gitlab_merge_train_queue_attempt"];

        assert!(
            matches!(summary["event"].as_str(), Some("error" | "summary")),
            "{mode}: {summary}"
        );
        assert_eq!(summary["status"], "error", "{mode}: {summary}");
        assert_eq!(record["status"], expected_status, "{mode}: {record}");
        assert_eq!(record["touched_remote"], true, "{mode}: {record}");
        assert_eq!(attempt["state"], "attempted", "{mode}: {attempt}");
        assert_eq!(attempt["operation_id"], operation_id, "{mode}: {attempt}");
        assert_eq!(
            fixture.requeued.exists(),
            provider_succeeded,
            "{mode}: {text}"
        );
        assert!(error.contains(operation_id), "{mode}: {text}");
        assert!(
            error.contains("requeue attempt marker remains authoritative"),
            "{mode}: {text}"
        );

        match mode {
            "requeue-success-finalization-fail" => {
                assert!(error.contains("requeue succeeded remotely"), "{text}");
                assert!(error.contains("finalization checkpoint failed"), "{text}");
                assert!(error.contains("inspect GitLab and operation"), "{text}");
            }
            "requeue-fail-finalization-fail" => {
                assert!(
                    error.contains("Remote mutation outcome is uncertain"),
                    "{text}"
                );
                assert!(error.contains("landing request failed"), "{text}");
                assert!(
                    error.contains("finalization checkpoint also failed"),
                    "{text}"
                );
                assert!(
                    error.contains("do not retry the POST automatically"),
                    "{text}"
                );
            }
            "requeue-success-marker-clear-fail" => {
                assert!(error.contains("requeue succeeded remotely"), "{text}");
                assert!(
                    error.contains(&format!("operation {operation_id} was finalized")),
                    "{text}"
                );
                assert!(error.contains("marker-clear checkpoint failed"), "{text}");
            }
            _ => unreachable!(),
        }
    }
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_ambiguous_requeue_is_restart_safe_without_provider_access() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");
    fs::write(&fixture.landing_request_fail, "block-requeue-fail\n")
        .expect("block and fail the requeue request");
    let args = ["land", "--all", "--wait", "--jsonl", "--no-clean"];

    let land = fixture.start_land_with_args(&args);
    fixture.wait_until_queued();
    fs::write(&fixture.head_moved, "moved\n").expect("move fake MR head");
    fs::write(&fixture.auto_merge_dropped, "dropped\n").expect("drop fake auto-merge");
    wait_for_file(&fixture.landing_request_started());

    let pending: Vec<Value> = land_operation_records(&fixture.repo_path)
        .into_iter()
        .filter(|record| record["status"] == "pending")
        .collect();
    assert_eq!(
        pending.len(),
        1,
        "expected exact pending requeue: {pending:?}"
    );
    let operation_id = pending[0]["id"]
        .as_str()
        .expect("pending requeue operation id")
        .to_string();
    assert_eq!(pending[0]["touched_remote"], true);
    let markers = pending[0]["args"]
        .as_array()
        .expect("requeue operation args")
        .iter()
        .filter_map(Value::as_str)
        .filter(|arg| arg.starts_with("gitlab:"))
        .collect::<Vec<_>>();
    assert_eq!(
        markers,
        [
            "gitlab:requeue_merge_train",
            "gitlab:stack=land-wait",
            "gitlab:mr=41",
            "gitlab:target_branch=main",
        ]
    );
    let config_path = fixture.repo_path.join(".git/gg/config.json");
    let pending_config: Value =
        serde_json::from_slice(&fs::read(&config_path).expect("read pending config"))
            .expect("parse pending config");
    let pending_attempt =
        &pending_config["stacks"]["land-wait"]["gitlab_merge_train_queue_attempt"];
    assert_eq!(pending_attempt["action"], "requeue_merge_train");
    assert_eq!(pending_attempt["state"], "attempted");
    assert_eq!(pending_attempt["mr_number"], 41);
    assert_eq!(pending_attempt["target_branch"], "main");
    assert_eq!(pending_attempt["operation_id"], operation_id);

    fixture.release_landing_request();
    let output = land.wait_with_output().expect("wait for land");
    let first_text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        first_text.contains("Remote mutation outcome is uncertain"),
        "{first_text}"
    );
    assert!(first_text.contains("MR !41"), "{first_text}");
    assert!(first_text.contains(&operation_id), "{first_text}");
    assert!(
        !fixture.requeued.exists(),
        "failed requeue must not look successful"
    );
    let attempts_path = std::path::PathBuf::from(format!(
        "{}.attempts",
        fixture.landing_request_fail.display()
    ));
    assert_eq!(
        fs::read_to_string(&attempts_path)
            .expect("read requeue attempt count")
            .lines()
            .count(),
        1,
        "mutation must not be retried"
    );

    let records = land_operation_records(&fixture.repo_path);
    let record = records
        .iter()
        .find(|record| record["id"].as_str() == Some(operation_id.as_str()))
        .expect("same requeue operation after failure");
    assert_eq!(record["status"], "committed");
    assert_eq!(record["touched_remote"], true);
    let config_before = fs::read(&config_path).expect("snapshot config");
    let retained_config: Value =
        serde_json::from_slice(&config_before).expect("parse retained config");
    let retained_attempt =
        &retained_config["stacks"]["land-wait"]["gitlab_merge_train_queue_attempt"];
    assert_eq!(retained_attempt["action"], "requeue_merge_train");
    assert_eq!(retained_attempt["state"], "attempted");
    assert_eq!(retained_attempt["operation_id"], operation_id);
    let operations_dir = fixture.repo_path.join(".git/gg/operations");
    let snapshot_operations = || {
        let mut files = fs::read_dir(&operations_dir)
            .expect("list operations")
            .map(|entry| {
                let path = entry.expect("operation entry").path();
                (
                    path.file_name().unwrap().to_os_string(),
                    fs::read(path).expect("read operation"),
                )
            })
            .collect::<Vec<_>>();
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    };
    let operations_before = snapshot_operations();
    let refs_before = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&fixture.repo_path)
        .output()
        .expect("snapshot refs")
        .stdout;
    fs::write(&fixture.merge_settings_fail, "forbid-all")
        .expect("forbid every provider process access");

    let restart = fixture.land_command(&args).output().expect("restart Land");
    let restart_text = format!(
        "{}{}",
        String::from_utf8_lossy(&restart.stdout),
        String::from_utf8_lossy(&restart.stderr)
    );
    assert!(!restart.status.success(), "restart must fail closed");
    assert!(restart_text.contains("MR !41"), "{restart_text}");
    assert!(restart_text.contains(&operation_id), "{restart_text}");
    assert!(
        restart_text.contains("will not be retried automatically"),
        "{restart_text}"
    );
    assert!(!fixture
        .merge_settings_fail
        .with_extension("accessed")
        .exists());
    assert_eq!(
        fs::read_to_string(attempts_path)
            .expect("read requeue attempts after restart")
            .lines()
            .count(),
        1,
        "restart retried the requeue POST"
    );
    assert_eq!(fs::read(config_path).expect("read config"), config_before);
    assert_eq!(snapshot_operations(), operations_before);
    let refs_after = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&fixture.repo_path)
        .output()
        .expect("read refs")
        .stdout;
    assert_eq!(refs_after, refs_before);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_merge_train_recovery_rejects_conflicting_action_markers() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");
    fs::write(&fixture.landing_request_fail, "block-requeue-fail\n")
        .expect("block and fail the requeue request");
    let args = ["land", "--all", "--wait", "--jsonl", "--no-clean"];

    let land = fixture.start_land_with_args(&args);
    fixture.wait_until_queued();
    fs::write(&fixture.head_moved, "moved\n").expect("move fake MR head");
    fs::write(&fixture.auto_merge_dropped, "dropped\n").expect("drop fake auto-merge");
    wait_for_file(&fixture.landing_request_started());
    fixture.release_landing_request();
    let _ = land.wait_with_output().expect("wait for ambiguous requeue");

    let config_path = fixture.repo_path.join(".git/gg/config.json");
    let config: Value = serde_json::from_slice(&fs::read(&config_path).expect("read config"))
        .expect("parse config");
    let attempt = &config["stacks"]["land-wait"]["gitlab_merge_train_queue_attempt"];
    assert_eq!(attempt["action"], "requeue_merge_train");
    assert_eq!(attempt["state"], "attempted");
    let operation_id = attempt["operation_id"]
        .as_str()
        .expect("requeue operation ID");
    let operation_path = fixture
        .repo_path
        .join(format!(".git/gg/operations/{operation_id}.json"));
    let mut operation: Value =
        serde_json::from_slice(&fs::read(&operation_path).expect("read operation"))
            .expect("parse operation");
    let operation_args = operation["args"].as_array_mut().expect("operation args");
    assert!(operation_args
        .iter()
        .any(|arg| arg == "gitlab:requeue_merge_train"));
    assert!(!operation_args
        .iter()
        .any(|arg| arg == "gitlab:queue_merge_train"));
    operation_args.push("gitlab:queue_merge_train".into());
    let conflicting_operation =
        serde_json::to_vec_pretty(&operation).expect("serialize conflicting operation");
    fs::write(&operation_path, &conflicting_operation).expect("inject conflicting action marker");

    let config_before = fs::read(&config_path).expect("snapshot config");
    let operations_dir = fixture.repo_path.join(".git/gg/operations");
    let snapshot_operations = || {
        let mut files = fs::read_dir(&operations_dir)
            .expect("list operations")
            .map(|entry| {
                let path = entry.expect("operation entry").path();
                (
                    path.file_name().unwrap().to_os_string(),
                    fs::read(path).expect("read operation"),
                )
            })
            .collect::<Vec<_>>();
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    };
    let operations_before = snapshot_operations();
    let refs_before = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&fixture.repo_path)
        .output()
        .expect("snapshot refs")
        .stdout;
    fs::write(&fixture.merge_settings_fail, "forbid-all")
        .expect("forbid every provider process access");

    let restart = fixture.land_command(&args).output().expect("restart Land");
    let restart_text = format!(
        "{}{}",
        String::from_utf8_lossy(&restart.stdout),
        String::from_utf8_lossy(&restart.stderr)
    );
    assert!(!restart.status.success(), "restart must fail closed");
    assert!(
        restart_text.contains(&format!(
            "Inconsistent merge-train recovery evidence for operation {operation_id}"
        )),
        "{restart_text}"
    );
    assert!(!fixture
        .merge_settings_fail
        .with_extension("accessed")
        .exists());
    assert_eq!(fs::read(config_path).expect("read config"), config_before);
    assert_eq!(snapshot_operations(), operations_before);
    let refs_after = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&fixture.repo_path)
        .output()
        .expect("read refs")
        .stdout;
    assert_eq!(refs_after, refs_before);
    assert_eq!(
        fs::read(operation_path).expect("read conflicting operation"),
        conflicting_operation
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_revalidates_between_consecutive_remote_mutations() {
    let pipeline_fixture = WaitingLandFixture::new();
    pipeline_fixture.use_gitlab_provider();
    pipeline_fixture.enable_gitlab_merge_trains();
    pipeline_fixture.add_second_entry();
    fs::write(
        pipeline_fixture.repo_path.join("fake-retarget-pipeline"),
        "missing-after-pipeline",
    )
    .expect("drop draft state after pipeline creation");

    let pipeline_output = pipeline_fixture
        .start_land_with_args(&[
            "land",
            "--until",
            "2",
            "--wait",
            "--json",
            "--no-clean",
            "--no-squash",
            "--rebase-if-needed",
        ])
        .wait_with_output()
        .expect("wait for pipeline landing");
    let pipeline_stdout = String::from_utf8(pipeline_output.stdout).expect("land stdout");
    let pipeline_stderr = String::from_utf8(pipeline_output.stderr).expect("land stderr");
    let pipeline_summary: Value =
        serde_json::from_str(pipeline_stdout.trim()).expect("pipeline landing summary");
    assert!(
        pipeline_summary["land"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("missing field `draft`")),
        "{pipeline_summary}"
    );
    assert!(
        format!("{pipeline_stdout}{pipeline_stderr}").contains("missing field `draft`"),
        "pipeline path must report the unsafe state: stdout={pipeline_stdout} stderr={pipeline_stderr}"
    );
    assert!(
        std::path::PathBuf::from(format!(
            "{}.started",
            pipeline_fixture
                .repo_path
                .join("fake-retarget-pipeline")
                .display()
        ))
        .exists(),
        "the pipeline mutation must happen before the state transition"
    );
    assert!(
        !pipeline_fixture
            .queued_request()
            .contains("merge_requests/42"),
        "MR !42 must not be queued after the state transition"
    );

    let squash_fixture = WaitingLandFixture::new();
    squash_fixture.use_gitlab_provider();
    fs::write(&squash_fixture.squash_option, "default_on").expect("set fake squash option");
    fs::write(&squash_fixture.mr_squash, "on\n").expect("enable fake MR squash");
    fs::write(
        squash_fixture.repo_path.join("fake-retarget-pipeline"),
        "missing-after-squash",
    )
    .expect("drop draft state after squash update");

    let squash_output = squash_fixture
        .start_land_with_args(&["land", "--json", "--no-clean", "--no-squash"])
        .wait_with_output()
        .expect("wait for direct landing");
    let squash_stdout = String::from_utf8(squash_output.stdout).expect("land stdout");
    let squash_stderr = String::from_utf8(squash_output.stderr).expect("land stderr");
    let squash_summary: Value =
        serde_json::from_str(squash_stdout.trim()).expect("direct landing summary");
    assert!(
        squash_summary["land"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("missing field `draft`")),
        "{squash_summary}"
    );
    assert!(squash_fixture.squash_disabled.exists());
    assert!(
        !squash_fixture.merged.exists(),
        "MR must not be merged after the state transition"
    );
    assert!(
        format!("{squash_stdout}{squash_stderr}").contains("missing field `draft`"),
        "direct path must report the unsafe state: stdout={squash_stdout} stderr={squash_stderr}"
    );
    let records = land_operation_records(&squash_fixture.repo_path);
    assert_eq!(records.len(), 1, "expected the squash segment: {records:?}");
    assert_eq!(records[0]["status"], "committed");
    assert_eq!(records[0]["touched_remote"], true);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_refuses_failed_mr_lookup_before_remote_mutation() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();
    let pipeline_state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&pipeline_state, "lookup-fail").expect("fail MR lookup after retargeting");

    let output = fixture
        .start_land_with_args(&[
            "land",
            "--until",
            "2",
            "--wait",
            "--jsonl",
            "--no-clean",
            "--no-squash",
            "--rebase-if-needed",
        ])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("land stdout");
    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");

    assert_eq!(summary["status"], "error", "{summary}");
    assert!(
        summary["error"]
            .as_str()
            .or_else(|| summary["message"].as_str())
            .is_some_and(|error| error.contains("MR lookup failed")),
        "{summary}"
    );
    assert!(
        !std::path::PathBuf::from(format!("{}.started", pipeline_state.display())).exists(),
        "the pipeline must not be started when MR lookup fails"
    );
    assert!(
        !fixture.queued_request().contains("merge_requests/42"),
        "MR !42 must not be queued when its lookup fails"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_pipeline_revalidates_identity_immediately_before_post() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();
    let state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&state, "identity-change").unwrap();
    let output = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("identity changed"), "{text}");
    assert!(!std::path::PathBuf::from(format!("{}.attempts", state.display())).exists());
    assert!(!fixture.queued_request().contains("merge_requests/42"));
    assert!(gitlab_landing_attempt(&fixture.repo_path).is_none());
    let (_, operation) = pipeline_operation(&fixture.repo_path);
    assert_eq!(operation["status"], "committed");
    assert_eq!(operation["touched_remote"], false);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_wait_rechecks_approval_before_pipeline_post() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();
    let state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&state, "missing").unwrap();
    fs::write(&fixture.approval_regress, "pipeline-operation\n").unwrap();

    let output = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    let stdout = String::from_utf8(output.stdout).expect("land stdout");
    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    let attempts = std::path::PathBuf::from(format!("{}.attempts", state.display()));
    let started = std::path::PathBuf::from(format!("{}.started", state.display()));

    assert!(
        !attempts.exists(),
        "pipeline POST attempt marker exists: {stdout}"
    );
    assert!(
        !started.exists(),
        "pipeline POST started marker exists: {stdout}"
    );
    assert_eq!(summary["error"], "MR !42 is not approved", "{summary}");
    assert!(
        !fixture.queued_request().contains("merge_requests/42"),
        "MR !42 queue was mutated"
    );
    assert!(gitlab_landing_attempt(&fixture.repo_path).is_none());
    let (_, operation) = pipeline_operation(&fixture.repo_path);
    assert_eq!(operation["status"], "committed");
    assert_eq!(operation["touched_remote"], false);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_waiter_reacquire_refuses_foreign_pipeline_uncertainty() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();
    fs::write(&fixture.unapproved, "hold").unwrap();
    let mut command = fixture.land_command(&PIPELINE_LAND_ARGS);
    let mut guard = ProcessGroupGuard::spawn(&mut command).unwrap();
    let mut reader = BufReader::new(guard.take_stdout().expect("waiter stdout"));
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["event"],
        "start"
    );
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["event"],
        "wait"
    );

    fs::remove_file(&fixture.unapproved).unwrap();
    fs::write(&fixture.train_merged, "ready").unwrap();
    fs::write(&fixture.merged, "merged").unwrap();
    let state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&state, "applied-response-fail").unwrap();
    let mut competing = fixture.land_command(&PIPELINE_LAND_ARGS);
    let competing_output = ProcessGroupGuard::spawn(&mut competing)
        .unwrap()
        .wait_with_output_timeout(Duration::from_secs(30))
        .unwrap();
    let competing_text = format!(
        "{}{}",
        String::from_utf8_lossy(&competing_output.stdout),
        String::from_utf8_lossy(&competing_output.stderr)
    );
    assert!(
        competing_text.contains("will not be resubmitted"),
        "{competing_text}"
    );
    let output = guard
        .wait_with_output_timeout(Duration::from_secs(30))
        .unwrap();
    let mut remaining = String::new();
    reader.read_to_string(&mut remaining).unwrap();
    let text = format!("{}{}", remaining, String::from_utf8_lossy(&output.stderr));
    assert!(text.contains("will not be resubmitted"), "{text}");
    assert_eq!(
        fs::read_to_string(format!("{}.attempts", state.display()))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(!fixture.queued.exists());
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_waiter_reacquire_refuses_foreign_merge_train_queue_uncertainty() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();
    fs::write(&fixture.unapproved, "hold").unwrap();
    let mut command = fixture.land_command(&PIPELINE_LAND_ARGS);
    let mut guard = ProcessGroupGuard::spawn(&mut command).unwrap();
    let mut reader = BufReader::new(guard.take_stdout().expect("waiter stdout"));
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["event"],
        "start"
    );
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["event"],
        "wait"
    );

    fs::remove_file(&fixture.unapproved).unwrap();
    fs::write(&fixture.train_merged, "ready").unwrap();
    fs::write(&fixture.merged, "merged").unwrap();
    fs::write(
        &fixture.landing_request_fail,
        "merge-train-applied-response-fail",
    )
    .unwrap();
    let competing = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    let competing_text = format!(
        "{}{}",
        String::from_utf8_lossy(&competing.stdout),
        String::from_utf8_lossy(&competing.stderr)
    );
    assert!(competing_text.contains("will not be retried automatically"));

    let output = guard
        .wait_with_output_timeout(Duration::from_secs(30))
        .unwrap();
    let mut remaining = String::new();
    reader.read_to_string(&mut remaining).unwrap();
    let text = format!("{}{}", remaining, String::from_utf8_lossy(&output.stderr));
    assert!(text.contains("will not be retried automatically"), "{text}");
    assert_eq!(
        fs::read_to_string(format!("{}.attempts", fixture.queued.display()))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_pipeline_uncertainty_is_restart_safe() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();
    let pipeline_state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&pipeline_state, "applied-response-fail").unwrap();

    let first = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    assert!(!first.status.success(), "ambiguous POST must be fatal");
    let first_output = format!(
        "{}{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        first_output.contains("Remote mutation outcome is uncertain"),
        "{first_output}"
    );
    assert!(
        !fixture.queued_request().contains("merge_requests/42"),
        "uncertain pipeline must not be queued"
    );

    let records = land_operation_records(&fixture.repo_path);
    let pipeline = records
        .iter()
        .find(|record| {
            record["args"]
                .as_array()
                .is_some_and(|args| args.iter().any(|arg| arg == "gitlab:pipeline_start"))
        })
        .expect("dedicated pipeline operation");
    assert_eq!(pipeline["status"], "pending");
    assert_eq!(pipeline["touched_remote"], true);

    let retry = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    assert!(!retry.status.success(), "restart must refuse resubmission");
    let retry_output = format!(
        "{}{}",
        String::from_utf8_lossy(&retry.stdout),
        String::from_utf8_lossy(&retry.stderr)
    );
    assert!(
        retry_output.contains("will not be resubmitted"),
        "{retry_output}"
    );
    let attempts = fs::read_to_string(format!("{}.attempts", pipeline_state.display())).unwrap();
    assert_eq!(attempts.lines().count(), 1);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_completed_recovery_finalizes_without_provider_lookup() {
    let (fixture, _) = completed_pipeline_fixture();
    let (path, mut operation) = pipeline_operation(&fixture.repo_path);
    operation["status"] = "pending".into();
    fs::write(&path, serde_json::to_vec_pretty(&operation).unwrap()).unwrap();
    let state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&state, "recovery-order").unwrap();
    let output = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    let start = text.find("\"event\":\"start\"").unwrap();
    let checkpoint = text.find("\"event\":\"pipeline_checkpoint\"").unwrap();
    assert!(start < checkpoint);
    let recovered: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(recovered["status"], "committed");
    assert!(!std::path::PathBuf::from(format!("{}.early", state.display())).exists());
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_completed_handoff_survives_stale_null() {
    let (fixture, _) = completed_pipeline_fixture();
    let state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&state, "missing").unwrap();
    fs::remove_file(format!("{}.started", state.display())).unwrap();
    let mut command = fixture.land_command(&PIPELINE_LAND_ARGS);
    let output = ProcessGroupGuard::spawn(&mut command)
        .unwrap()
        .wait_with_output_timeout(Duration::from_secs(30))
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(format!("{}.attempts", state.display()))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(fixture.queued_request().contains("merge_requests/42"));
    assert!(gitlab_landing_attempt(&fixture.repo_path).is_none());
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_accepts_merged_results_pipeline_response() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();
    let state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&state, "missing").unwrap();
    let output = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        fs::read_to_string(format!("{}.attempts", state.display()))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(fixture.queued_request().contains("merge_requests/42"));
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_pipeline_post_error_is_fatal_remote_uncertainty() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();
    let state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&state, "rejected").unwrap();
    let output = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        text.contains("Remote mutation outcome is uncertain"),
        "{text}"
    );
    assert!(!fixture.queued_request().contains("merge_requests/42"));
    assert_eq!(
        gitlab_landing_attempt(&fixture.repo_path).unwrap()["state"],
        "attempted"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_pipeline_checkpoint_broken_pipe_blocks_queue() {
    let (fixture, output) = completed_pipeline_fixture();
    assert!(!output.status.success());
    assert!(!fixture.queued_request().contains("merge_requests/42"));
    assert_eq!(
        gitlab_landing_attempt(&fixture.repo_path).unwrap()["state"],
        "completed"
    );
    let (_, operation) = pipeline_operation(&fixture.repo_path);
    assert_eq!(operation["status"], "committed");
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_preceding_segment_finalize_failure_is_durability_checkpoint() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();
    let state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&state, "preceding-finalize-fail").unwrap();
    let output = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("Durability checkpoint error"), "{text}");
    assert!(!std::path::PathBuf::from(format!("{}.attempts", state.display())).exists());
    assert!(!fixture.queued_request().contains("merge_requests/42"));
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_pipeline_recovery_rejects_conflicting_identity_or_operation() {
    for corruption in ["marker", "kind", "status", "effect", "pipeline", "identity"] {
        let fixture = WaitingLandFixture::new();
        fixture.use_gitlab_provider();
        fixture.enable_gitlab_merge_trains();
        fixture.add_second_entry();
        let state = fixture.repo_path.join("fake-retarget-pipeline");
        fs::write(&state, "applied-response-fail").unwrap();
        let _ = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
        let config_path = fixture.repo_path.join(".git/gg/config.json");
        let (operation_path, mut operation) = pipeline_operation(&fixture.repo_path);
        let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        let attempt = &mut config["stacks"]["land-wait"]["gitlab_landing_attempt"];
        match corruption {
            "marker" => operation["args"]
                .as_array_mut()
                .unwrap()
                .retain(|arg| arg != "gitlab:source_project=7"),
            "kind" => operation["kind"] = "sync".into(),
            "status" => operation["status"] = "invalid".into(),
            "effect" => {
                operation["remote_effects"] =
                    serde_json::json!([{"kind":"pr_merged","number":42,"url":"wrong"}])
            }
            "pipeline" => {
                attempt["state"] = "completed".into();
                attempt.as_object_mut().unwrap().remove("pipeline_id");
            }
            "identity" => {
                attempt.as_object_mut().unwrap().remove("source_oid");
            }
            _ => unreachable!(),
        }
        fs::write(
            operation_path,
            serde_json::to_vec_pretty(&operation).unwrap(),
        )
        .unwrap();
        fs::write(config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        let retry = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&retry.stdout),
            String::from_utf8_lossy(&retry.stderr)
        );
        assert!(!retry.status.success(), "{corruption}: {text}");
        assert!(
            !text.contains("will not be resubmitted"),
            "{corruption}: {text}"
        );
        assert_eq!(
            fs::read_to_string(format!("{}.attempts", state.display()))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_startup_merged_handoff_clears_before_later_entry() {
    let (fixture, _) = completed_pipeline_fixture();
    repoint_completed_pipeline_handoff_to_first_entry(&fixture);

    let output = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(fixture.queued_request().contains("merge_requests/42"));
    assert!(gitlab_landing_attempt(&fixture.repo_path).is_none());
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_readiness_merged_handoff_clears_before_later_entry() {
    let (fixture, _) = completed_pipeline_fixture();
    repoint_completed_pipeline_handoff_to_first_entry(&fixture);
    fs::remove_file(&fixture.merged).unwrap();
    fixture.complete_merge_train_status();

    let output = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fixture.queued_request().contains("merge_requests/42"));
    assert!(gitlab_landing_attempt(&fixture.repo_path).is_none());
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_startup_closed_handoff_clears_before_later_entry() {
    let (fixture, _) = completed_pipeline_fixture();
    repoint_completed_pipeline_handoff_to_first_entry(&fixture);
    fs::write(&fixture.closed, "closed").unwrap();

    let output = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(fixture.queued_request().contains("merge_requests/42"));
    assert!(gitlab_landing_attempt(&fixture.repo_path).is_none());
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_pipeline_recovery_rejects_internal_operation_id_mismatch() {
    let (fixture, _) = completed_pipeline_fixture();
    let (path, mut operation) = pipeline_operation(&fixture.repo_path);
    operation["status"] = "pending".into();
    operation["id"] = "op_0000000000000_mismatched".into();
    fs::write(&path, serde_json::to_vec_pretty(&operation).unwrap()).unwrap();

    let output = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{text}");
    assert!(
        text.contains("conflicting GitLab pipeline identity"),
        "{text}"
    );
    let persisted: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(persisted["status"], "pending");
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_recovered_handoff_revalidates_identity_before_queue() {
    let (fixture, _) = completed_pipeline_fixture();
    let state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&state, "identity-change").unwrap();

    let output = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("identity changed"), "{text}");
    assert!(!fixture.queued_request().contains("merge_requests/42"));
    assert_eq!(
        gitlab_landing_attempt(&fixture.repo_path).unwrap()["state"],
        "completed"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_pipeline_recovery_finalizes_sweep_interrupted_record() {
    let (fixture, _) = completed_pipeline_fixture();
    let (path, mut operation) = pipeline_operation(&fixture.repo_path);
    operation["status"] = "pending".into();
    operation["created_at_ms"] = 0.into();
    fs::write(&path, serde_json::to_vec_pretty(&operation).unwrap()).unwrap();
    let sweep = Command::new(env!("CARGO_BIN_EXE_gg"))
        .args(["co", "sweep-trigger"])
        .current_dir(&fixture.other_worktree)
        .env("HOME", &fixture.test_home)
        .env("PATH", &fixture.path)
        .output()
        .unwrap();
    assert!(
        sweep.status.success(),
        "{}",
        String::from_utf8_lossy(&sweep.stderr)
    );
    let interrupted: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(interrupted["status"], "interrupted");

    let output = fixture.land_command(&PIPELINE_LAND_ARGS).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let recovered: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(recovered["status"], "committed");
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_finalizes_pipeline_mutation_when_queue_request_fails() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.add_second_entry();
    let pipeline_state = fixture.repo_path.join("fake-retarget-pipeline");
    fs::write(&pipeline_state, "missing").expect("require a pipeline after retargeting");
    fixture.block_second_gitlab_landing_request(false);

    let (output, operation_id) = run_blocked_landing_and_capture_pending(
        &fixture,
        &[
            "land",
            "--until",
            "2",
            "--wait",
            "--jsonl",
            "--no-clean",
            "--no-squash",
            "--rebase-if-needed",
        ],
    );
    let stdout = String::from_utf8(output.stdout).expect("land stdout");
    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");

    assert_eq!(summary["status"], "error", "{summary}");
    assert!(
        summary["error"]
            .as_str()
            .is_some_and(|error| error.contains("landing request failed")),
        "{summary}"
    );
    assert!(
        std::path::PathBuf::from(format!("{}.started", pipeline_state.display())).exists(),
        "the pipeline must be created before the final queue request"
    );
    assert!(
        !fixture.queued_request().contains("merge_requests/42"),
        "MR !42 must not have a successful queue marker"
    );
    let records = land_operation_records(&fixture.repo_path);
    let pipeline_record = records
        .iter()
        .find(|record| record["id"].as_str() == Some(operation_id.as_str()))
        .expect("same pipeline and queue operation");
    assert_eq!(
        pipeline_record["status"], "committed",
        "the failed queue segment must be finalized: {pipeline_record}"
    );
    assert_eq!(
        pipeline_record["touched_remote"], true,
        "the successful pipeline mutation must remain non-undoable: {pipeline_record}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_disconnect_after_pipeline_creation_finalizes_remote_state() {
    for succeeds in [true, false] {
        let fixture = WaitingLandFixture::new();
        fixture.use_gitlab_provider();
        fixture.enable_gitlab_merge_trains();
        fixture.add_second_entry();
        let pipeline_state = fixture.repo_path.join("fake-retarget-pipeline");
        fs::write(&pipeline_state, "missing").expect("require a pipeline after retargeting");
        fixture.block_second_gitlab_landing_request(succeeds);

        let (output, operation_id) = disconnect_jsonl_and_capture_pending_landing(
            &fixture,
            &[
                "land",
                "--until",
                "2",
                "--wait",
                "--jsonl",
                "--no-clean",
                "--no-squash",
                "--rebase-if-needed",
            ],
        );

        assert!(!output.status.success(), "disconnected JSONL must fail");
        assert!(
            std::path::PathBuf::from(format!("{}.started", pipeline_state.display())).exists(),
            "the pipeline transition must occur before disconnecting"
        );
        assert_eq!(
            fixture.queued_request().contains("merge_requests/42"),
            succeeds,
            "the final queue marker must match the provider result"
        );
        let records = land_operation_records(&fixture.repo_path);
        let record = records
            .iter()
            .find(|record| record["id"] == operation_id)
            .expect("same operation after stream disconnect");
        assert_eq!(
            record["status"], "committed",
            "the exact blocked segment must be finalized before streaming: {record}"
        );
        assert_eq!(
            record["touched_remote"], true,
            "the exact blocked segment must remain non-undoable: {record}"
        );
    }
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_merge_train_wait_stops_when_pipeline_fails_before_joining_train() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");

    let land = fixture.start_land_with_args(&["land", "--all", "--wait", "--jsonl", "--no-clean"]);
    fixture.wait_until_queued();
    fixture.set_glab_pipeline_status("failed");

    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert!(
        summary["error"]
            .as_str()
            .is_some_and(|error| error.contains("pipeline failed")),
        "a failed pipeline must end the merge-train wait: {summary}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_merge_train_wait_requeues_when_a_push_cancels_auto_merge() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");

    let land = fixture.start_land_with_args(&["land", "--all", "--wait", "--jsonl", "--no-clean"]);
    fixture.wait_until_queued();
    // A push to the source branch moves the MR head and makes GitLab cancel
    // auto-merge.
    fs::write(&fixture.head_moved, "moved\n").expect("move fake MR head");
    fs::write(&fixture.auto_merge_dropped, "dropped\n").expect("drop fake auto-merge");

    let deadline = Instant::now() + Duration::from_secs(60);
    while !fixture.requeued.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        fixture.requeued.exists(),
        "land never re-queued the MR after auto-merge was cancelled"
    );
    let requeue = fs::read_to_string(&fixture.requeued).expect("read re-queue request");
    assert!(requeue.contains("auto_merge=true"), "{requeue}");
    assert!(requeue.contains("squash=true"), "{requeue}");

    fs::write(&fixture.merged, "merged\n").expect("complete fake merge train");
    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");

    let events: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse JSONL event"))
        .collect();
    assert!(
        events.iter().any(|event| event["event"] == "wait"
            && event["status"] == "warning"
            && event["error"]
                .as_str()
                .is_some_and(|error| error.contains("re-queued"))),
        "a wait event must report the re-queue: {stdout}"
    );
    let summary = events.last().expect("summary event");
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["landed"][0]["action"], "merged", "{summary}");
    assert!(summary["error"].is_null(), "{summary}");
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_merge_train_wait_stops_when_auto_merge_cancelled_without_a_push() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");

    let land = fixture.start_land_with_args(&["land", "--all", "--wait", "--jsonl", "--no-clean"]);
    fixture.wait_until_queued();
    // Same head: the train pipeline failed or someone cancelled auto-merge.
    fs::write(&fixture.auto_merge_dropped, "dropped\n").expect("drop fake auto-merge");

    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["event"], "summary");
    assert!(
        summary["error"]
            .as_str()
            .is_some_and(|error| error.contains("removed") && error.contains("without a new push")),
        "the wait must stop without re-queueing: {summary}"
    );
    assert!(
        !fixture.requeued.exists(),
        "gg must not re-queue an MR whose head did not move"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_no_squash_turns_off_mr_squash_before_queueing() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fs::write(&fixture.squash_option, "default_on").expect("set fake squash option");
    fs::write(&fixture.mr_squash, "on\n").expect("enable fake MR squash");

    let land = fixture.start_land_with_args(&[
        "land",
        "--all",
        "--wait",
        "--jsonl",
        "--no-clean",
        "--no-squash",
    ]);
    fixture.wait_until_queued();
    let squash_update =
        fs::read_to_string(&fixture.squash_disabled).expect("MR squash must be turned off first");
    assert!(squash_update.contains("squash=false"), "{squash_update}");
    let request = fixture.queued_request();
    assert!(request.contains("squash=false"), "{request}");

    fs::write(&fixture.merged, "merged\n").expect("complete fake merge train");
    let output = land.wait_with_output().expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(stdout.contains("squash_checkpoint") && stdout.contains("\"squash\":false"));
    let records = land_operation_records(&fixture.repo_path);
    let squash_record = records
        .iter()
        .find(|record| record["args"].to_string().contains("gitlab:squash_disable"))
        .expect("dedicated squash operation");
    assert_eq!(squash_record["status"], "committed");
    assert_eq!(
        squash_record["remote_effects"],
        serde_json::json!([{"kind":"pr_squash_updated","number":41,"squash":false}])
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_no_squash_rechecks_approval_before_put() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fs::write(&fixture.squash_option, "default_on").expect("set fake squash option");
    fs::write(&fixture.mr_squash, "on\n").expect("enable fake MR squash");
    fs::write(&fixture.approval_regress, "2\n").expect("regress third approval lookup");

    let output = fixture
        .land_command(&["land", "--json", "--no-clean", "--no-squash"])
        .output()
        .expect("run land");
    let result: Value = serde_json::from_slice(&output.stdout).expect("parse land JSON");

    assert!(!output.status.success());
    assert!(result["land"]["error"]
        .as_str()
        .or_else(|| result["error"].as_str())
        .is_some_and(|error| error.contains("MR !41 is not approved")));
    assert_eq!(
        fs::read_to_string(format!("{}.calls", fixture.approval_regress.display()))
            .expect("approval call count")
            .trim(),
        "3"
    );
    assert!(
        !fixture.squash_disabled.exists(),
        "squash PUT marker exists"
    );
    assert!(
        !std::path::PathBuf::from(format!("{}.attempts", fixture.squash_disabled.display()))
            .exists()
    );
    let record = land_operation_records(&fixture.repo_path)
        .pop()
        .expect("approval refusal segment");
    assert_eq!(record["status"], "committed", "{record}");
    assert_eq!(record["touched_remote"], false, "{record}");
    assert_eq!(record["remote_effects"], serde_json::json!([]));
    assert_eq!(record["refs_after"], record["refs_before"]);
    assert!(gitlab_landing_attempt(&fixture.repo_path).is_none());
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_wait_no_squash_rechecks_approval_inside_squash_segment() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.set_glab_pipeline_status("pending");
    fs::write(
        fixture.readiness_poll.with_extension("enabled"),
        "enabled\n",
    )
    .expect("enable readiness-poll handshake");
    fs::write(&fixture.squash_option, "default_on").expect("set fake squash option");
    fs::write(&fixture.mr_squash, "on\n").expect("enable fake MR squash");
    let refs_before = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&fixture.repo_path)
        .output()
        .expect("snapshot refs")
        .stdout;

    let mut command = fixture.land_command(&[
        "land",
        "--all",
        "--wait",
        "--jsonl",
        "--no-clean",
        "--no-squash",
    ]);
    let land = ProcessGroupGuard::spawn(&mut command).expect("start waiting land");
    wait_for_file(&fixture.readiness_poll);
    assert_eq!(
        fs::read_to_string(fixture.readiness_poll.with_extension("view-calls"))
            .unwrap()
            .trim(),
        "3"
    );
    fixture.mark_unapproved();
    fixture.set_glab_pipeline_status("success");
    fs::write(
        fixture.readiness_poll.with_extension("release"),
        "release\n",
    )
    .expect("release readiness poll");
    let output = land
        .wait_with_output_timeout(Duration::from_secs(30))
        .expect("wait for bounded land");
    let stdout = String::from_utf8(output.stdout).expect("land stdout");
    let events: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse JSONL event"))
        .collect();
    let terminal = events.last().expect("terminal error event");

    assert!(!output.status.success(), "{stdout}");
    assert_eq!(terminal["event"], "error", "{terminal}");
    assert_eq!(terminal["status"], "error", "{terminal}");
    assert_eq!(terminal["message"], "MR !41 is not approved", "{terminal}");
    assert!(
        !events
            .iter()
            .any(|event| matches!(event["event"].as_str(), Some("entry" | "squash_checkpoint"))),
        "{stdout}"
    );
    assert!(
        !fixture.squash_disabled.exists(),
        "squash PUT marker exists"
    );
    assert!(!fixture.merged.exists(), "downstream merge marker exists");
    assert!(gitlab_landing_attempt(&fixture.repo_path).is_none());
    let records = land_operation_records(&fixture.repo_path);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["status"], "committed", "{records:?}");
    assert_eq!(records[0]["touched_remote"], false, "{records:?}");
    assert_eq!(records[0]["remote_effects"], serde_json::json!([]));
    assert_eq!(records[0]["refs_after"], records[0]["refs_before"]);
    let refs_after = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&fixture.repo_path)
        .output()
        .expect("read refs")
        .stdout;
    assert_eq!(refs_after, refs_before);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_squash_approval_lookup_failure_finalizes_proven_noop() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fs::write(&fixture.squash_option, "default_on").unwrap();
    fs::write(&fixture.mr_squash, "on\n").unwrap();
    fs::write(&fixture.approval_regress, "fail-squash-segment\n").unwrap();
    let refs_before = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&fixture.repo_path)
        .output()
        .unwrap()
        .stdout;

    let output = fixture
        .land_command(&["land", "--jsonl", "--no-clean", "--no-squash"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!output.status.success(), "{stdout}");
    assert!(stdout.contains("MR !41 is not approved"), "{stdout}");
    assert!(fixture.approval_regress.with_extension("failed").exists());
    assert!(!stdout.contains("squash_checkpoint"), "{stdout}");
    assert!(!fixture.squash_disabled.exists());
    assert!(!fixture.queued.exists() && !fixture.merged.exists());
    let records = land_operation_records(&fixture.repo_path);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["status"], "committed");
    assert_eq!(records[0]["touched_remote"], false);
    assert_eq!(records[0]["remote_effects"], serde_json::json!([]));
    assert_eq!(records[0]["refs_after"], records[0]["refs_before"]);
    assert_eq!(
        Command::new("git")
            .args(["for-each-ref", "--format=%(refname) %(objectname)"])
            .current_dir(&fixture.repo_path)
            .output()
            .unwrap()
            .stdout,
        refs_before
    );
    assert!(gitlab_landing_attempt(&fixture.repo_path).is_none());
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_ambiguous_applied_squash_reconciles_without_second_put() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fs::write(&fixture.squash_option, "default_on").expect("set fake squash option");
    fs::write(&fixture.mr_squash, "on\n").expect("enable fake MR squash");
    fixture.fail_gitlab_squash_update_response();
    let args = ["land", "--json", "--no-clean", "--no-squash"];

    assert!(!fixture
        .land_command(&args)
        .output()
        .unwrap()
        .status
        .success());
    fs::remove_file(&fixture.landing_request_fail).unwrap();
    let restart = fixture.land_command(&args).output().unwrap();
    assert!(
        restart.status.success(),
        "{}",
        String::from_utf8_lossy(&restart.stderr)
    );
    assert_eq!(
        fs::read_to_string(format!("{}.attempts", fixture.squash_disabled.display()))
            .unwrap()
            .lines()
            .count(),
        1,
        "ambiguous PUT was retried"
    );
    assert!(gitlab_landing_attempt(&fixture.repo_path).is_none());
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_squash_veto_noop_and_unapplied_ambiguity_fail_closed() {
    let args = ["land", "--json", "--no-clean", "--no-squash"];

    let noop = WaitingLandFixture::new();
    noop.use_gitlab_provider();
    fs::write(&noop.squash_option, "default_on").expect("set squash policy");
    fs::write(&noop.approval_regress, "fail-squash-segment\n").unwrap();
    let output = noop.land_command(&args).output().unwrap();
    assert!(output.status.success());
    assert!(!noop.approval_regress.with_extension("failed").exists());

    assert!(!noop.squash_disabled.exists());
    assert!(
        !serde_json::to_string(&land_operation_records(&noop.repo_path))
            .unwrap()
            .contains("gitlab:squash_disable")
    );

    let veto = WaitingLandFixture::new();
    veto.use_gitlab_provider();
    fs::write(&veto.squash_option, "default_on").expect("set squash policy");
    fs::write(&veto.mr_squash, "on\n").expect("enable squash");
    fs::write(
        veto.repo_path.join("fake-retarget-pipeline"),
        "identity-change",
    )
    .expect("change fresh identity");
    let output = veto.land_command(&args).output().unwrap();
    assert!(!output.status.success());
    assert!(
        !veto.squash_disabled.exists(),
        "identity veto must precede PUT"
    );
    let veto_record = land_operation_records(&veto.repo_path)
        .pop()
        .expect("no-effect segment");
    assert_eq!(veto_record["status"], "committed");
    assert_eq!(veto_record["touched_remote"], false);

    let ambiguous = WaitingLandFixture::new();
    ambiguous.use_gitlab_provider();
    fs::write(&ambiguous.squash_option, "default_on").expect("set squash policy");
    fs::write(&ambiguous.mr_squash, "on\n").expect("enable squash");
    fs::write(
        &ambiguous.landing_request_fail,
        "squash-unapplied-response-fail\n",
    )
    .expect("fail unapplied PUT");
    for _ in 0..2 {
        assert!(!ambiguous
            .land_command(&args)
            .output()
            .unwrap()
            .status
            .success());
    }
    let attempts = fs::read_to_string(format!("{}.attempts", ambiguous.squash_disabled.display()))
        .expect("read attempts");
    assert_eq!(attempts.lines().count(), 1, "ambiguous PUT was retried");
    assert!(
        !ambiguous.merged.exists(),
        "downstream merge must remain blocked"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_squash_recovery_rejects_corrupt_foreign_terminal_and_handoff() {
    for case in ["schema", "effect", "terminal", "draft", "pipeline"] {
        let fixture = WaitingLandFixture::new();
        fixture.use_gitlab_provider();
        fs::write(&fixture.squash_option, "default_on").unwrap();
        fs::write(&fixture.mr_squash, "on\n").unwrap();
        fs::write(
            &fixture.landing_request_fail,
            "squash-unapplied-response-fail\n",
        )
        .unwrap();
        let args = ["land", "--json", "--no-clean", "--no-squash"];
        let _ = fixture.land_command(&args).output().unwrap();
        let config_path = fixture.repo_path.join(".git/gg/config.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        let attempt = &mut config["stacks"]["land-wait"]["gitlab_landing_attempt"];
        let operation_id = attempt["operation_id"].as_str().unwrap().to_string();
        let record_path = fixture
            .repo_path
            .join(format!(".git/gg/operations/{operation_id}.json"));
        let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
        let retarget = fixture.repo_path.join("fake-retarget-pipeline");
        match case {
            "schema" => record["schema_version"] = Value::from(999),
            "effect" => {
                record["remote_effects"] = serde_json::json!([{
                    "kind":"pr_squash_updated", "number":999, "squash":false
                }])
            }
            "terminal" => fs::write(&retarget, "closed-on-landing").unwrap(),
            "draft" => fs::write(&retarget, "draft-on-landing").unwrap(),
            "pipeline" => attempt["action"] = Value::from("pipeline_start"),
            _ => unreachable!(),
        }
        fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        let restart = fixture.land_command(&args).output().unwrap();
        assert!(
            !restart.status.success(),
            "{case} recovery must fail closed"
        );
        let attempts =
            fs::read_to_string(format!("{}.attempts", fixture.squash_disabled.display())).unwrap();
        assert_eq!(attempts.lines().count(), 1, "{case} retried the PUT");
        assert!(!fixture.merged.exists(), "{case} allowed downstream merge");
    }
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_untouched_squash_recovery_does_not_undo_intervening_ref_move() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fs::write(&fixture.squash_option, "default_on").unwrap();
    fs::write(&fixture.mr_squash, "on\n").unwrap();
    fs::write(
        &fixture.landing_request_fail,
        "squash-unapplied-response-fail",
    )
    .unwrap();
    let args = ["land", "--json", "--no-clean", "--no-squash"];
    let _ = fixture.land_command(&args).output().unwrap();

    let config: Value =
        serde_json::from_slice(&fs::read(fixture.repo_path.join(".git/gg/config.json")).unwrap())
            .unwrap();
    let id = config["stacks"]["land-wait"]["gitlab_landing_attempt"]["operation_id"]
        .as_str()
        .unwrap();
    let record_path = fixture
        .repo_path
        .join(format!(".git/gg/operations/{id}.json"));
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["touched_remote"] = false.into();
    fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();

    let moved =
        fs::read_to_string(fixture.repo_path.join(".git/refs/heads/testuser/land-wait")).unwrap();
    run_git(
        &fixture.repo_path,
        &["update-ref", "refs/heads/testuser/other", moved.trim()],
    );
    fixture.fail_gitlab_merge_settings_lookup();
    let _ = fixture.land_command(&args).output().unwrap();
    assert!(gitlab_landing_attempt(&fixture.repo_path).is_none());
    let committed: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(committed["refs_after"], committed["refs_before"]);

    let undo = fixture
        .land_command(&["undo", id, "--json"])
        .output()
        .unwrap();
    assert!(!undo.status.success(), "moved ref must make undo refuse");
    let after =
        fs::read_to_string(fixture.repo_path.join(".git/refs/heads/testuser/other")).unwrap();
    assert_eq!(after.trim(), moved.trim());
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_no_squash_records_remote_touch_when_landing_request_fails() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fs::write(&fixture.squash_option, "default_on").expect("set fake squash option");
    fs::write(&fixture.mr_squash, "on\n").expect("enable fake MR squash");
    fixture.fail_gitlab_landing_request();

    let output = fixture
        .start_land_with_args(&["land", "--json", "--no-clean", "--no-squash"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let result: Value = serde_json::from_str(&stdout).expect("parse land JSON");

    assert!(
        fixture.squash_disabled.exists(),
        "MR squash must be disabled before the final request"
    );
    assert!(!fixture.queued.exists(), "final landing request must fail");
    assert!(result["land"]["error"]
        .as_str()
        .is_some_and(|error| error.contains("landing request failed")));

    let records = land_operation_records(&fixture.repo_path);
    assert_eq!(records.len(), 2, "expected squash and queue segments");
    assert!(records
        .iter()
        .all(|record| record["status"] == "committed" && record["touched_remote"] == true));
    assert!(records
        .iter()
        .any(|record| record["remote_effects"][0]["kind"] == "pr_squash_updated"));
}

#[cfg(unix)]
fn assert_failed_landing_attempt_is_committed(fixture: &WaitingLandFixture, args: &[&str]) {
    let output = fixture
        .start_land_with_args(args)
        .wait_with_output()
        .expect("wait for land");
    assert!(
        output.status.success(),
        "structured land must report failure in its payload"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let result: Value = serde_json::from_str(&stdout).expect("parse land JSON");
    assert!(
        result["land"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("landing request failed")),
        "the structured response must report the failed landing request: {result}"
    );
    assert!(
        !fixture.queued.exists(),
        "a failed landing request must not create the success marker"
    );
    assert!(
        !std::path::PathBuf::from(format!("{}.glab", fixture.merged.display())).exists(),
        "a failed direct merge must not create the GitLab merge-success marker"
    );
    let records = land_operation_records(&fixture.repo_path);
    assert_eq!(records.len(), 1, "expected one land operation: {records:?}");
    assert_eq!(records[0]["status"], "committed");
    assert_eq!(records[0]["touched_remote"], true);
}

#[cfg(unix)]
#[test]
fn test_land_merge_train_failure_records_remote_uncertainty_before_attempt() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.fail_gitlab_landing_request();
    assert_failed_landing_attempt_is_committed(&fixture, &["land", "--json", "--no-clean"]);
}

#[cfg(unix)]
#[test]
fn test_land_auto_merge_failure_records_remote_uncertainty_before_attempt() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.fail_gitlab_landing_request();
    assert_failed_landing_attempt_is_committed(
        &fixture,
        &["land", "--json", "--auto-merge", "--no-clean"],
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_auto_merge_lost_response_recovery_is_at_most_once() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fs::write(
        &fixture.landing_request_fail,
        "auto-merge-applied-response-fail\n",
    )
    .expect("fail the response after enabling auto-merge");
    let args = ["land", "--json", "--auto-merge", "--no-clean"];

    let first = fixture
        .land_command(&args)
        .output()
        .expect("run first Land");
    let first_text = format!(
        "{}{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        first_text.contains("connection lost after enabling auto-merge"),
        "{first_text}"
    );
    assert!(fixture.queued.exists(), "the PUT must reach GitLab");

    let attempt = gitlab_landing_attempt(&fixture.repo_path).expect("saved queue attempt");
    assert_eq!(attempt["action"], "queue_auto_merge");
    let operation_id = attempt["operation_id"]
        .as_str()
        .expect("saved operation ID")
        .to_string();
    let records_before = land_operation_records(&fixture.repo_path);
    assert_eq!(records_before.len(), 1, "{records_before:?}");
    assert_eq!(records_before[0]["id"], operation_id);
    assert_eq!(records_before[0]["touched_remote"], true);
    assert_eq!(
        fs::read_to_string(format!(
            "{}.attempts",
            fixture.landing_request_fail.display()
        ))
        .expect("read auto-merge attempts")
        .lines()
        .count(),
        1
    );

    fs::remove_file(&fixture.queued).expect("make provider state stale and not queued");
    let config_path = fixture.repo_path.join(".git/gg/config.json");
    let config_before = fs::read(&config_path).expect("snapshot config");
    let refs_before = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&fixture.repo_path)
        .output()
        .expect("snapshot refs")
        .stdout;

    let second = fixture.land_command(&args).output().expect("restart Land");
    let second_text = format!(
        "{}{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(!second.status.success(), "recovery must fail closed");
    assert!(second_text.contains(&operation_id), "{second_text}");
    assert_eq!(
        fs::read_to_string(format!(
            "{}.attempts",
            fixture.landing_request_fail.display()
        ))
        .expect("read auto-merge attempts")
        .lines()
        .count(),
        1,
        "recovery retried the auto-merge PUT"
    );
    assert_eq!(land_operation_records(&fixture.repo_path), records_before);
    assert_eq!(fs::read(config_path).expect("read config"), config_before);
    let refs_after = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&fixture.repo_path)
        .output()
        .expect("read refs")
        .stdout;
    assert_eq!(refs_after, refs_before);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_merge_train_lost_response_restart_is_at_most_once_without_provider_access() {
    let (fixture, pipeline_output) = completed_pipeline_fixture();
    assert!(
        !pipeline_output.status.success(),
        "the completed pipeline handoff must stop at its broken checkpoint"
    );
    let pipeline_handoff = gitlab_landing_attempt(&fixture.repo_path)
        .expect("completed pipeline handoff must remain durable");
    assert_eq!(pipeline_handoff["action"], "pipeline_start");
    assert_eq!(pipeline_handoff["state"], "completed");
    let pipeline_operation_id = pipeline_handoff["operation_id"]
        .as_str()
        .expect("pipeline operation ID")
        .to_string();

    fs::write(
        &fixture.landing_request_fail,
        "merge-train-applied-response-fail\n",
    )
    .expect("lose the response after accepting the merge-train POST");
    let first = fixture
        .land_command(&PIPELINE_LAND_ARGS)
        .output()
        .expect("queue merge train");
    let first_text = format!(
        "{}{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        first.status.success(),
        "structured Land must report the ambiguous queue POST in its payload"
    );
    assert!(
        first_text.contains("connection lost after adding MR to merge train"),
        "{first_text}"
    );
    assert!(
        fixture
            .queued_request()
            .contains("merge_trains/merge_requests/42"),
        "the fake provider must accept the initial queue POST"
    );
    let attempts_path = std::path::PathBuf::from(format!("{}.attempts", fixture.queued.display()));
    assert_eq!(
        fs::read_to_string(&attempts_path)
            .expect("read merge-train attempts")
            .lines()
            .count(),
        1
    );

    let config_path = fixture.repo_path.join(".git/gg/config.json");
    let config_before = fs::read(&config_path).expect("snapshot config");
    let config: Value = serde_json::from_slice(&config_before).expect("parse config");
    assert_eq!(
        config["stacks"]["land-wait"]["gitlab_landing_attempt"], pipeline_handoff,
        "queue recovery must not replace the completed pipeline handoff"
    );
    let queue_attempt = &config["stacks"]["land-wait"]["gitlab_merge_train_queue_attempt"];
    assert_eq!(queue_attempt["action"], "queue_merge_train");
    assert_eq!(queue_attempt["mr_number"], 42);
    assert_eq!(queue_attempt["target_branch"], "main");
    let queue_operation_id = queue_attempt["operation_id"]
        .as_str()
        .expect("queue operation ID")
        .to_string();
    assert_ne!(queue_operation_id, pipeline_operation_id);
    let queue_operation = land_operation_records(&fixture.repo_path)
        .into_iter()
        .find(|record| record["id"] == queue_operation_id)
        .expect("fresh queue Land operation");
    assert_eq!(queue_operation["status"], "committed");
    assert_eq!(queue_operation["touched_remote"], true);
    assert!(queue_operation["remote_effects"]
        .as_array()
        .is_some_and(Vec::is_empty));

    let operations_dir = fixture.repo_path.join(".git/gg/operations");
    let snapshot_operations = || {
        let mut files = fs::read_dir(&operations_dir)
            .expect("list operations")
            .map(|entry| {
                let path = entry.expect("operation entry").path();
                (
                    path.file_name().unwrap().to_os_string(),
                    fs::read(path).expect("read operation"),
                )
            })
            .collect::<Vec<_>>();
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    };
    let operations_before = snapshot_operations();
    let refs_before = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&fixture.repo_path)
        .output()
        .expect("snapshot refs")
        .stdout;
    fs::write(&fixture.merge_settings_fail, "forbid-all")
        .expect("forbid every provider process access");

    let restart = fixture
        .land_command(&PIPELINE_LAND_ARGS)
        .output()
        .expect("restart Land");
    let restart_text = format!(
        "{}{}",
        String::from_utf8_lossy(&restart.stdout),
        String::from_utf8_lossy(&restart.stderr)
    );
    assert!(!restart.status.success(), "restart must fail closed");
    assert!(restart_text.contains("MR !42"), "{restart_text}");
    assert!(restart_text.contains(&queue_operation_id), "{restart_text}");
    assert!(
        restart_text.contains("will not be retried automatically"),
        "{restart_text}"
    );
    assert_eq!(
        fs::read_to_string(attempts_path)
            .expect("read merge-train attempts after restart")
            .lines()
            .count(),
        1,
        "restart retried the merge-train POST"
    );
    assert!(!fixture
        .merge_settings_fail
        .with_extension("accessed")
        .exists());
    assert_eq!(fs::read(config_path).expect("read config"), config_before);
    assert_eq!(snapshot_operations(), operations_before);
    let refs_after = Command::new("git")
        .args(["for-each-ref", "--format=%(refname) %(objectname)"])
        .current_dir(&fixture.repo_path)
        .output()
        .expect("read refs")
        .stdout;
    assert_eq!(refs_after, refs_before);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_auto_merge_recovery_rejects_wrong_operation_kind() {
    for invalid_field in ["id", "schema", "kind", "status", "untouched", "effects"] {
        let fixture = WaitingLandFixture::new();
        fixture.use_gitlab_provider();
        fs::write(
            &fixture.landing_request_fail,
            "auto-merge-applied-response-fail\n",
        )
        .expect("fail the response after enabling auto-merge");
        let args = ["land", "--json", "--auto-merge", "--no-clean"];
        let _ = fixture.land_command(&args).output().expect("seed attempt");

        let attempt = gitlab_landing_attempt(&fixture.repo_path).expect("saved queue attempt");
        let operation_id = attempt["operation_id"]
            .as_str()
            .expect("saved operation ID")
            .to_string();
        let operation_path = fixture
            .repo_path
            .join(format!(".git/gg/operations/{operation_id}.json"));
        let mut operation: Value =
            serde_json::from_slice(&fs::read(&operation_path).expect("read operation"))
                .expect("parse operation");
        match invalid_field {
            "id" => operation["id"] = "wrong-operation-id".into(),
            "schema" => operation["schema_version"] = 0.into(),
            "kind" => operation["kind"] = "sync".into(),
            "status" => operation["status"] = "invalid".into(),
            "untouched" => operation["touched_remote"] = false.into(),
            "effects" => {
                operation["remote_effects"] = serde_json::json!([{
                    "kind": "pushed", "remote": "origin", "branch": "topic", "force": false
                }])
            }
            _ => unreachable!(),
        }
        let corrupt_operation = serde_json::to_vec_pretty(&operation).expect("serialize operation");
        fs::write(&operation_path, &corrupt_operation).expect("corrupt operation evidence");
        fs::write(&fixture.merge_settings_fail, "forbid-all").expect("forbid provider access");

        let operations_dir = fixture.repo_path.join(".git/gg/operations");
        let operation_count = fs::read_dir(&operations_dir)
            .expect("list operations")
            .count();
        let config_path = fixture.repo_path.join(".git/gg/config.json");
        let config_before = fs::read(&config_path).expect("snapshot config");

        let restart = fixture.land_command(&args).output().expect("restart Land");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&restart.stdout),
            String::from_utf8_lossy(&restart.stderr)
        );
        assert!(!restart.status.success(), "{invalid_field}: {text}");
        assert!(
            text.contains(&format!(
                "Inconsistent queue_auto_merge recovery evidence for operation {operation_id}"
            )),
            "{invalid_field}: {text}"
        );
        assert_eq!(
            fs::read_to_string(format!(
                "{}.attempts",
                fixture.landing_request_fail.display()
            ))
            .expect("read provider mutation attempts")
            .lines()
            .count(),
            1,
            "recovery issued a provider mutation"
        );
        assert_eq!(
            fs::read_dir(operations_dir)
                .expect("list operations after recovery")
                .count(),
            operation_count,
            "recovery created an operation"
        );
        assert_eq!(fs::read(config_path).expect("read config"), config_before);
        assert_eq!(
            fs::read(operation_path).expect("read corrupt operation"),
            corrupt_operation
        );
        assert!(!fixture
            .merge_settings_fail
            .with_extension("accessed")
            .exists());
    }
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_auto_merge_success_clears_attempt_after_finalization() {
    for (mode, action) in [
        (None, "queued"),
        (Some("already-auto-merge"), "already_queued"),
    ] {
        let fixture = WaitingLandFixture::new();
        fixture.use_gitlab_provider();
        if let Some(mode) = mode {
            fs::write(&fixture.landing_request_fail, mode).expect("set provider result");
        }
        let output = fixture
            .land_command(&["land", "--json", "--auto-merge", "--no-clean"])
            .output()
            .expect("run Land");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let response: Value = serde_json::from_slice(&output.stdout).expect("parse Land response");
        assert_eq!(response["land"]["landed"][0]["action"], action);
        let attempts = fs::read_to_string(format!("{}.attempts", fixture.queued.display()))
            .expect("read auto-merge PUT attempts");
        assert_eq!(attempts.lines().count(), 1, "provider PUT count");
        let records = land_operation_records(&fixture.repo_path);
        assert_eq!(records.len(), 1, "expected one Land operation: {records:?}");
        assert_eq!(records[0]["status"], "committed");
        assert!(
            gitlab_landing_attempt(&fixture.repo_path).is_none(),
            "retained attempt"
        );
    }
}

#[cfg(unix)]
#[test]
fn test_land_direct_merge_failure_records_remote_uncertainty_before_attempt() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.fail_gitlab_landing_request();
    assert_failed_landing_attempt_is_committed(&fixture, &["land", "--json", "--no-clean"]);
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_disconnect_after_squash_checkpoint_recovers_before_merge() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fs::write(&fixture.squash_option, "default_on").expect("set fake squash option");
    fs::write(&fixture.mr_squash, "on\n").expect("enable fake MR squash");
    fs::write(&fixture.landing_request_fail, "squash-block-success\n")
        .expect("block squash response");

    let args = ["land", "--jsonl", "--no-clean", "--no-squash"];
    let mut land = fixture.start_land_with_args(&args);
    let stdout = land.stdout.take().expect("capture land stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read start event");
    assert_eq!(
        serde_json::from_str::<Value>(line.trim()).unwrap()["event"],
        "start"
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    let started = fixture.landing_request_started();
    while !started.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(started.exists(), "squash PUT never started");
    drop(reader);
    fixture.release_landing_request();

    let output = land.wait_with_output().expect("wait for broken pipe");
    assert!(!output.status.success(), "broken checkpoint pipe must fail");
    assert!(!fixture.merged.exists(), "merge must wait for checkpoint");
    let config_path = fixture.repo_path.join(".git/gg/config.json");
    let config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert_eq!(
        config["stacks"]["land-wait"]["gitlab_landing_attempt"]["state"],
        "completed"
    );

    fs::remove_file(&fixture.landing_request_fail).expect("unblock restart");
    let restart = fixture
        .start_land_with_args(&args)
        .wait_with_output()
        .unwrap();
    let stdout = String::from_utf8(restart.stdout).unwrap();
    assert!(restart.status.success(), "{stdout}");
    let events: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let event_pos = |name| {
        events
            .iter()
            .position(|event| event["event"] == name)
            .unwrap()
    };
    assert!(event_pos("start") < event_pos("squash_checkpoint"));
    let config: Value = serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();
    assert!(config["stacks"]["land-wait"]
        .get("gitlab_landing_attempt")
        .is_none());
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_disconnect_after_successful_merge_train_queue_finalizes_operation() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.block_then_succeed_gitlab_landing_request();

    let output = disconnect_jsonl_after_landing_request_started(
        &fixture,
        &["land", "--jsonl", "--no-clean"],
    );
    assert!(!output.status.success(), "disconnected JSONL must fail");
    assert!(
        fixture.queued.exists(),
        "MR must be queued before streaming"
    );

    let records = land_operation_records(&fixture.repo_path);
    assert_eq!(records.len(), 1, "expected one land operation: {records:?}");
    assert_eq!(records[0]["status"], "committed");
    assert_eq!(records[0]["touched_remote"], true);
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_disconnect_after_successful_auto_merge_queue_finalizes_operation() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.block_then_succeed_gitlab_landing_request();

    let output = disconnect_jsonl_after_landing_request_started(
        &fixture,
        &["land", "--jsonl", "--auto-merge", "--no-clean"],
    );
    assert!(!output.status.success(), "disconnected JSONL must fail");
    assert!(
        fixture.queued.exists(),
        "MR must be queued before streaming"
    );

    let records = land_operation_records(&fixture.repo_path);
    assert_eq!(records.len(), 1, "expected one land operation: {records:?}");
    assert_eq!(records[0]["status"], "committed");
    assert_eq!(records[0]["touched_remote"], true);
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_disconnect_after_successful_direct_merge_finalizes_operation() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.block_then_succeed_gitlab_landing_request();

    let output = disconnect_jsonl_after_landing_request_started(
        &fixture,
        &["land", "--jsonl", "--no-clean"],
    );
    assert!(!output.status.success(), "disconnected JSONL must fail");
    assert!(
        fixture.merged.with_extension("glab").exists(),
        "MR must be merged before streaming"
    );

    let records = land_operation_records(&fixture.repo_path);
    assert_eq!(records.len(), 1, "expected one land operation: {records:?}");
    assert_eq!(records[0]["status"], "committed");
    assert_eq!(records[0]["touched_remote"], true);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_no_squash_preserves_remote_uncertainty_before_update() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fs::write(&fixture.squash_option, "default_on").expect("set fake squash option");
    fs::write(&fixture.mr_squash, "on\n").expect("enable fake MR squash");
    fixture.fail_gitlab_squash_update_response();

    let output = fixture
        .start_land_with_args(&["land", "--json", "--no-clean", "--no-squash"])
        .wait_with_output()
        .expect("wait for land");

    assert!(
        !output.status.success(),
        "land must report the failed update"
    );
    assert!(
        fixture.squash_disabled.exists(),
        "the fake GitLab server must apply the squash update before the response fails"
    );
    assert!(!fixture.queued.exists(), "the landing request must not run");

    let records = land_operation_records(&fixture.repo_path);
    assert_eq!(records.len(), 1, "expected one land operation: {records:?}");
    assert_eq!(records[0]["status"], "pending");
    assert_eq!(
        records[0]["touched_remote"], true,
        "an ambiguous squash-update outcome must be persisted before the request: {}",
        records[0]
    );
    assert!(records[0]["remote_effects"].as_array().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn test_land_readiness_merged_reports_entry_without_requeueing() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.complete_merge_train_status();

    let output = fixture
        .start_land_with_args(&["land", "--wait", "--jsonl", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(output.status.success(), "land failed: {stderr}");
    assert!(
        !fixture.queued.exists(),
        "merged MR must not be queued again"
    );

    let events: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse JSONL event"))
        .collect();
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event"] == "entry" && event["action"] == "merged")
            .count(),
        1,
        "expected exactly one merged entry: {stdout}"
    );
    let summary = events.last().expect("summary event");
    assert_eq!(summary["event"], "summary");
    assert_eq!(summary["remaining"], 0);
    assert!(summary["error"].is_null(), "{summary}");
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_disconnect_after_readiness_observes_merged_train_finalizes_cleanup() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");
    fs::write(&fixture.unapproved, "unapproved\n").expect("hold readiness before merged status");

    let mut land = fixture.start_land_with_args(&["land", "--wait", "--jsonl", "--no-clean"]);
    let stdout = land.stdout.take().expect("capture land stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read land start event");
    let start: Value = serde_json::from_str(line.trim()).expect("parse land start event");
    assert_eq!(start["event"], "start");
    line.clear();
    reader
        .read_line(&mut line)
        .expect("read initial readiness wait event");
    let wait: Value = serde_json::from_str(line.trim()).expect("parse readiness wait event");
    assert_eq!(wait["event"], "wait");
    assert_eq!(wait["phase"], "readiness");

    fs::write(&fixture.train_merged, "block\n").expect("block fake merged-train status");
    let status_started =
        std::path::PathBuf::from(format!("{}.started", fixture.train_merged.display()));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !status_started.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        status_started.exists(),
        "land never started the terminal merge-train status request"
    );
    drop(reader);
    fs::write(
        format!("{}.release", fixture.train_merged.display()),
        "release\n",
    )
    .expect("release fake merged-train status");

    let output = land.wait_with_output().expect("wait for land");
    assert!(
        !output.status.success(),
        "terminal merged entry must observe the disconnected JSONL consumer"
    );
    assert!(
        !fixture.queued.exists(),
        "an MR already merged through the train must not be queued again"
    );

    let config: Value = serde_json::from_slice(
        &fs::read(fixture.repo_path.join(".git/gg/config.json")).expect("read persisted config"),
    )
    .expect("parse persisted config");
    assert!(
        config["stacks"]["land-wait"]["mrs"]
            .get("c-1111111")
            .is_none(),
        "merged MR mapping must be persisted before the terminal event: {config}"
    );

    let records = land_operation_records(&fixture.repo_path);
    assert!(
        !records.is_empty(),
        "expected cleanup operations: {records:?}"
    );
    assert!(
        records.iter().all(|record| record["status"] == "committed"),
        "all cleanup operations must be finalized: {records:?}"
    );
    assert!(
        records
            .iter()
            .all(|record| record["touched_remote"] == true),
        "all cleanup operations must retain remote-touch metadata: {records:?}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_disconnect_after_merge_train_merge_finalizes_cleanup() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");

    let mut land =
        fixture.start_land_with_args(&["land", "--all", "--wait", "--jsonl", "--no-clean"]);
    let stdout = land.stdout.take().expect("capture land stdout");
    let mut reader = BufReader::new(stdout);
    let mut saw_queued = false;
    loop {
        let mut line = String::new();
        assert!(
            reader.read_line(&mut line).expect("read JSONL event") > 0,
            "land exited before entering merge-train wait"
        );
        let event: Value = serde_json::from_str(line.trim()).expect("parse JSONL event");
        if event["event"] == "entry"
            && matches!(event["action"].as_str(), Some("queued" | "already_queued"))
        {
            saw_queued = true;
        }
        if saw_queued && event["event"] == "wait" && event["phase"] == "merge_train" {
            break;
        }
    }

    fixture.complete_merge_train_status();
    drop(reader);

    let output = land.wait_with_output().expect("wait for land");
    assert!(
        !output.status.success(),
        "terminal merged event must observe the disconnected JSONL consumer"
    );

    let config: Value = serde_json::from_slice(
        &fs::read(fixture.repo_path.join(".git/gg/config.json")).expect("read persisted config"),
    )
    .expect("parse persisted config");
    assert!(
        config["stacks"]["land-wait"]["mrs"]
            .get("c-1111111")
            .is_none(),
        "merged MR mapping must be persisted before the terminal event: {config}"
    );

    let records = land_operation_records(&fixture.repo_path);
    assert_eq!(
        records.len(),
        2,
        "expected queue and cleanup operations: {records:?}"
    );
    assert!(
        records.iter().all(|record| record["status"] == "committed"),
        "both operation segments must be finalized: {records:?}"
    );
    assert!(
        records
            .iter()
            .all(|record| record["touched_remote"] == true),
        "both operation segments must retain remote-touch metadata: {records:?}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_disconnect_after_merge_train_requeue_finalizes_operation() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fixture.set_glab_pipeline_status("running");
    fs::write(&fixture.landing_request_fail, "block-requeue-success\n")
        .expect("block the requeue request");

    let original_branch = run_git(&fixture.repo_path, &["branch", "--show-current"]).1;
    let original_head = run_git(&fixture.repo_path, &["rev-parse", "HEAD"]).1;
    let original_local_refs = run_git(&fixture.repo_path, &["show-ref"]).1;
    let original_remote_refs = Command::new("git")
        .args([
            "--git-dir",
            fixture.remote_path.to_str().unwrap(),
            "show-ref",
        ])
        .output()
        .expect("read original remote refs")
        .stdout;

    let mut land =
        fixture.start_land_with_args(&["land", "--all", "--wait", "--jsonl", "--no-clean"]);
    let stdout = land.stdout.take().expect("capture land stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read land start event");
    let start: Value = serde_json::from_str(line.trim()).expect("parse land start event");
    assert_eq!(start["event"], "start");

    fixture.wait_until_queued();
    let initial_operation_ids: Vec<String> = land_operation_records(&fixture.repo_path)
        .iter()
        .map(|record| record["id"].as_str().expect("operation id").to_string())
        .collect();
    assert_eq!(
        initial_operation_ids.len(),
        1,
        "initial queue must have exactly one finalized segment"
    );
    fs::write(&fixture.head_moved, "moved\n").expect("move fake MR head");
    fs::write(&fixture.auto_merge_dropped, "dropped\n").expect("drop fake auto-merge");

    let deadline = Instant::now() + Duration::from_secs(60);
    let request_started = fixture.landing_request_started();
    while !request_started.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(request_started.exists(), "requeue request never started");
    let pending: Vec<Value> = land_operation_records(&fixture.repo_path)
        .into_iter()
        .filter(|record| record["status"] == "pending")
        .collect();
    assert_eq!(
        pending.len(),
        1,
        "expected exact pending requeue: {pending:?}"
    );
    let requeue_operation_id = pending[0]["id"]
        .as_str()
        .expect("pending requeue operation id")
        .to_string();
    assert_eq!(
        pending[0]["touched_remote"], true,
        "uncertainty must be durable before the requeue request"
    );
    fixture.release_landing_request();
    while !fixture.requeued.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        fixture.requeued.exists(),
        "land never re-queued the MR before the JSONL disconnect"
    );
    drop(reader);

    let output = land.wait_with_output().expect("wait for land");
    assert!(
        !output.status.success(),
        "land must fail when its JSONL consumer disconnects"
    );
    assert_eq!(
        run_git(&fixture.repo_path, &["branch", "--show-current"]).1,
        original_branch,
        "land must preserve the checkout"
    );
    assert_eq!(
        run_git(&fixture.repo_path, &["rev-parse", "HEAD"]).1,
        original_head,
        "land must preserve HEAD"
    );
    assert_eq!(
        run_git(&fixture.repo_path, &["show-ref"]).1,
        original_local_refs,
        "land must preserve local refs"
    );
    let remote_refs = Command::new("git")
        .args([
            "--git-dir",
            fixture.remote_path.to_str().unwrap(),
            "show-ref",
        ])
        .output()
        .expect("read final remote refs")
        .stdout;
    assert_eq!(
        remote_refs, original_remote_refs,
        "land must preserve remote refs"
    );

    let records = land_operation_records(&fixture.repo_path);
    let requeue_record = records
        .iter()
        .find(|record| record["id"].as_str() == Some(requeue_operation_id.as_str()))
        .expect("same requeue operation after disconnect");
    assert_eq!(
        requeue_record["status"], "committed",
        "the requeue operation must be finalized before the broken-pipe failure returns: {}",
        requeue_record
    );
    assert_eq!(requeue_record["touched_remote"], true);
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_no_squash_refused_when_project_requires_squash() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.enable_gitlab_merge_trains();
    fs::write(&fixture.squash_option, "always").expect("set fake squash option");

    let output = fixture
        .start_land_with_args(&["land", "--all", "--json", "--no-clean", "--no-squash"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(!output.status.success(), "land must refuse: {stdout}");
    assert!(
        format!("{stdout}{stderr}").contains("requires squashing"),
        "stdout={stdout} stderr={stderr}"
    );
    assert!(!fixture.queued.exists(), "no MR may be queued");
    assert!(!fixture.merged.exists(), "no MR may be merged");
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_refuses_when_project_settings_cannot_be_read() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.fail_gitlab_merge_settings_lookup();

    let output = fixture
        .start_land_with_args(&["land", "--json", "--no-clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(!output.status.success(), "land must fail closed: {stdout}");
    assert!(
        format!("{stdout}{stderr}").contains("project merge settings"),
        "error should explain that GitLab project merge settings could not be read: stdout={stdout} stderr={stderr}"
    );
    assert!(!fixture.queued.exists(), "MR must not be queued");
    assert!(!fixture.merged.exists(), "MR must not be merged");
    assert!(
        land_operation_records(&fixture.repo_path).is_empty(),
        "provider preflight failure must not leave a pending operation"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_no_squash_refuses_when_project_settings_cannot_be_read() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.fail_gitlab_merge_settings_lookup();

    let output = fixture
        .start_land_with_args(&["land", "--json", "--no-clean", "--no-squash"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(!output.status.success(), "land must fail closed: {stdout}");
    assert!(
        format!("{stdout}{stderr}").contains("project merge settings"),
        "error should explain that GitLab project merge settings could not be read: stdout={stdout} stderr={stderr}"
    );
    assert!(
        !fixture.squash_disabled.exists(),
        "MR squash must not be changed"
    );
    assert!(!fixture.queued.exists(), "MR must not be queued");
    assert!(!fixture.merged.exists(), "MR must not be merged");
    assert!(
        land_operation_records(&fixture.repo_path).is_empty(),
        "provider preflight failure must not leave a pending operation"
    );
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_no_squash_refuses_unparseable_project_settings() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.return_invalid_gitlab_merge_settings();

    let output = fixture
        .start_land_with_args(&["land", "--json", "--no-clean", "--no-squash"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(!output.status.success(), "land must fail closed: {stdout}");
    assert!(
        format!("{stdout}{stderr}").contains("parse GitLab project merge settings"),
        "error should explain that GitLab project merge settings could not be parsed: stdout={stdout} stderr={stderr}"
    );
    assert!(
        !fixture.squash_disabled.exists(),
        "MR squash must not be changed"
    );
    assert!(!fixture.queued.exists(), "MR must not be queued");
    assert!(!fixture.merged.exists(), "MR must not be merged");
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_no_squash_refuses_missing_squash_option() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fixture.omit_gitlab_squash_option();

    let output = fixture
        .start_land_with_args(&["land", "--json", "--no-clean", "--no-squash"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(!output.status.success(), "land must fail closed: {stdout}");
    assert!(
        format!("{stdout}{stderr}")
            .to_ascii_lowercase()
            .contains("could not establish"),
        "error should explain that the squash policy is unknown: stdout={stdout} stderr={stderr}"
    );
    assert!(
        !fixture.squash_disabled.exists(),
        "MR squash must not be changed"
    );
    assert!(!fixture.queued.exists(), "MR must not be queued");
    assert!(!fixture.merged.exists(), "MR must not be merged");
}

#[cfg(unix)]
#[test]
fn test_land_gitlab_no_squash_refuses_unrecognized_squash_option() {
    let fixture = WaitingLandFixture::new();
    fixture.use_gitlab_provider();
    fs::write(&fixture.squash_option, "unexpected").expect("set unknown squash option");

    let output = fixture
        .start_land_with_args(&["land", "--json", "--no-clean", "--no-squash"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(!output.status.success(), "land must fail closed: {stdout}");
    assert!(
        format!("{stdout}{stderr}")
            .to_ascii_lowercase()
            .contains("could not establish"),
        "error should explain that the squash policy is unknown: stdout={stdout} stderr={stderr}"
    );
    assert!(
        !fixture.squash_disabled.exists(),
        "MR squash must not be changed"
    );
    assert!(!fixture.queued.exists(), "MR must not be queued");
    assert!(!fixture.merged.exists(), "MR must not be merged");
}

#[cfg(unix)]
#[test]
fn test_land_jsonl_clean_does_not_prompt_for_a_configured_worktree() {
    let fixture = WaitingLandFixture::new();
    fixture.release_ci();
    let config_path = fixture.repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).expect("read config"))
        .expect("parse config");
    config["stacks"]["land-wait"]["worktree_path"] = Value::String(
        fixture
            .repo_path
            .join("configured-worktree")
            .display()
            .to_string(),
    );
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).expect("serialize config"),
    )
    .expect("write config");

    let output = fixture
        .start_land_with_args(&["land", "--jsonl", "--clean"])
        .wait_with_output()
        .expect("wait for land");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(
        output.status.success(),
        "land failed: stdout={stdout} stderr={stderr}"
    );
    assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    assert!(
        stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()),
        "every JSONL line must parse: {stdout}"
    );
    let summary: Value = serde_json::from_str(stdout.lines().last().expect("summary event"))
        .expect("parse summary event");
    assert_eq!(summary["cleaned"], false);
    assert!(
        summary["warnings"]
            .as_array()
            .expect("warnings must be an array")
            .iter()
            .any(|warning| warning
                .as_str()
                .is_some_and(|warning| warning.contains("worktree"))),
        "summary should explain why cleanup was skipped: {summary}"
    );
}

#[cfg(unix)]
#[test]
fn test_land_wait_releases_operation_lock_for_sync_in_another_worktree() {
    let fixture = WaitingLandFixture::new();
    let land = fixture.start_land();
    fixture.wait_until_polling();

    let sync = Command::new(env!("CARGO_BIN_EXE_gg"))
        .arg("sync")
        .current_dir(&fixture.other_worktree)
        .env("HOME", &fixture.test_home)
        .output()
        .expect("run sync in other worktree");
    let config_path = fixture.repo_path.join(".git/gg/config.json");
    let mut concurrent_config: Value =
        serde_json::from_slice(&fs::read(&config_path).expect("read config after concurrent sync"))
            .expect("parse config after concurrent sync");
    concurrent_config["stacks"]["other"] = serde_json::json!({"base":"main"});
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&concurrent_config).expect("serialize concurrent config"),
    )
    .expect("simulate unrelated stack config update");
    fixture.release_ci();
    let land_output = land.wait_with_output().expect("wait for land");
    assert!(
        sync.status.success(),
        "sync should run while land is waiting: {}",
        String::from_utf8_lossy(&sync.stderr)
    );
    assert!(
        land_output.status.success(),
        "land should finish: stdout={} stderr={}",
        String::from_utf8_lossy(&land_output.stdout),
        String::from_utf8_lossy(&land_output.stderr)
    );
    assert!(
        fixture.merged.exists(),
        "land should merge after CI becomes ready"
    );
    let final_config: Value =
        serde_json::from_slice(&fs::read(&config_path).expect("read config after land"))
            .expect("parse config after land");
    assert_eq!(final_config["stacks"]["other"]["base"], "main");

    let operations_dir = fixture.repo_path.join(".git/gg/operations");
    for entry in fs::read_dir(operations_dir).expect("read operation records") {
        let path = entry.expect("read operation record entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let record: Value = serde_json::from_slice(&fs::read(path).expect("read operation record"))
            .expect("parse operation record");
        assert_eq!(record["status"], "committed");
    }
}

#[cfg(unix)]
#[test]
fn test_land_wait_aborts_when_target_stack_changes() {
    let fixture = WaitingLandFixture::new();
    let land = fixture.start_land();
    fixture.wait_until_polling();

    fs::write(fixture.repo_path.join("new.txt"), "new\n").expect("write new entry");
    run_git(&fixture.repo_path, &["add", "new.txt"]);
    run_git(
        &fixture.repo_path,
        &["commit", "-m", "New entry\n\nGG-ID: c-2222222"],
    );

    fixture.release_ci();
    let output = land.wait_with_output().expect("wait for land");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "land should reject stale stack state"
    );
    assert!(
        stderr.contains("Stack changed while gg land was waiting"),
        "unexpected error: {stderr}"
    );
    assert!(
        !fixture.merged.exists(),
        "land must not merge after its target stack changes"
    );
}

#[cfg(unix)]
#[test]
fn test_land_wait_rechecks_readiness_after_reacquiring_lock() {
    let fixture = WaitingLandFixture::new();
    fixture.release_ci();
    fs::write(&fixture.regress, "regress once\n").expect("enable readiness regression");

    let land = fixture.start_land();
    let output = land.wait_with_output().expect("wait for land");
    assert!(
        output.status.success(),
        "land should resume waiting and finish: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let ci_calls: u32 = fs::read_to_string(&fixture.ci_calls)
        .expect("read CI call count")
        .trim()
        .parse()
        .expect("parse CI call count");
    assert!(
        ci_calls >= 4,
        "land should recheck readiness and observe the regression; calls={ci_calls}"
    );
}

#[test]
fn test_gg_land_help_has_until() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success);
    assert!(stdout.contains("--until"));
}

#[test]
fn test_gg_land_json_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success);
    assert!(stdout.contains("--json"));
}

#[test]
fn test_gg_land_jsonl_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success);
    assert!(stdout.contains("--jsonl"), "help should mention --jsonl");
}

#[test]
fn test_gg_land_rejects_json_and_jsonl_together() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, _stdout, stderr) = run_gg(&repo_path, &["land", "--json", "--jsonl"]);

    assert!(!success, "--json and --jsonl should conflict");
    assert!(stderr.contains("cannot be used with"));
}

#[test]
fn test_gg_land_json_error_without_provider() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "json-land-error"]);
    assert!(success, "Failed to create stack: {}", stderr);

    let (success, stdout, stderr) = run_gg(&repo_path, &["land", "--json"]);
    assert!(!success, "land --json should fail without provider");
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSON mode"
    );

    let parsed: Value = serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["version"], 1);
    assert!(parsed["error"].is_string(), "error field must be string");
}

#[test]
fn test_gg_land_jsonl_error_without_provider() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "jsonl-land-error"]);
    assert!(success, "Failed to create stack: {stderr}");

    let (success, stdout, stderr) = run_gg(&repo_path, &["land", "--jsonl"]);
    assert!(!success, "land --jsonl should fail without provider");
    assert!(
        stderr.trim().is_empty(),
        "stderr should be empty in JSONL mode: {stderr}"
    );

    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "fatal error should emit one event");
    let event: Value = serde_json::from_str(lines[0]).expect("error line must be valid JSON");
    assert_eq!(event["version"], 1);
    assert_eq!(event["command"], "land");
    assert_eq!(event["status"], "error");
    assert_eq!(event["event"], "error");
    assert!(event["message"].is_string());
}

#[test]
fn test_land_help_shows_no_squash_option() {
    let (_temp_dir, repo_path) = create_test_repo();

    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success, "Help should succeed");
    assert!(
        stdout.contains("--no-squash"),
        "Should show --no-squash option: {}",
        stdout
    );
    assert!(
        stdout.contains("squash") && stdout.contains("default"),
        "Should mention squash is default: {}",
        stdout
    );
}

#[test]
fn test_land_help_shows_admin_option() {
    let (_temp_dir, repo_path) = create_test_repo();

    let (success, stdout, _stderr) = run_gg(&repo_path, &["land", "--help"]);

    assert!(success, "Help should succeed");
    assert!(
        stdout.contains("--admin"),
        "Should show --admin option: {}",
        stdout
    );
    assert!(
        stdout.contains("GitHub only") || stdout.contains("GitHub-only"),
        "Should indicate --admin is GitHub-only: {}",
        stdout
    );
}

#[test]
fn test_land_admin_flag_accepted() {
    // Test that the --admin flag is recognized and doesn't cause a clap error
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config with username
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    let (success, _, stderr) = run_gg(&repo_path, &["co", "test-stack"]);
    assert!(success, "Failed to create stack: {}", stderr);

    // Verify --admin flag is accepted (it will fail for other reasons,
    // like no PRs to land, but should not fail on unknown argument)
    let (_, _stdout, stderr) = run_gg(&repo_path, &["land", "--admin"]);

    // Should not contain clap errors
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("invalid value"),
        "The --admin flag should be recognized, stderr: {}",
        stderr
    );
}

#[test]
fn test_land_admin_config_default() {
    // Test that land_admin config defaults to false (not present in minimal config)
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(gg_dir.join("config.json"), r#"{"defaults":{}}"#).expect("Failed to write config");

    let config_path = gg_dir.join("config.json");
    let content = fs::read_to_string(config_path).expect("Failed to read config");

    assert!(
        !content.contains("land_admin"),
        "Default config should not contain land_admin when false"
    );
}

#[test]
fn test_land_admin_config_enabled() {
    // Test that land_admin can be set to true in config
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"land_admin":true}}"#,
    )
    .expect("Failed to write config");

    let config_path = gg_dir.join("config.json");
    let content = fs::read_to_string(config_path).expect("Failed to read config");

    assert!(
        content.contains("\"land_admin\":true"),
        "Config should contain land_admin when enabled"
    );
}

#[cfg(unix)]
#[test]
fn land_persists_removal_of_only_the_successfully_merged_entry_intent() {
    let fixture = WaitingLandFixture::new();
    fixture.add_second_entry();
    let config_path = fixture.repo_path.join(".git/gg/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["stacks"]["land-wait"]["review_creation_intents"] = serde_json::json!({
        "c-1111111": {
            "source_branch": "testuser/land-wait--c-1111111",
            "target_branch": "main",
            "origin_operation_id": "op_0000000000041_landed"
        },
        "c-2222222": {
            "source_branch": "testuser/land-wait--c-2222222",
            "target_branch": "testuser/land-wait--c-1111111",
            "origin_operation_id": "op_0000000000042_pending"
        }
    });
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let stdout_path = fixture.repo_path.join("land-intent-cleanup.stdout");
    let stderr_path = fixture.repo_path.join("land-intent-cleanup.stderr");
    let mut command = fixture.land_command(&["land", "--wait", "--no-clean"]);
    command
        .stdout(Stdio::from(fs::File::create(&stdout_path).unwrap()))
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
    let mut child = ProcessGroupGuard::spawn(&mut command).expect("start guarded land");

    let readiness_deadline = Instant::now() + Duration::from_secs(30);
    while !fixture.polled.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "land exited before readiness ({status}): stdout={} stderr={}",
                fs::read_to_string(&stdout_path).unwrap_or_default(),
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        assert!(
            Instant::now() < readiness_deadline,
            "land did not reach CI polling readiness"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    fixture.release_ci();

    let execution_deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < execution_deadline,
            "land did not finish before the execution deadline"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    drop(child);
    let stdout = fs::read_to_string(&stdout_path).unwrap();
    let stderr = fs::read_to_string(&stderr_path).unwrap();
    assert!(
        status.success(),
        "land failed: stdout={stdout} stderr={stderr}"
    );

    let persisted: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert!(persisted["stacks"]["land-wait"]["review_creation_intents"]["c-1111111"].is_null());
    assert_eq!(
        persisted["stacks"]["land-wait"]["review_creation_intents"]["c-2222222"]
            ["origin_operation_id"],
        "op_0000000000042_pending"
    );
}
