//! `gg run` - Run a command on each commit in the stack

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use console::style;
use git2::Oid;

use crate::error::{GgError, Result};
use crate::git;
use crate::output::{
    self, RunCommandResult, RunCommitResult, RunResponse, RunResultJson, OUTPUT_VERSION,
};
use crate::stack::{Stack, StackEntry};

/// How to handle working-tree changes after running commands on a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeMode {
    /// Fail with error if the working tree is dirty after running commands.
    /// This is the default — safe for read-only validation (build, test).
    ReadOnly,
    /// Stage all changes and amend them into the commit.
    /// Used by `gg lint` and auto-fixers (formatters, codemods).
    Amend,
    /// Discard all working-tree changes after each commit.
    /// For commands with known side effects you want to ignore.
    Discard,
}

/// One command to execute on a commit.
///
/// Distinguishes pre-parsed argv (from the CLI, where clap has already done
/// word-splitting) from raw shell strings (from config, where users may rely
/// on shell metacharacters like `&&`, `|`, redirects). Keeping them distinct
/// at the type level prevents `gg run git commit -m "hello world"` from being
/// joined-and-resplit on whitespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunCommand {
    /// Pre-parsed argv. Executed directly without any shell involvement.
    Argv(Vec<String>),
    /// Shell command string. Executed via `sh -c` if it contains shell
    /// metacharacters, otherwise split on whitespace and exec'd directly.
    Shell(String),
}

impl RunCommand {
    /// Human-readable display form. For `Shell`, returns the string verbatim.
    /// For `Argv`, shell-escapes each argument so the result is safe to copy
    /// into a terminal. This is the string that ends up in
    /// `RunCommandResult.command` and in non-JSON output.
    pub fn display(&self) -> String {
        match self {
            RunCommand::Shell(s) => s.clone(),
            RunCommand::Argv(v) => display_argv(v),
        }
    }
}

