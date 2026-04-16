//! `gg log` - Smartlog-style view of the current stack
//!
//! Renders the current stack as a tree view (bottom-to-top), with PR/MR
//! status, CI badges, and a HEAD marker. Stack-scoped — to see all stacks,
//! use `gg ls --all` instead.

use console::style;

use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::output::{print_json, LogJson, LogResponse, StackEntryJson, OUTPUT_VERSION};
use crate::provider::{CiStatus, PrState, Provider};
use crate::stack::Stack;

/// Run the log command
pub fn run(json: bool, refresh: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.commondir();
    let config = Config::load_with_global(git_dir)?;

    let mut stack = Stack::load(&repo, &config)?;

    if should_refresh_mr_info(refresh, json) {
        if refresh {
            let provider = Provider::detect(&repo)?;
            if !json {
                print!("Refreshing {} status... ", provider.pr_label());
            }
            stack.refresh_mr_info(&provider)?;
            if !json {
                println!("{}", style("done").green());
            }
        } else if let Ok(provider) = Provider::detect(&repo) {
            stack.refresh_mr_info(&provider)?;
        }
    }

    if json {
        render_json(&stack);
    } else {
        render_text(&stack, &repo);
    }

    Ok(())
}

fn render_json(stack: &Stack) {
    let current_pos_1based = stack.current_position.map(|p| p + 1);

    let entries: Vec<StackEntryJson> = stack
        .entries
        .iter()
        .map(|entry| {
            let is_current = match current_pos_1based {
                Some(p) => entry.position == p,
                None => entry.position == stack.len(),
            };

            StackEntryJson {
                position: entry.position,
                sha: entry.short_sha.clone(),
                title: entry.title.clone(),
                gg_id: entry.gg_id.clone(),
                gg_parent: entry.gg_parent.clone(),
                pr_number: entry.mr_number,
                pr_state: entry.mr_state.as_ref().map(pr_state_to_json),
                approved: entry.approved,
                ci_status: entry.ci_status.as_ref().map(ci_status_to_json),
                is_current,
                in_merge_train: entry.in_merge_train,
                merge_train_position: entry.merge_train_position,
            }
        })
        .collect();

    print_json(&LogResponse {
        version: OUTPUT_VERSION,
        log: LogJson {
            stack: stack.name.clone(),
            base: stack.base.clone(),
            current_position: current_pos_1based,
            entries,
        },
    });
}

fn render_text(stack: &Stack, repo: &git2::Repository) {
    println!(
        "{} ({} commits, base: {})",
        style(&stack.name).cyan().bold(),
        stack.len(),
        style(&stack.base).dim(),
    );
    println!();

    if git::is_rebase_in_progress(repo) {
        println!(
            "{} {}",
            style("⚠️").yellow(),
            style("Rebase in progress. Run `gg continue` or `gg abort`")
                .yellow()
                .bold()
        );
        println!();
    }

    if stack.is_empty() {
        println!(
            "{}",
            style("  (empty stack — use `git commit` to add changes)").dim()
        );
        return;
    }

    let provider = Provider::detect(repo).ok();
    let pr_prefix = provider
        .as_ref()
        .map(|p| p.pr_number_prefix())
        .unwrap_or("!");

    let current_pos_0based = stack
        .current_position
        .unwrap_or(stack.len().saturating_sub(1));
    let total = stack.len();

    for (i, entry) in stack.entries.iter().enumerate() {
        let is_current = i == current_pos_0based;
        let glyph = glyph_for_position(i, total);
        let line = format_entry_line(entry, is_current, pr_prefix);
        println!("  {} {}", style(glyph).dim(), line);

        if let Some(mr_num) = entry.mr_number {
            let mut mr_line = format!("{}{}", pr_prefix, mr_num);

            if entry.in_merge_train {
                if let Some(pos) = entry.merge_train_position {
                    mr_line.push_str(&format!(" [train pos {}]", pos));
                } else {
                    mr_line.push_str(" [train]");
                }
            }

            let continuation = if i + 1 < total { "│" } else { " " };
            println!(
                "  {}     {}",
                style(continuation).dim(),
                style(&mr_line).blue()
            );
        }
    }

    println!();
}

