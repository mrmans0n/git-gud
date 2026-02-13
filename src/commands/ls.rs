//! `gg ls` - List current stack or all stacks

use console::style;

use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::provider::{CiStatus, PrState, Provider};
use crate::stack::{self, Stack};

/// Run the list command
pub fn run(all: bool, refresh: bool, remote: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let config = Config::load(repo.commondir())?;

    // Handle --remote flag
    if remote {
        return list_remote_stacks(&repo, &config);
    }

    // Try to load current stack
    let current_stack = Stack::load(&repo, &config).ok();

    match current_stack {
        None => {
            // List all stacks
            list_all_stacks(&repo, &config)?;
        }
        Some(mut stack) if !all => {
            // Show current stack details
            if refresh {
                // Detect provider for refresh
                let provider = Provider::detect(&repo)?;
                print!("Refreshing MR status... ");
                stack.refresh_mr_info(&provider)?;
                println!("{}", style("done").green());
            }

            show_stack(&stack)?;
        }
        Some(_) => {
            // List all stacks (--all flag)
            list_all_stacks(&repo, &config)?;
        }
    }

    Ok(())
}

/// List all available stacks with their commits in a tree view
fn list_all_stacks(repo: &git2::Repository, config: &Config) -> Result<()> {
    // Get username - try provider if in a repo
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| Provider::detect(repo).ok().and_then(|p| p.whoami().ok()))
        .unwrap_or_else(|| "unknown".to_string());

    git::validate_branch_username(&username)?;

    let stacks = stack::list_all_stacks(repo, config, &username)?;

    if stacks.is_empty() {
        println!(
            "{}",
            style("No stacks found. Use `gg co <name>` to create one.").dim()
        );
        return Ok(());
    }

    // Get current branch to highlight active stack
    let current_branch = git::current_branch_name(repo);
    let current_stack = current_branch
        .as_ref()
        .and_then(|b| git::parse_stack_branch(b))
        .map(|(_, name)| name);

    // Get base branch for commit listing
    let base_branch = git::find_base_branch(repo).unwrap_or_else(|_| "main".to_string());

    println!("{}", style("Stacks:").bold());

    for stack_name in &stacks {
        let is_current = current_stack.as_deref() == Some(stack_name);
        let marker = if is_current { "→ " } else { "  " };
        let wt_indicator = if config
            .get_stack(stack_name)
            .and_then(|s| s.worktree_path.as_ref())
            .is_some()
        {
            " [wt]"
        } else {
            ""
        };

        // Get commits for this stack
        let full_branch = git::format_stack_branch(&username, stack_name);
        let commits = get_stack_commits_info(repo, &full_branch, &base_branch);

        let commit_count = commits.as_ref().map(|c| c.len()).unwrap_or(0);
        let commit_info = format!(" ({} commits)", commit_count);

        println!();
        if is_current {
            println!(
                "{}{}{}{}",
                style(marker).cyan().bold(),
                style(stack_name).cyan().bold(),
                style(wt_indicator).yellow(),
                style(&commit_info).dim()
            );
        } else {
            println!(
                "{}{}{}{}",
                marker,
                stack_name,
                style(wt_indicator).yellow(),
                style(&commit_info).dim()
            );
        }

        // Show commits in tree format
        if let Ok(ref commits) = commits {
            let total = commits.len();
            for (i, (sha, title)) in commits.iter().enumerate() {
                let is_last = i == total - 1;
                let branch_char = if is_last { "└──" } else { "├──" };
                let position = i + 1;

                if is_current {
                    println!(
                        "    {} {} {} {}",
                        style(branch_char).dim(),
                        style(format!("[{}]", position)).dim(),
                        style(sha).yellow(),
                        title
                    );
                } else {
                    println!(
                        "    {} {} {} {}",
                        style(branch_char).dim(),
                        style(format!("[{}]", position)).dim(),
                        style(sha).yellow().dim(),
                        style(title).dim()
                    );
                }
            }
        }
    }

    println!();
    println!(
        "{}",
        style("Use `gg co <name>` to switch stacks, or `gg ls` while on a stack to see details.")
            .dim()
    );

    Ok(())
}

/// Get commit info (sha, title) for a stack branch
fn get_stack_commits_info(
    repo: &git2::Repository,
    branch: &str,
    base: &str,
) -> Result<Vec<(String, String)>> {
    use git2::Sort;

    let head = repo.revparse_single(branch)?;
    let base_ref = repo
        .revparse_single(base)
        .or_else(|_| repo.revparse_single(&format!("origin/{}", base)))?;

    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
    revwalk.push(head.id())?;
    revwalk.hide(base_ref.id())?;

    let mut commits = Vec::new();
    for oid in revwalk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        let sha = oid.to_string()[..7].to_string();
        let title = commit.summary().unwrap_or("<no message>").to_string();
        commits.push((sha, title));
    }

    Ok(commits)
}