/// Render an argv as a single copy-pasteable shell command string.
/// Arguments containing whitespace or shell-special chars get single-quoted;
/// embedded single quotes are escaped as `'\''`. Plain arguments pass through.
///
/// This is for **display only**. Do not round-trip it back into execution —
/// use the original `RunCommand::Argv` variant for that.
fn display_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| {
            if arg.is_empty() {
                "''".to_string()
            } else if arg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./=:".contains(c))
            {
                arg.clone()
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Options for `run::execute()`.
pub struct RunOptions {
    /// Commands to execute on each commit.
    pub commands: Vec<RunCommand>,
    /// How to handle file modifications.
    pub change_mode: ChangeMode,
    /// Stop at this commit position (1-indexed). None = current position or full stack.
    pub until: Option<usize>,
    /// Stop on first command failure instead of continuing.
    pub stop_on_error: bool,
    /// Output structured JSON instead of text.
    pub json: bool,
    /// Whether to actually emit JSON output (false when called from sync).
    pub emit_json_output: bool,
    /// Optional label for the header (e.g. "lint" instead of "run").
    pub header_label: Option<String>,
    /// Number of parallel jobs. 0 = auto (num CPUs), 1 = sequential.
    /// Parallel only applies to ReadOnly mode.
    pub jobs: usize,
}

/// Raw result from running commands on the stack.
pub struct RunResult {
    pub all_passed: bool,
    pub results: Vec<RunCommitResult>,
}

/// Run commands on each commit in the stack and print output.
///
/// Returns `Ok(true)` when all commands passed on all commits,
/// `Ok(false)` when one or more had failures.
pub fn execute(options: RunOptions) -> Result<bool> {
    let json = options.json;
    let emit_json_output = options.emit_json_output;
    let result = execute_raw(options)?;

    if json && emit_json_output {
        output::print_json(&RunResponse {
            version: OUTPUT_VERSION,
            run: RunResultJson {
                results: result.results,
                all_passed: result.all_passed,
            },
        });
    }

    Ok(result.all_passed)
}

/// Execute and return raw results (for `gg lint` to wrap in LintResponse).
pub fn execute_raw(options: RunOptions) -> Result<RunResult> {
    let repo = git::open_repo()?;

    // Require clean working directory
    git::require_clean_working_directory(&repo)?;

    // Load stack
    let config = crate::config::Config::load_with_global(repo.commondir())?;
    let stack = Stack::load(&repo, &config)?;

    if stack.is_empty() {
        if !options.json {
            println!("{}", style("Stack is empty. Nothing to run.").dim());
        }
        return Ok(RunResult {
            all_passed: true,
            results: vec![],
        });
    }

    // Determine the end position
    let end_pos = options
        .until
        .unwrap_or_else(|| stack.current_position.map(|p| p + 1).unwrap_or(stack.len()));

    if end_pos > stack.len() {
        return Err(GgError::Other(format!(
            "Position {} is out of range (max: {})",
            end_pos,
            stack.len()
        )));
    }

    // Determine whether to use parallel execution
    let use_parallel = options.change_mode == ChangeMode::ReadOnly && options.jobs != 1;

    if use_parallel {
        if !options.json {
            let jobs = effective_jobs(options.jobs);
            let header = format!(
                "Running {} command(s) on commits 1-{} (mode: read-only, jobs: {})",
                options.commands.len(),
                end_pos,
                if options.jobs == 0 {
                    format!("auto={}", jobs)
                } else {
                    jobs.to_string()
                },
            );
            println!("{}", style(header).dim());
        }

        return run_on_commits_parallel(&repo, &stack, &options, end_pos);
    }

    // --- Sequential path ---
    if !options.json {
        let header = if let Some(ref label) = options.header_label {
            format!(
                "Running {} on commits 1-{} ({} {} commands)",
                label,
                end_pos,
                options.commands.len(),
                label,
            )
        } else {
            let mode_label = match options.change_mode {
                ChangeMode::ReadOnly => "read-only",
                ChangeMode::Amend => "amend",
                ChangeMode::Discard => "discard",
            };
            format!(
                "Running {} command(s) on commits 1-{} (mode: {})",
                options.commands.len(),
                end_pos,
                mode_label,
            )
        };
        println!("{}", style(header).dim());
    }

    if options.jobs > 1 && options.change_mode != ChangeMode::ReadOnly && !options.json {
        println!(
            "{}",
            style("Note: --jobs is ignored in amend/discard mode (requires sequential execution)")
                .dim()
        );
    }

    let original_branch = git::current_branch_name(&repo);
    let original_head = repo.head()?.peel_to_commit()?.id();

    let result = run_on_commits(&repo, stack, &options, end_pos);

    if result.is_err() && !git::is_rebase_in_progress(&repo) {
        restore_original_position(
            &repo,
            original_branch.as_deref(),
            original_head,
            options.json,
        );
    }

    result
}

fn run_on_commits(
    repo: &git2::Repository,
    stack: Stack,
    options: &RunOptions,
    end_pos: usize,
) -> Result<RunResult> {
    let original_branch = git::current_branch_name(repo);
    let original_head = repo.head()?.peel_to_commit()?.id();
    let repo_root = repo
        .workdir()
        .ok_or_else(|| GgError::Other("Repository has no working directory".to_string()))?;
    let mut had_changes = false;
    let base_branch = stack.base.clone();
    let stack_branch = stack.branch_name();
    let mut entries = stack.entries.clone();
    let mut run_results: Vec<RunCommitResult> = Vec::with_capacity(end_pos);
    let mut all_passed = true;

    let mut i = 0;
    while i < end_pos {
        let entry = entries[i].clone();
        let mut had_changes_this_commit = false;
        // Captured right after `git commit --amend` succeeds so `final_sha`
        // reports the amended commit's OID rather than whatever HEAD is at
        // after a subsequent rebase-onto (which moves HEAD to the stack tip).
        // See Bug #3.
        let mut amended_oid: Option<Oid> = None;

        if !options.json {
            println!();
            println!(
                "{} Running on [{}] {} {}",
                style("→").cyan(),
                entry.position,
                style(&entry.short_sha).yellow(),
                entry.title
            );
        }

        // Checkout this commit
        let commit = repo.find_commit(entry.oid)?;
        git::checkout_commit(repo, &commit)?;

        // Run commands
        let mut commit_passed = true;
        let mut command_results = Vec::with_capacity(options.commands.len());

        for cmd in &options.commands {
            let cmd_display = cmd.display();
            if !options.json {
                print!("  Running: {} ... ", style(&cmd_display).dim());
            }

            let output = match execute_command(cmd, repo_root, repo) {
                Ok(output) => output,
                Err(e) => {
                    if !options.json {
                        println!("{}", style("ERROR").red().bold());
                    }
                    let error_msg = if e.kind() == std::io::ErrorKind::NotFound {
                        format!(
                            "Command '{}' not found. Make sure it's installed and in your PATH.\n\
                             Note: Shell aliases don't work here. Use the full command (e.g., './gradlew' instead of 'gw').",
                            cmd_display
                        )
                    } else {
                        format!("Failed to run '{}': {}", cmd_display, e)
                    };
                    return Err(GgError::Other(error_msg));
                }
            };

            let passed = output.status.success();
            if passed {
                if !options.json {
                    println!("{}", style("OK").green());
                }
            } else {
                commit_passed = false;
                all_passed = false;
                if !options.json {
                    println!("{}", style("FAILED").red());
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !stderr.is_empty() {
                        for line in stderr.lines().take(5) {
                            println!("    {}", style(line).dim());
                        }
                    }
                }
            }

            let combined_output = if passed {
                None
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut text = String::new();
                if !stderr.trim().is_empty() {
                    text.push_str(stderr.trim_end());
                }
                if !stdout.trim().is_empty() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(stdout.trim_end());
                }
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            };

            command_results.push(RunCommandResult {
                command: cmd_display.clone(),
                passed,
                output: combined_output,
            });
        }

        // Handle file changes based on mode
        if !git::is_working_directory_clean(repo)? {
            match options.change_mode {
                ChangeMode::ReadOnly => {
                    return Err(GgError::Other(format!(
                        "Command modified files on commit [{}] {}.\n\
                         Use --amend to fold changes into each commit, or --discard to ignore them.",
                        entry.position, entry.short_sha
                    )));
                }
                ChangeMode::Amend => {
                    if !options.json {
                        println!(
                            "  {} Command made changes, squashing...",
                            style("!").yellow()
                        );
                    }

                    // Stage all changes
                    let add_output = Command::new("git")
                        .args(["add", "-A"])
                        .current_dir(repo_root)
                        .output()?;

                    if !add_output.status.success() {
                        return Err(GgError::Other(format!(
                            "Failed to stage changes: {}",
                            String::from_utf8_lossy(&add_output.stderr).trim()
                        )));
                    }

                    // Amend the commit
                    let amend_output = Command::new("git")
                        .args(["commit", "--amend", "--no-edit"])
                        .current_dir(repo_root)
                        .stdin(Stdio::null())
                        .output()?;

                    if !amend_output.status.success() {
                        return Err(GgError::Other(format!(
                            "Failed to amend commit: {}",
                            String::from_utf8_lossy(&amend_output.stderr).trim()
                        )));
                    }

                    // Capture the new OID BEFORE any rebase-onto can move HEAD.
                    // This is the authoritative final_sha for the current commit
                    // (Bug #3 fix).
                    let new_head = repo.head()?.peel_to_commit()?.id();
                    amended_oid = Some(new_head);

                    had_changes = true;
                    had_changes_this_commit = true;
                    if !options.json {
                        println!("  {} Changes squashed", style("OK").green());
                    }

                    // Rebase remaining commits onto the amended commit
                    if i + 1 < end_pos {
                        let new_commit_oid = new_head;
                        let old_tip_oid = entries[end_pos - 1].oid;

                        let new_commit = new_commit_oid.to_string();
                        let old_commit = entry.oid.to_string();
                        let old_tip = old_tip_oid.to_string();

                        let target_branch =
                            original_branch.as_deref().unwrap_or(stack_branch.as_str());

                        git::run_git_command(&["branch", "-f", target_branch, &old_tip])?;
                        git::checkout_branch(repo, target_branch)?;

                        if let Err(e) = git::run_git_command(&[
                            "rebase",
                            "--onto",
                            &new_commit,
                            &old_commit,
                            target_branch,
                        ]) {
                            if git::is_rebase_in_progress(repo) {
                                print_rebase_conflict_help(repo_root, options.json);
                                return Err(GgError::Other(
                                    "Rebase conflict occurred. Resolve conflicts and run `gg continue`."
                                        .to_string(),
                                ));
                            }
                            return Err(e);
                        }

                        entries = refresh_stack_entries(repo, &base_branch, None)?;
                    }
                }
                ChangeMode::Discard => {
                    if !options.json {
                        println!("  {} Discarding changes...", style("!").yellow());
                    }

                    let checkout_output = Command::new("git")
                        .args(["checkout", "."])
                        .current_dir(repo_root)
                        .output()?;

                    if !checkout_output.status.success() {
                        return Err(GgError::Other(format!(
                            "Failed to discard changes: {}",
                            String::from_utf8_lossy(&checkout_output.stderr).trim()
                        )));
                    }

                    // Also clean untracked files created by the command
                    let clean_output = Command::new("git")
                        .args(["clean", "-fd"])
                        .current_dir(repo_root)
                        .output()?;

                    if !clean_output.status.success() {
                        return Err(GgError::Other(format!(
                            "Failed to clean untracked files: {}",
                            String::from_utf8_lossy(&clean_output.stderr).trim()
                        )));
                    }

                    if !options.json {
                        println!("  {} Changes discarded", style("OK").green());
                    }
                }
            }
        }

        let final_sha = if had_changes_this_commit {
            // Use the OID captured right after `git commit --amend` rather
            // than reading HEAD — after the subsequent rebase-onto, HEAD
            // points at the rebased tip, not the amended commit. See Bug #3.
            if let Some(oid) = amended_oid {
                let commit = repo.find_commit(oid)?;
                git::short_sha(&commit)
            } else {
                let head = repo.head()?.peel_to_commit()?;
                git::short_sha(&head)
            }
        } else {
            entry.short_sha.clone()
        };

        run_results.push(RunCommitResult {
            position: entry.position,
            sha: final_sha,
            title: entry.title.clone(),
            passed: commit_passed,
            commands: command_results,
        });

        // Stop on first failure if requested
        if !commit_passed && options.stop_on_error {
            if !options.json {
                println!();
                println!(
                    "{} Stopping at commit [{}] due to failure",
                    style("!").yellow(),
                    entry.position,
                );
            }
            break;
        }

        i += 1;
    }

    // Return to original position
    if !options.json {
        println!();
    }
    if let Some(branch) = original_branch {
        if had_changes {
            git::move_branch_to_head(repo, &branch)?;
            git::checkout_branch(repo, &branch)?;

            if !options.json {
                if end_pos < stack.len() {
                    println!(
                        "{}",
                        style("Changes were made. Run `gg rebase` to update remaining commits, then `gg sync`.")
                            .dim()
                    );
                } else {
                    println!(
                        "{}",
                        style("Changes were made. Review with `gg ls` and sync with `gg sync`.")
                            .dim()
                    );
                }
            }
        } else {
            git::checkout_branch(repo, &branch)?;
        }
    } else if !had_changes {
        let commit = repo.find_commit(original_head)?;
        git::checkout_commit(repo, &commit)?;
    }

    if !options.json {
        let status_msg = if all_passed {
            format!(
                "{} Ran on {} commit(s) — all passed",
                style("OK").green().bold(),
                run_results.len()
            )
        } else {
            format!(
                "{} Ran on {} commit(s) — some failed",
                style("FAIL").red().bold(),
                run_results.len()
            )
        };
        println!("{}", status_msg);
    }

    Ok(RunResult {
        all_passed,
        results: run_results,
    })
}

// ---------------------------------------------------------------------------
// Parallel execution
// ---------------------------------------------------------------------------

/// RAII guard that ensures temporary worktrees are cleaned up on all exit paths.
struct WorktreeGuard {
    repo_root: PathBuf,
    base_dir: PathBuf,
    paths: Vec<PathBuf>,
}

impl WorktreeGuard {
    fn new(repo_root: &Path) -> Result<Self> {
        let base_dir = std::env::temp_dir().join(format!("gg-run-{}", std::process::id()));
        std::fs::create_dir_all(&base_dir).map_err(|e| {
            GgError::Other(format!(
                "Failed to create temp directory for worktrees: {}",
                e
            ))
        })?;
        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            base_dir,
            paths: Vec::new(),
        })
    }

    /// Create a detached worktree for the given commit OID.
    fn add(&mut self, index: usize, oid: Oid) -> Result<PathBuf> {
        let wt_path = self.base_dir.join(format!("commit-{}", index));
        let sha = oid.to_string();
        let wt_str = wt_path.to_string_lossy().to_string();

        let output = Command::new("git")
            .args(["worktree", "add", "--detach", &wt_str, &sha])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| GgError::Other(format!("Failed to run git worktree add: {}", e)))?;

        if !output.status.success() {
            return Err(GgError::Other(format!(
                "git worktree add failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        self.paths.push(wt_path.clone());
        Ok(wt_path)
    }
}

impl Drop for WorktreeGuard {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = Command::new("git")
                .args(["worktree", "remove", "--force", &path.to_string_lossy()])
                .current_dir(&self.repo_root)
                .output();
        }
        let _ = std::fs::remove_dir_all(&self.base_dir);
    }
}

