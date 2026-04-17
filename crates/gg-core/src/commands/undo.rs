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
    print!("{}", format_list_human(records, true));
}

fn format_list_human(records: &[OperationRecord], color: bool) -> String {
    if records.is_empty() {
        return format!("{}\n", style("No operations recorded yet.").dim());
    }
    let mut out = format!(
        "{:<10}  {:<12}  {:<10}  {:<8}  ARGS\n",
        "ID", "KIND", "STATUS", "UNDOABLE"
    );
    for r in records {
        let (undoable_text, colorize): (&str, fn(&str) -> String) =
            if r.is_undoable_locally() {
                ("yes", |s| style(s).green().to_string())
            } else if r.touched_remote {
                ("remote", |s| style(s).red().to_string())
            } else {
                ("no", |s| style(s).dim().to_string())
            };
        let undoable = if color {
            colorize(&format!("{:<8}", undoable_text))
        } else {
            format!("{:<8}", undoable_text)
        };
        let status = format!("{:?}", r.status).to_lowercase();
        let kind = format!("{:?}", r.kind).to_lowercase();
        out.push_str(&format!(
            "{:<10}  {:<12}  {:<10}  {}  {}\n",
            short_id(&r.id),
            kind,
            status,
            undoable,
            short_args(&r.args),
        ));
    }
    out
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
    use crate::operations::OperationStatus;

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

    fn make_record(id: &str, kind: OperationKind, touched_remote: bool, args: Vec<String>) -> OperationRecord {
        OperationRecord {
            id: id.to_string(),
            schema_version: crate::operations::SCHEMA_VERSION,
            kind,
            status: OperationStatus::Committed,
            created_at_ms: 0,
            args,
            stack_name: None,
            refs_before: vec![],
            refs_after: vec![],
            remote_effects: vec![],
            touched_remote,
            undoes: None,
            pending_plan: None,
        }
    }

    #[test]
    fn list_table_args_column_alignment() {
        let records = vec![
            make_record("op_0000000000000_aaaa1111bbbb2222", OperationKind::Checkout, false, vec!["co".into(), "fix-LAUNCHER-v2-11W".into()]),
            make_record("op_0000000000001_cccc3333dddd4444", OperationKind::Undo, false, vec!["undo".into()]),
            make_record("op_0000000000002_eeee5555ffff6666", OperationKind::Checkout, false, vec!["co".into(), "ktfmt".into()]),
        ];
        let output = format_list_human(&records, false);
        let lines: Vec<&str> = output.lines().collect();
        assert!(lines.len() >= 4, "expected header + 3 rows, got {}", lines.len());

        let header_args_col = lines[0].find("ARGS").expect("header must contain ARGS");
        for (i, line) in lines[1..].iter().enumerate() {
            // The ARGS value starts after the UNDOABLE column. Find its position
            // by looking for the content after the 4th double-space-separated column.
            let args_col = find_args_column(line);
            assert_eq!(
                args_col, header_args_col,
                "row {} ARGS column at {} but header at {}: {:?}",
                i, args_col, header_args_col, line
            );
        }
    }

    #[test]
    fn list_table_remote_alignment() {
        let records = vec![
            make_record("op_0000000000000_aaaa1111bbbb2222", OperationKind::Checkout, false, vec!["co".into(), "main".into()]),
            make_record("op_0000000000001_cccc3333dddd4444", OperationKind::Land, true, vec!["land".into()]),
        ];
        let output = format_list_human(&records, false);
        let lines: Vec<&str> = output.lines().collect();
        assert!(lines.len() >= 3);

        let header_args_col = lines[0].find("ARGS").expect("header must contain ARGS");
        for (i, line) in lines[1..].iter().enumerate() {
            let args_col = find_args_column(line);
            assert_eq!(
                args_col, header_args_col,
                "row {} ARGS column at {} but header at {}: {:?}",
                i, args_col, header_args_col, line
            );
        }
    }

    /// Find the byte offset where the ARGS value starts in a data row.
    /// The table has 4 columns before ARGS, each separated by two spaces.
    fn find_args_column(line: &str) -> usize {
        // Skip 4 column groups (ID, KIND, STATUS, UNDOABLE) each followed by "  "
        let mut pos = 0;
        for _ in 0..4 {
            // Skip non-space content
            while pos < line.len() && !line[pos..].starts_with("  ") {
                pos += 1;
            }
            // Skip the double-space separator
            while pos < line.len() && line.as_bytes()[pos] == b' ' {
                pos += 1;
            }
        }
        pos
    }
}
