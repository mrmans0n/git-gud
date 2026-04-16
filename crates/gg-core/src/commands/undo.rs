//! `gg undo` command handler. See design §2.4.
//!
//! Wraps [`crate::operations::run_undo`] with the record-itself pattern: an
//! `Undo` operation is itself recorded in the op log (D5), so the user can
//! run `gg undo; gg undo` to redo the first op.
//!
//! Refusal modes (remote/interrupted/stale/unsupported-schema) intentionally
//! drop the guard without calling `finalize`, which leaves the record as
//! Pending on disk; the next lock-acquiring op will sweep it to Interrupted.
//! This is fine — we did not mutate anything.

use console::style;

use crate::config::Config;
use crate::error::{GgError, Result};
use crate::git;
use crate::operations::{
    self, OperationKind, OperationRecord, SnapshotScope, UndoOptions, UndoOutcome,
};
use crate::output::{
    print_json, OperationSummaryJson, UndoJsonStatus, UndoListResponse, UndoRefusalJson,
    UndoRefusalReason, UndoResponse, OUTPUT_VERSION,
};

/// Options for the undo command.
#[derive(Debug, Default)]
pub struct UndoCliOptions {
    /// When true, list recent operations instead of undoing.
    pub list: bool,
    /// Target a specific operation id (most-recent-undoable when `None`).
    pub operation_id: Option<String>,
    /// Emit machine-readable JSON.
    pub json: bool,
    /// Limit for `--list`. Defaults to 100 when 0 (matches the op-log cap).
    pub limit: usize,
}

/// Run the undo command.
pub fn run(options: UndoCliOptions) -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;

    if options.list {
        return run_list(&repo, &options);
    }
    run_undo(&repo, &config, &options)
}

fn run_list(repo: &git2::Repository, options: &UndoCliOptions) -> Result<()> {
    let limit = if options.limit == 0 {
        100
    } else {
        options.limit
    };
    let records = operations::list(repo, limit)?;

    if options.json {
        let response = UndoListResponse {
            version: OUTPUT_VERSION,
            operations: records.iter().map(OperationSummaryJson::from).collect(),
        };
        print_json(&response);
    } else {
        print_list_human(&records);
    }
    Ok(())
}

fn run_undo(repo: &git2::Repository, config: &Config, options: &UndoCliOptions) -> Result<()> {
    // Undo itself takes a lock and records itself (D5). Use AllUserBranches
    // scope so the undo record's refs_before captures whatever user-owned
    // state existed before the replay.
    let args: Vec<String> = if let Some(id) = &options.operation_id {
        vec!["undo".into(), id.clone()]
    } else {
        vec!["undo".into()]
    };
    let (_lock, guard) = git::acquire_operation_lock_and_record(
        repo,
        config,
        OperationKind::Undo,
        args,
        None,
        SnapshotScope::AllUserBranches,
    )?;

    let undo_opts = UndoOptions {
        operation_id: options.operation_id.clone(),
        json: options.json,
    };
    let outcome = operations::run_undo(repo, config, undo_opts)?;

    // On success we capture the post-replay snapshot and finalize. On any
    // refusal we drop the guard without finalize (see module docs).
    match &outcome {
        UndoOutcome::Succeeded(target) => {
            let refs_after =
                operations::snapshot_refs(repo, config, SnapshotScope::AllUserBranches)?;
            guard.finalize_as_undo(refs_after, target.id.clone())?;
        }
        _ => {
            // Drop guard: the Pending record will be swept to Interrupted.
            drop(guard);
        }
    }

    emit_response(&outcome, options.json)?;

    // `emit_response` already printed a detailed human-readable diagnostic
    // (or the structured JSON refusal). For refusals we return the marker
    // `Silenced` error so the CLI exits non-zero without prepending another
    // generic "error: ..." line that would duplicate the refusal message.
    match outcome {
        UndoOutcome::Succeeded(_) => Ok(()),
        UndoOutcome::RefusedRemote { .. }
        | UndoOutcome::RefusedInterrupted(_)
        | UndoOutcome::RefusedStale { .. }
        | UndoOutcome::RefusedUnsupportedSchema(_) => Err(GgError::Silenced),
    }
}

fn emit_response(outcome: &UndoOutcome, json: bool) -> Result<()> {
    if json {
        let response = build_json_response(outcome);
        print_json(&response);
        return Ok(());
    }

    match outcome {
        UndoOutcome::Succeeded(target) => {
            println!(
                "{} Undid {} ({})",
                style("OK").green().bold(),
                short_id(&target.id),
                short_args(&target.args),
            );
        }
        UndoOutcome::RefusedRemote { target, hints } => {
            eprintln!(
                "{} Refusing to undo {}: operation touched a remote.",
                style("Refused").red().bold(),
                short_id(&target.id),
            );
            for hint in hints {
                eprintln!("  {}", hint);
            }
        }
        UndoOutcome::RefusedInterrupted(t) => {
            eprintln!(
                "{} Refusing to undo {}: operation did not finish cleanly.",
                style("Refused").red().bold(),
                short_id(&t.id),
            );
        }
        UndoOutcome::RefusedStale {
            target,
            ref_name,
            expected,
            actual,
        } => {
            eprintln!(
                "{} Refusing to undo {}: ref `{}` moved since the operation (expected `{}`, now `{}`).",
                style("Refused").red().bold(),
                short_id(&target.id),
                ref_name,
                expected,
                actual,
            );
        }
        UndoOutcome::RefusedUnsupportedSchema(target) => {
            eprintln!(
                "{} Refusing to undo {}: schema version {} is newer than this gg ({}).",
                style("Refused").red().bold(),
                short_id(&target.id),
                target.schema_version,
                operations::SCHEMA_VERSION,
            );
        }
    }
    Ok(())
}