/// Pre-resolve commands that reference `.git/` paths to the repo's commondir.
/// Both `Argv(v)` and `Shell(s)` variants get their first program token
/// rewritten if it starts with `.git/` or `./.git/`.
fn pre_resolve_commands(commands: &[RunCommand], repo: &git2::Repository) -> Vec<RunCommand> {
    commands
        .iter()
        .map(|cmd| match cmd {
            RunCommand::Argv(v) => {
                if v.is_empty() {
                    return cmd.clone();
                }
                match resolve_git_path(&v[0], repo) {
                    Some(resolved) => {
                        let mut new_v = Vec::with_capacity(v.len());
                        new_v.push(resolved.to_string_lossy().into_owned());
                        new_v.extend(v.iter().skip(1).cloned());
                        RunCommand::Argv(new_v)
                    }
                    None => cmd.clone(),
                }
            }
            RunCommand::Shell(s) => {
                let parts: Vec<&str> = s.split_whitespace().collect();
                if parts.is_empty() {
                    return cmd.clone();
                }
                match resolve_git_path(parts[0], repo) {
                    Some(resolved) => {
                        let mut new_s = resolved.to_string_lossy().into_owned();
                        for part in &parts[1..] {
                            new_s.push(' ');
                            new_s.push_str(part);
                        }
                        RunCommand::Shell(new_s)
                    }
                    None => cmd.clone(),
                }
            }
        })
        .collect()
}