/// List remote stacks that aren't checked out locally
fn list_remote_stacks(repo: &git2::Repository, config: &Config) -> Result<()> {
    // Get username
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| Provider::detect(repo).ok().and_then(|p| p.whoami().ok()))
        .unwrap_or_else(|| "unknown".to_string());

    git::validate_branch_username(&username)?;

    // Fetch latest from origin first
    println!("{}", style("Fetching from origin...").dim());
    let _ = std::process::Command::new("git")
        .args(["fetch", "origin", "--prune"])
        .output();

    // Get local stacks for comparison
    let local_stacks = stack::list_all_stacks(repo, config, &username)?;

    // Scan remote branches (both stack branches and entry branches)
    let mut remote_stacks: Vec<String> = Vec::new();
    let branches = repo.branches(Some(git2::BranchType::Remote))?;

    for branch_result in branches {
        let (branch, _) = branch_result?;
        if let Some(name) = branch.name()? {
            // Remote branches are prefixed with "origin/"
            if let Some(branch_name) = name.strip_prefix("origin/") {
                // Check if it's a stack branch (username/stack-name)
                if let Some((branch_user, stack_name)) = git::parse_stack_branch(branch_name) {
                    if branch_user == username
                        && !local_stacks.contains(&stack_name)
                        && !remote_stacks.contains(&stack_name)
                    {
                        remote_stacks.push(stack_name);
                    }
                }
                // Also check entry branches (username/stack-name--entry-id)
                else if let Some((branch_user, stack_name, _entry_id)) =
                    git::parse_entry_branch(branch_name)
                {
                    if branch_user == username
                        && !local_stacks.contains(&stack_name)
                        && !remote_stacks.contains(&stack_name)
                    {
                        remote_stacks.push(stack_name);
                    }
                }
            }
        }
    }

    if remote_stacks.is_empty() {
        println!(
            "{}",
            style("No remote stacks found that aren't already checked out locally.").dim()
        );
        return Ok(());
    }

    remote_stacks.sort();

    // Try to get provider for MR info
    let provider = Provider::detect(repo).ok();

    println!("{}", style("Remote stacks:").bold());
    println!();

    for stack_name in &remote_stacks {
        let remote_branch = format!("origin/{}/{}", username, stack_name);

        // Count commits
        let commit_info = if let Ok(base) = git::find_base_branch(repo) {
            if let Ok(count) = count_stack_commits(repo, &remote_branch, &base) {
                format!(" ({} commits)", count)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Try to get MR info if we have a provider and config
        let mr_info = if provider.is_some() {
            if let Some(stack_config) = config.get_stack(stack_name) {
                let mrs: Vec<String> = stack_config
                    .mrs
                    .values()
                    .map(|n| format!("#{}", n))
                    .collect();
                if !mrs.is_empty() {
                    format!(" [{}]", mrs.join(", "))
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        println!(
            "  {} {}{}{}",
            style("○").dim(),
            style(stack_name).cyan(),
            style(&commit_info).dim(),
            style(&mr_info).blue()
        );
    }

    println!();
    println!(
        "{}",
        style("Use `gg co <name>` to check out a remote stack.").dim()
    );

    Ok(())
}

/// Count commits in a stack branch
fn count_stack_commits(repo: &git2::Repository, branch: &str, base: &str) -> Result<usize> {
    let head = repo.revparse_single(branch)?;
    let base_ref = repo
        .revparse_single(base)
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

    // Check for ongoing rebase and warn the user
    let repo = git::open_repo()?;
    if git::is_rebase_in_progress(&repo) {
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
            style("  No commits yet. Use `git commit` to add changes.").dim()
        );
        return Ok(());
    }

    // Try to detect provider for proper PR/MR prefix
    let repo = git::open_repo()?;
    let provider = Provider::detect(&repo).ok();
    let pr_prefix = provider
        .as_ref()
        .map(|p| p.pr_number_prefix())
        .unwrap_or("!");

    // Determine the current position
    let current_pos = stack
        .current_position
        .unwrap_or(stack.len().saturating_sub(1));

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
            Some(PrState::Merged) => style(&status).green(),
            Some(PrState::Closed) => style(&status).red(),
            Some(PrState::Draft) => style(&status).dim(),
            Some(PrState::Open) if entry.approved => style(&status).green(),
            Some(PrState::Open) => style(&status).yellow(),
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

        // Merge train indicator
        let train = if entry.in_merge_train { " 🚂" } else { "" };

        // GG-ID display
        let gg_id = entry.gg_id.as_deref().unwrap_or("-");

        // MR number
        let mr_display = entry
            .mr_number
            .map(|n| format!("{}{}", pr_prefix, n))
            .unwrap_or_default();

        // HEAD marker
        let head_marker = if is_current { " <- HEAD" } else { "" };

        if is_current {
            println!(
                "  {} {} {} {} {}{} (id: {}){}",
                style(&position).bold(),
                style(sha).yellow().bold(),
                style(title).bold(),
                status_styled,
                ci,
                train,
                style(gg_id).dim(),
                style(head_marker).cyan().bold()
            );
        } else {
            println!(
                "  {} {} {} {} {}{} (id: {})",
                style(&position).dim(),
                style(sha).yellow(),
                title,
                status_styled,
                ci,
                train,
                style(gg_id).dim()
            );
        }

        // Show MR link and merge train info if available
        if !mr_display.is_empty() {
            let mut mr_line = mr_display.clone();

            // Add merge train indicator if in train
            if entry.in_merge_train {
                if let Some(pos) = entry.merge_train_position {
                    mr_line.push_str(&format!(" [train pos {}]", pos));
                } else {
                    mr_line.push_str(" [train]");
                }
            }

            println!("      {}", style(&mr_line).blue());
        }
    }

    println!();

    Ok(())
}
