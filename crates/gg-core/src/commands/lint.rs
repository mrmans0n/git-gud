//! `gg lint` - Run configured lint commands on each commit in the stack
//!
//! Thin wrapper around `gg run` that reads commands from config
//! and uses `ChangeMode::Amend`.

use console::style;

use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::output::{
    self, LintCommandResult, LintCommitResult, LintResponse, LintResultJson, OUTPUT_VERSION,
};

use super::run::{self, ChangeMode, RunOptions};

/// Run the lint command.
///
/// Returns `Ok(true)` when all lint commands passed for all linted commits,
/// `Ok(false)` when one or more commits had lint failures.
pub fn run(until: Option<usize>, json: bool, emit_json_output: bool) -> Result<bool> {
    run_inner(until, json, emit_json_output, false)
}

/// Same as [`run`], but intended for callers that already hold the operation
/// lock (e.g. `gg sync --run-lint`). Skips re-acquiring the advisory lock,
/// avoiding a self-deadlock inside `run::execute_raw`.
pub(crate) fn run_without_lock(
    until: Option<usize>,
    json: bool,
    emit_json_output: bool,
) -> Result<bool> {
    run_inner(until, json, emit_json_output, true)
}

fn run_inner(
    until: Option<usize>,
    json: bool,
    emit_json_output: bool,
    skip_lock: bool,
) -> Result<bool> {
    let repo = git::open_repo()?;
    let config = Config::load_with_global(repo.commondir())?;

    let lint_commands = &config.defaults.lint;
    if lint_commands.is_empty() {
        if json && emit_json_output {
            print_empty_response();
        } else if !json {
            println!(
                "{}",
                style("No lint commands configured. Run 'gg setup' to configure lint commands.")
                    .dim()
            );
            println!();
            println!("Example configuration:");
            println!("  {{");
            println!("    \"defaults\": {{");
            println!("      \"lint\": [\"cargo fmt\", \"cargo clippy -- -D warnings\"]");
            println!("    }}");
            println!("  }}");
        }
        return Ok(true);
    }

    let run_options = RunOptions {
        commands: lint_commands
            .iter()
            .map(|s| run::RunCommand::Shell(s.clone()))
            .collect(),
        change_mode: ChangeMode::Amend,
        until,
        stop_on_error: false,
        json,
        emit_json_output,
        header_label: Some("lint".to_string()),
        jobs: 1,
    };

    let result = if skip_lock {
        run::execute_raw_without_lock(run_options)?
    } else {
        run::execute_raw(run_options)?
    };

    if json && emit_json_output {
        // Emit LintResponse (key: "lint") for backward compatibility
        let lint_results: Vec<LintCommitResult> = result
            .results
            .into_iter()
            .map(|r| LintCommitResult {
                position: r.position,
                sha: r.sha,
                title: r.title,
                passed: r.passed,
                commands: r
                    .commands
                    .into_iter()
                    .map(|c| LintCommandResult {
                        command: c.command,
                        passed: c.passed,
                        output: c.output,
                    })
                    .collect(),
            })
            .collect();

        output::print_json(&LintResponse {
            version: OUTPUT_VERSION,
            lint: LintResultJson {
                results: lint_results,
                all_passed: result.all_passed,
            },
        });
    }

    Ok(result.all_passed)
}

fn print_empty_response() {
    output::print_json(&LintResponse {
        version: OUTPUT_VERSION,
        lint: LintResultJson {
            results: vec![],
            all_passed: true,
        },
    });
}