/// Pick the tree glyph for an entry at index `i` out of `total`.
///
/// `├──` for all entries except the last (HEAD), which uses `└──`.
fn glyph_for_position(i: usize, total: usize) -> &'static str {
    if i + 1 == total {
        "└──"
    } else {
        "├──"
    }
}

/// Format a single entry's display line (without the leading tree glyph).
fn format_entry_line(
    entry: &crate::stack::StackEntry,
    is_current: bool,
    pr_prefix: &str,
) -> String {
    let position = format!("[{}]", entry.position);
    let sha = &entry.short_sha;
    let title = &entry.title;

    let status = entry.status_display();
    let status_styled = match &entry.mr_state {
        Some(PrState::Merged) => style(&status).green(),
        Some(PrState::Closed) => style(&status).red(),
        Some(PrState::Draft) => style(&status).dim(),
        Some(PrState::Open) if entry.approved => style(&status).green(),
        Some(PrState::Open) => style(&status).yellow(),
        None => style(&status).dim(),
    };

    let ci = match &entry.ci_status {
        Some(CiStatus::Success) => style("✓").green().to_string(),
        Some(CiStatus::Failed) => style("✗").red().to_string(),
        Some(CiStatus::Running) => style("●").yellow().to_string(),
        Some(CiStatus::Pending) => style("○").dim().to_string(),
        _ => String::new(),
    };

    let train = if entry.in_merge_train { " 🚂" } else { "" };
    let mr_display = entry
        .mr_number
        .map(|n| format!(" {}{}", pr_prefix, n))
        .unwrap_or_default();
    let head_marker = if is_current { " <- HEAD" } else { "" };

    if is_current {
        format!(
            "{} {} {} {} {}{}{}{}",
            style(&position).bold(),
            style(sha).yellow().bold(),
            style(title).bold(),
            status_styled,
            ci,
            train,
            style(&mr_display).blue(),
            style(head_marker).cyan().bold(),
        )
    } else {
        format!(
            "{} {} {} {} {}{}{}",
            style(&position).dim(),
            style(sha).yellow(),
            title,
            status_styled,
            ci,
            train,
            style(&mr_display).blue(),
        )
    }
}

fn pr_state_to_json(state: &PrState) -> String {
    match state {
        PrState::Open => "open".to_string(),
        PrState::Merged => "merged".to_string(),
        PrState::Closed => "closed".to_string(),
        PrState::Draft => "draft".to_string(),
    }
}

fn ci_status_to_json(status: &CiStatus) -> String {
    match status {
        CiStatus::Pending => "pending".to_string(),
        CiStatus::Running => "running".to_string(),
        CiStatus::Success => "success".to_string(),
        CiStatus::Failed => "failed".to_string(),
        CiStatus::Canceled => "canceled".to_string(),
        CiStatus::Unknown => "unknown".to_string(),
    }
}

fn should_refresh_mr_info(refresh: bool, json: bool) -> bool {
    refresh || json
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_middle_entries_use_tee() {
        assert_eq!(glyph_for_position(0, 3), "├──");
        assert_eq!(glyph_for_position(1, 3), "├──");
    }

    #[test]
    fn glyph_last_entry_uses_corner() {
        assert_eq!(glyph_for_position(2, 3), "└──");
    }

    #[test]
    fn glyph_single_entry_is_corner() {
        // Single-entry stack: the lone entry is both first and last.
        assert_eq!(glyph_for_position(0, 1), "└──");
    }

    #[test]
    fn refresh_triggers_on_json_without_flag() {
        assert!(should_refresh_mr_info(false, true));
    }

    #[test]
    fn refresh_triggers_on_explicit_flag() {
        assert!(should_refresh_mr_info(true, false));
    }

    #[test]
    fn refresh_skipped_for_human_output_without_flag() {
        assert!(!should_refresh_mr_info(false, false));
    }
}