/// Execute a command in a specific directory, without repo-aware path resolution.
/// Used by parallel execution where commands run in isolated worktrees and any
/// `.git/...` path has already been resolved by `pre_resolve_commands`.
fn execute_command_in_dir(cmd: &RunCommand, dir: &Path) -> std::io::Result<Output> {
    match cmd {
        RunCommand::Shell(s) => {
            if s.contains("&&")
                || s.contains("||")
                || s.contains('|')
                || s.contains('>')
                || s.contains('<')
                || s.contains(';')
            {
                Command::new("sh").args(["-c", s]).current_dir(dir).output()
            } else {
                let parts: Vec<&str> = s.split_whitespace().collect();
                if parts.is_empty() {
                    return Command::new("true").output();
                }
                Command::new(parts[0])
                    .args(&parts[1..])
                    .current_dir(dir)
                    .output()
            }
        }
        RunCommand::Argv(v) => {
            if v.is_empty() {
                return Command::new("true").output();
            }
            Command::new(&v[0]).args(&v[1..]).current_dir(dir).output()
        }
    }
}

/// Run all commands on a single commit in its isolated worktree directory.
///
/// `commands` is the pre-resolved form used for execution (e.g. `.git/gg/lint.sh`
/// rewritten to an absolute path so it resolves inside the detached worktree).
/// `original_commands` is the user's input as typed on the CLI / from config,
/// and is the string that gets reported in the JSON output so that `--jobs 1`
/// and `--jobs N` produce byte-for-byte identical `command` fields.
///
/// When `read_only` is true, we enforce the read-only contract after running:
/// if the worktree is dirty, the commit is marked failed with an error message
/// matching the sequential path's behavior.
fn run_commands_in_worktree(
    commands: &[RunCommand],
    original_commands: &[RunCommand],
    wt_path: &Path,
    entry: &StackEntry,
    read_only: bool,
) -> RunCommitResult {
    let mut commit_passed = true;
    let mut command_results = Vec::with_capacity(commands.len());

    for (cmd, orig) in commands.iter().zip(original_commands.iter()) {
        let output = execute_command_in_dir(cmd, wt_path);

        let passed = output.as_ref().map(|o| o.status.success()).unwrap_or(false);
        if !passed {
            commit_passed = false;
        }

        let combined_output = if passed {
            None
        } else {
            match output {
                Ok(ref o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    let mut text = String::new();
                    if !stderr.trim().is_empty() {
                        text.push_str(stderr.trim_end());
                    }
                    if !stdout.trim().is_empty() {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(stdout.trim_end());
                    }
                    if text.is_empty() {
                        None
                    } else {
                        Some(text)
                    }
                }
                Err(ref e) => Some(format!("Failed to execute: {}", e)),
            }
        };

        command_results.push(RunCommandResult {
            command: orig.display(),
            passed,
            output: combined_output,
        });
    }

    // Enforce read-only contract in parallel mode: if any command passed exit-code
    // but dirtied the worktree, mark this commit as failed so we match the
    // sequential path's behavior (which errors out on dirty trees in ReadOnly mode).
    if read_only && commit_passed && is_worktree_dirty(wt_path) {
        commit_passed = false;
        let msg = format!(
            "Command modified files on commit [{}] {}. Use --amend to fold changes, or --discard to ignore them.",
            entry.position, entry.short_sha
        );
        if let Some(last) = command_results.last_mut() {
            last.passed = false;
            last.output = Some(match last.output.take() {
                Some(existing) => format!("{existing}\n{msg}"),
                None => msg,
            });
        }
    }

    RunCommitResult {
        position: entry.position,
        sha: entry.short_sha.clone(),
        title: entry.title.clone(),
        passed: commit_passed,
        commands: command_results,
    }
}