fn build_json_response(outcome: &UndoOutcome) -> UndoResponse {
    match outcome {
        UndoOutcome::Succeeded(target) => UndoResponse {
            version: OUTPUT_VERSION,
            status: UndoJsonStatus::Succeeded,
            undone: Some(OperationSummaryJson::from(target)),
            refusal: None,
        },
        UndoOutcome::RefusedRemote { target, hints } => UndoResponse {
            version: OUTPUT_VERSION,
            status: UndoJsonStatus::Refused,
            undone: None,
            refusal: Some(UndoRefusalJson {
                reason: UndoRefusalReason::Remote,
                message: "operation touched a remote".into(),
                target: Some(OperationSummaryJson::from(target)),
                hints: hints.clone(),
            }),
        },
        UndoOutcome::RefusedInterrupted(target) => UndoResponse {
            version: OUTPUT_VERSION,
            status: UndoJsonStatus::Refused,
            undone: None,
            refusal: Some(UndoRefusalJson {
                reason: UndoRefusalReason::Interrupted,
                message: "operation was interrupted".into(),
                target: Some(OperationSummaryJson::from(target)),
                hints: vec![],
            }),
        },
        UndoOutcome::RefusedStale {
            target,
            ref_name,
            expected,
            actual,
        } => UndoResponse {
            version: OUTPUT_VERSION,
            status: UndoJsonStatus::Refused,
            undone: None,
            refusal: Some(UndoRefusalJson {
                reason: UndoRefusalReason::Stale,
                message: format!(
                    "ref `{ref_name}` moved since the operation (expected `{expected}`, now `{actual}`)"
                ),
                target: Some(OperationSummaryJson::from(target)),
                hints: vec![],
            }),
        },
        UndoOutcome::RefusedUnsupportedSchema(target) => UndoResponse {
            version: OUTPUT_VERSION,
            status: UndoJsonStatus::Refused,
            undone: None,
            refusal: Some(UndoRefusalJson {
                reason: UndoRefusalReason::UnsupportedSchema,
                message: format!(
                    "schema version {} newer than this gg",
                    target.schema_version
                ),
                target: Some(OperationSummaryJson::from(target)),
                hints: vec![],
            }),
        },
    }
}

fn print_list_human(records: &[OperationRecord]) {
    if records.is_empty() {
        println!("{}", style("No operations recorded yet.").dim());
        return;
    }
    println!(
        "{:<10}  {:<12}  {:<10}  {:<8}  ARGS",
        "ID", "KIND", "STATUS", "UNDOABLE"
    );
    for r in records {
        let undoable = if r.is_undoable_locally() {
            style("yes").green().to_string()
        } else if r.touched_remote {
            style("remote").red().to_string()
        } else {
            style("no").dim().to_string()
        };
        let status = format!("{:?}", r.status).to_lowercase();
        let kind = format!("{:?}", r.kind).to_lowercase();
        println!(
            "{:<10}  {:<12}  {:<10}  {:<8}  {}",
            short_id(&r.id),
            kind,
            status,
            undoable,
            short_args(&r.args),
        );
    }
}

fn short_id(id: &str) -> String {
    // IDs look like "<unix_ms>_<random>". Show a compact form: last 8 chars
    // of the random suffix, or the whole id if shorter.
    if id.len() <= 10 {
        return id.to_string();
    }
    let tail_len = 8;
    let start = id.len().saturating_sub(tail_len);
    id[start..].to_string()
}

fn short_args(args: &[String]) -> String {
    let joined = args.join(" ");
    // Byte-length cap at 80. When truncating we reserve room for "..." (3 bytes)
    // and take the first 77 bytes, clipped to a char boundary.
    if joined.len() <= 80 {
        joined
    } else {
        let mut cut = 77;
        while cut > 0 && !joined.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}...", &joined[..cut])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_id_truncates_long_ids() {
        let id = "1700000000000_abcdef0123";
        // Last 8 chars of the full id (length 24 → chars [16..24]).
        assert_eq!(short_id(id), "cdef0123");
    }

    #[test]
    fn short_id_keeps_short_ids() {
        assert_eq!(short_id("abc"), "abc");
    }

    #[test]
    fn short_args_truncates_long_strings() {
        let long: Vec<String> = (0..30).map(|i| format!("arg{i}")).collect();
        let s = short_args(&long);
        assert!(s.len() <= 80, "expected <= 80 bytes, got {}", s.len());
        assert!(s.ends_with("..."));
    }

    #[test]
    fn short_args_short_roundtrip() {
        let args = vec!["sync".to_string(), "main".to_string()];
        assert_eq!(short_args(&args), "sync main");
    }
}
