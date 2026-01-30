//! `gg ls` - List current stack or all stacks

use console::style;

use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::glab::{self, CiStatus, MrState};
use crate::stack::{self, Stack};

/// Run the list command
pub fn run(all: bool, refresh: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let config = Config::load(git_dir)?;

    // Try to load current stack
    let current_stack = Stack::load(&repo, &config).ok();

    if all || current_stack.is_none() {
        // List all stacks
        list_all_stacks(&repo, &config)?;
    } else {
        // Show current stack details
        let mut stack = current_stack.unwrap();

        if refresh {
            print!("Refreshing MR status... ");
            stack.refresh_mr_info()?;
            println!("{}", style("done").green());
        }

        show_stack(&stack)?;
    }

    Ok(())
}

/// List all available stacks
fn list_all_stacks(repo: &git2::Repository, config: &Config) -> Result<()> {
    // Get username
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| glab::whoami().ok())
        .unwrap_or_else(|| "unknown".to_string());

    let stacks = stack::list_all_stacks(repo, config, &username)?;

    if stacks.is_empty() {
        println!("{}", style("No stacks found. Use `gg co <name>` to create one.").dim());
        return Ok(());
    }

    // Get current branch to highlight active stack
    let current_branch = git::current_branch_name(repo);
    let current_stack = current_branch
        .as_ref()
        .and_then(|b| git::parse_stack_branch(b))
        .map(|(_, name)| name);

    println!("{}", style("Stacks:").bold());
    println!();

    for stack_name in &stacks {
        let is_current = current_stack.as_ref().map(|s| s.as_str()) == Some(stack_name);
        let marker = if is_current { "→ " } else { "  " };

        // Count commits in stack if we can
        let commit_info = if let Ok(branch_name) = git::find_base_branch(repo) {
            let full_branch = git::format_stack_branch(&username, stack_name);
            if let Ok(stack_commits) = count_stack_commits(repo, &full_branch, &branch_name) {
                format!(" ({} commits)", stack_commits)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        if is_current {
            println!(
                "{}{}{}",
                style(marker).cyan().bold(),
                style(stack_name).cyan().bold(),
                style(&commit_info).dim()
            );
        } else {
            println!(
                "{}{}{}",
                marker,
                stack_name,
                style(&commit_info).dim()
            );
        }
    }

    println!();
    println!(
        "{}",
        style("Use `gg co <name>` to switch stacks, or `gg ls` while on a stack to see details.").dim()
    );

    Ok(())
}

/// Count commits in a stack branch
fn count_stack_commits(repo: &git2::Repository, branch: &str, base: &str) -> Result<usize> {
    let head = repo.revparse_single(branch)?;
    let base_ref = repo.revparse_single(base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", base)))?;

    let mut revwalk = repo.revwalk()?;
    revwalk.push(head.id())?;
    revwalk.hide(base_ref.id())?;

    Ok(revwalk.count())
}

/// Show detailed stack view
fn show_stack(stack: &Stack) -> Result<()> {
    let synced = stack.synced_count();
    let total = stack.len();

    println!(
        "{} ({} commits, {} synced)",
        style(&stack.name).cyan().bold(),
        total,
        synced
    );
    println!();

    if stack.is_empty() {
        println!(
            "{}",
            style("  No commits yet. Use `git commit` to add changes.").dim()
        );
        return Ok(());
    }

    // Determine the current position
    let current_pos = stack.current_position.unwrap_or(stack.len().saturating_sub(1));

    for entry in &stack.entries {
        let is_current = entry.position == current_pos + 1
            || (stack.current_position.is_none() && entry.position == stack.len());

        // Build the line
        let position = format!("[{}]", entry.position);
        let sha = &entry.short_sha;
        let title = &entry.title;

        // Status indicator
        let status = entry.status_display();
        let status_styled = match &entry.mr_state {
            Some(MrState::Merged) => style(&status).green(),
            Some(MrState::Closed) => style(&status).red(),
            Some(MrState::Draft) => style(&status).dim(),
            Some(MrState::Open) if entry.approved => style(&status).green(),
            Some(MrState::Open) => style(&status).yellow(),
            None => style(&status).dim(),
        };

        // CI indicator
        let ci = match &entry.ci_status {
            Some(CiStatus::Success) => style("✓").green().to_string(),
            Some(CiStatus::Failed) => style("✗").red().to_string(),
            Some(CiStatus::Running) => style("●").yellow().to_string(),
            Some(CiStatus::Pending) => style("○").dim().to_string(),
            _ => String::new(),
        };

        // GG-ID display
        let gg_id = entry.gg_id.as_deref().unwrap_or("-");

        // MR number
        let mr_display = entry
            .mr_number
            .map(|n| format!("!{}", n))
            .unwrap_or_default();

        // HEAD marker
        let head_marker = if is_current { " <- HEAD" } else { "" };

        if is_current {
            println!(
                "  {} {} {} {} {} (id: {}){}",
                style(&position).bold(),
                style(sha).yellow().bold(),
                style(title).bold(),
                status_styled,
                ci,
                style(gg_id).dim(),
                style(head_marker).cyan().bold()
            );
        } else {
            println!(
                "  {} {} {} {} {} (id: {})",
                style(&position).dim(),
                style(sha).yellow(),
                title,
                status_styled,
                ci,
                style(gg_id).dim()
            );
        }

        // Show MR link if available
        if !mr_display.is_empty() {
            println!("      {}", style(&mr_display).blue());
        }
    }

    println!();

    Ok(())
}