/// Check whether a worktree has any uncommitted/untracked changes via
/// `git status --porcelain`. Used to enforce the read-only contract in
/// parallel mode after commands finish running.
fn is_worktree_dirty(wt_path: &Path) -> bool {
    match Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(wt_path)
        .output()
    {
        Ok(out) if out.status.success() => !out.stdout.is_empty(),
        // If the status check itself failed, err on the safe side and treat as dirty
        // so the caller surfaces a failure rather than falsely reporting success.
        _ => true,
    }
}

/// Parallel execution path: creates isolated worktrees, runs commands concurrently,
/// collects results in commit order. Only valid for ReadOnly mode.
fn run_on_commits_parallel(
    repo: &git2::Repository,
    stack: &Stack,
    options: &RunOptions,
    end_pos: usize,
) -> Result<RunResult> {
    let repo_root = repo
        .workdir()
        .ok_or_else(|| GgError::Other("Repository has no working directory".to_string()))?;
    let jobs = effective_jobs(options.jobs);
    let entries = &stack.entries[..end_pos];

    let resolved_commands = pre_resolve_commands(&options.commands, repo);
    let original_commands: &[RunCommand] = &options.commands;
    // Parallelism is only valid for ReadOnly; we enforce the read-only contract
    // (post-command dirty check) inside each worker just like the sequential path.
    let is_read_only = options.change_mode == ChangeMode::ReadOnly;

    // Create worktrees (sequential — git requires this)
    let mut guard = WorktreeGuard::new(repo_root)?;
    let mut worktree_paths: Vec<PathBuf> = Vec::with_capacity(end_pos);

    if !options.json {
        println!(
            "{}",
            style(format!(
                "Creating {} worktree(s) for parallel execution...",
                end_pos
            ))
            .dim()
        );
    }

    for (i, entry) in entries.iter().enumerate() {
        let wt_path = guard.add(i, entry.oid)?;
        worktree_paths.push(wt_path);
    }

    // Progress bar for non-JSON output
    let pb = if !options.json {
        let pb = indicatif::ProgressBar::new(end_pos as u64);
        pb.set_style(
            indicatif::ProgressStyle::with_template(
                "  {spinner:.cyan} [{bar:30.cyan/dim}] {pos}/{len} commits ({elapsed})",
            )
            .unwrap()
            .progress_chars("━╸─"),
        );
        Some(pb)
    } else {
        None
    };

    // Build work items: (index, entry, worktree_path, original_commands)
    // original_commands is threaded through so the resulting RunCommandResult.command
    // reflects the user's input (e.g. `.git/gg/lint.sh`), not the pre-resolved
    // absolute path used for execution inside the detached worktree.
    let work_items: Vec<(usize, &StackEntry, &Path, &[RunCommand])> = entries
        .iter()
        .zip(worktree_paths.iter())
        .enumerate()
        .map(|(i, (entry, path))| (i, entry, path.as_path(), original_commands))
        .collect();

    // Run in parallel with bounded concurrency
    let work = std::sync::Mutex::new(work_items.into_iter());
    let collected: std::sync::Mutex<Vec<(usize, RunCommitResult)>> =
        std::sync::Mutex::new(Vec::with_capacity(end_pos));

    std::thread::scope(|s| {
        let num_workers = jobs.min(end_pos);
        for _ in 0..num_workers {
            s.spawn(|| {
                loop {
                    // Use into_inner on PoisonError so a sibling panic doesn't
                    // cascade into a double-panic abort (which would leak worktrees
                    // by skipping WorktreeGuard::drop).
                    let item = {
                        let mut guard = work
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        guard.next()
                    };
                    match item {
                        Some((idx, entry, wt_path, orig_cmds)) => {
                            let result = run_commands_in_worktree(
                                &resolved_commands,
                                orig_cmds,
                                wt_path,
                                entry,
                                is_read_only,
                            );
                            collected
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .push((idx, result));
                            if let Some(ref pb) = pb {
                                pb.inc(1);
                            }
                        }
                        None => break,
                    }
                }
            });
        }
        // Threads join here when scope exits
    });

    let results = collected
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if let Some(ref pb) = pb {
        pb.finish_and_clear();
    }

    // Sort results by commit position
    let mut sorted_results: Vec<RunCommitResult> = {
        let mut indexed = results;
        indexed.sort_by_key(|(idx, _)| *idx);
        indexed.into_iter().map(|(_, r)| r).collect()
    };

    let all_passed = sorted_results.iter().all(|r| r.passed);

    // Apply stop_on_error: truncate after first failure
    if options.stop_on_error && !all_passed {
        if let Some(first_fail) = sorted_results.iter().position(|r| !r.passed) {
            sorted_results.truncate(first_fail + 1);
        }
    }

    // Print results in commit order (non-JSON)
    if !options.json {
        for result in &sorted_results {
            println!();
            let status_icon = if result.passed {
                style("✓").green()
            } else {
                style("✗").red()
            };
            println!(
                "{} [{}] {} {}",
                status_icon,
                result.position,
                style(&result.sha).yellow(),
                result.title,
            );
            for cmd_result in &result.commands {
                let cmd_status = if cmd_result.passed {
                    style("OK").green().to_string()
                } else {
                    style("FAILED").red().to_string()
                };
                println!(
                    "  {} {} {}",
                    style("→").dim(),
                    cmd_result.command,
                    cmd_status
                );
                if let Some(ref output) = cmd_result.output {
                    for line in output.lines().take(5) {
                        println!("    {}", style(line).dim());
                    }
                }
            }
        }
        println!();
        let status_msg = if all_passed {
            format!(
                "{} Ran on {} commit(s) across {} worker(s) — all passed",
                style("OK").green().bold(),
                sorted_results.len(),
                jobs.min(end_pos),
            )
        } else {
            format!(
                "{} Ran on {} commit(s) across {} worker(s) — some failed",
                style("FAIL").red().bold(),
                sorted_results.len(),
                jobs.min(end_pos),
            )
        };
        println!("{}", status_msg);
    }

    // guard is dropped here — worktrees cleaned up automatically

    Ok(RunResult {
        all_passed,
        results: sorted_results,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve the effective number of parallel jobs.
/// 0 = auto (available parallelism), 1+ = explicit count.
fn effective_jobs(jobs: usize) -> usize {
    if jobs == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    } else {
        jobs
    }
}

/// Execute a command on the given repo root.
///
/// - `RunCommand::Shell(s)`: uses `sh -c` when `s` contains shell metacharacters,
///   otherwise splits on whitespace and execs directly. `.git/...` paths get
///   rewritten to the real commondir for linked-worktree support.
/// - `RunCommand::Argv(v)`: execs `v[0]` directly with `v[1..]` as arguments,
///   preserving whitespace and quoting inside each element. `.git/...` on
///   `v[0]` is rewritten the same way.
fn execute_command(
    cmd: &RunCommand,
    repo_root: &Path,
    repo: &git2::Repository,
) -> std::io::Result<Output> {
    match cmd {
        RunCommand::Shell(s) => {
            if s.contains("&&")
                || s.contains("||")
                || s.contains('|')
                || s.contains('>')
                || s.contains('<')
                || s.contains(';')
            {
                Command::new("sh")
                    .args(["-c", s])
                    .current_dir(repo_root)
                    .output()
            } else {
                let parts: Vec<&str> = s.split_whitespace().collect();
                if parts.is_empty() {
                    return Command::new("true").output();
                }

                let resolved_cmd = resolve_git_path(parts[0], repo);
                let cmd_str = resolved_cmd
                    .as_ref()
                    .map(|p| p.to_string_lossy())
                    .unwrap_or_else(|| parts[0].into());

                Command::new(cmd_str.as_ref())
                    .args(&parts[1..])
                    .current_dir(repo_root)
                    .output()
            }
        }
        RunCommand::Argv(v) => {
            if v.is_empty() {
                return Command::new("true").output();
            }
            let resolved = resolve_git_path(&v[0], repo);
            let program = resolved
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| v[0].clone());
            Command::new(program)
                .args(&v[1..])
                .current_dir(repo_root)
                .output()
        }
    }
}

fn refresh_stack_entries(
    repo: &git2::Repository,
    base_branch: &str,
    stack_branch: Option<&str>,
) -> Result<Vec<StackEntry>> {
    let oids = git::get_stack_commit_oids(repo, base_branch, stack_branch)?;

    let mut entries = Vec::with_capacity(oids.len());
    for (i, oid) in oids.iter().enumerate() {
        let commit = repo.find_commit(*oid)?;
        entries.push(StackEntry::from_commit(&commit, i + 1));
    }

    Ok(entries)
}

/// Restore the original branch/HEAD position.
fn restore_original_position(
    repo: &git2::Repository,
    original_branch: Option<&str>,
    original_head: Oid,
    json: bool,
) {
    if !json {
        println!();
        println!("{} Restoring original position...", style("→").cyan());
    }

    let restored = if let Some(branch) = original_branch {
        git::checkout_branch(repo, branch).is_ok()
    } else if let Ok(commit) = repo.find_commit(original_head) {
        git::checkout_commit(repo, &commit).is_ok()
    } else {
        false
    };

    if !json {
        if restored {
            println!("{} Restored to original position", style("OK").green());
        } else {
            println!(
                "{} Could not restore original position. You may be in detached HEAD.",
                style("Warning:").yellow()
            );
        }
    }
}

/// Resolve a command path that starts with `.git/` or `./.git/` to the real
/// git common directory (for linked worktree support).
pub(crate) fn resolve_git_path(cmd: &str, repo: &git2::Repository) -> Option<PathBuf> {
    let remainder = if let Some(rest) = cmd.strip_prefix("./.git/") {
        rest
    } else if let Some(rest) = cmd.strip_prefix(".git/") {
        rest
    } else {
        return None;
    };

    Some(repo.commondir().join(remainder))
}

/// Get list of files with conflicts.
fn get_conflicted_files(repo_root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["diff", "--name-only", "--diff-filter=U"])
        .current_dir(repo_root)
        .output();

    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Print helpful message when rebase conflict occurs.
fn print_rebase_conflict_help(repo_root: &Path, json: bool) {
    if json {
        return;
    }

    println!();
    println!(
        "{} Rebase conflict while rebasing after changes",
        style("⚠️").yellow()
    );
    println!();

    let conflicted_files = get_conflicted_files(repo_root);
    if !conflicted_files.is_empty() {
        println!("The following files have conflicts:");
        for file in &conflicted_files {
            println!("  {} {}", style("-").dim(), file);
        }
        println!();
    }

    println!("To resolve:");
    println!("  1. Edit the conflicting files to resolve conflicts");
    println!("  2. {}", style("git add <resolved-files>").cyan());
    println!("  3. {}", style("gg continue").cyan());
    println!();
    println!("To abort and undo changes:");
    println!("  {}", style("gg abort").cyan());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo() -> (tempfile::TempDir, git2::Repository) {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        (dir, repo)
    }

    #[test]
    fn test_resolve_git_path_with_dot_slash_prefix() {
        let (_dir, repo) = temp_repo();
        let result = resolve_git_path("./.git/gg/lint.sh", &repo);
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert!(resolved.ends_with("gg/lint.sh"));
        assert!(resolved.starts_with(repo.commondir()));
    }

    #[test]
    fn test_resolve_git_path_without_dot_slash_prefix() {
        let (_dir, repo) = temp_repo();
        let result = resolve_git_path(".git/gg/lint.sh", &repo);
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("gg/lint.sh"));
    }

    #[test]
    fn test_resolve_git_path_non_git_path_returns_none() {
        let (_dir, repo) = temp_repo();
        assert!(resolve_git_path("cargo", &repo).is_none());
        assert!(resolve_git_path("./scripts/lint.sh", &repo).is_none());
    }

    #[test]
    fn test_effective_jobs() {
        assert_eq!(effective_jobs(1), 1);
        assert_eq!(effective_jobs(4), 4);
        // jobs=0 means auto — should return at least 1
        assert!(effective_jobs(0) >= 1);
    }

    #[test]
    fn test_execute_command_in_dir_simple_shell() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = RunCommand::Shell("echo hello".to_string());
        let output = execute_command_in_dir(&cmd, dir.path()).unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }

    #[test]
    fn test_execute_command_in_dir_shell_metacharacters() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = RunCommand::Shell("echo a && echo b".to_string());
        let output = execute_command_in_dir(&cmd, dir.path()).unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("a"));
        assert!(stdout.contains("b"));
    }

    #[cfg(unix)]
    #[test]
    fn test_execute_command_in_dir_argv_preserves_quoted_args() {
        // Regression for Bug #1: argv variant must not split on whitespace.
        // `test "a b" = "a b"` exits 0 when args are preserved, 2 (usage
        // error) otherwise. We pick whichever of `test` lives on this OS.
        let test_bin = if std::path::Path::new("/usr/bin/test").exists() {
            "/usr/bin/test"
        } else {
            "/bin/test"
        };
        let dir = tempfile::tempdir().unwrap();
        let cmd = RunCommand::Argv(vec![
            test_bin.to_string(),
            "a b".to_string(),
            "=".to_string(),
            "a b".to_string(),
        ]);
        let output = execute_command_in_dir(&cmd, dir.path()).unwrap();
        assert!(
            output.status.success(),
            "{} should treat 'a b' as one argument",
            test_bin
        );
    }

    #[test]
    fn test_display_argv_single_quotes_whitespace() {
        let argv = vec!["echo".to_string(), "hello world".to_string()];
        assert_eq!(display_argv(&argv), "echo 'hello world'");
    }

    #[test]
    fn test_display_argv_passes_plain_args_through() {
        let argv = vec![
            "git".to_string(),
            "commit".to_string(),
            "--no-edit".to_string(),
        ];
        assert_eq!(display_argv(&argv), "git commit --no-edit");
    }

    #[test]
    fn test_display_argv_escapes_embedded_single_quote() {
        let argv = vec!["echo".to_string(), "it's".to_string()];
        assert_eq!(display_argv(&argv), r#"echo 'it'\''s'"#);
    }
}
