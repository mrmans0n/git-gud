//! `gg ls` - List current stack or all stacks

use console::style;

use crate::config::Config;
use crate::error::Result;
use crate::git;
use crate::output::{
    print_json, AllStacksResponse, RemoteStackJson, RemoteStacksResponse, SingleStackResponse,
    StackCommitJson, StackEntryJson, StackJson, StackSummaryJson, OUTPUT_VERSION,
};
use crate::provider::{CiStatus, PrState, Provider};
use crate::stack::{self, Stack};

/// Run the list command
pub fn run(all: bool, refresh: bool, remote: bool, json: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let config = Config::load(git_dir)?;

    // Handle --remote flag
    if remote {
        return list_remote_stacks(&repo, &config, json);
    }

    // Try to load current stack
    let current_stack = Stack::load(&repo, &config).ok();

    match current_stack {
        None => {
            list_all_stacks(&repo, &config, json)?;
        }
        Some(mut stack) if !all => {
            if refresh {
                let provider = Provider::detect(&repo)?;
                if !json {
                    print!("Refreshing {} status... ", provider.pr_label());
                }
                stack.refresh_mr_info(&provider)?;
                if !json {
                    println!("{}", style("done").green());
                }
            }

            show_stack(&stack, json)?;
        }
        Some(_) => {
            list_all_stacks(&repo, &config, json)?;
        }
    }

    Ok(())
}

/// List all available stacks with their commits in a tree view
fn list_all_stacks(repo: &git2::Repository, config: &Config, json: bool) -> Result<()> {
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| Provider::detect(repo).ok().and_then(|p| p.whoami().ok()))
        .unwrap_or_else(|| "unknown".to_string());

    git::validate_branch_username(&username)?;

    let stacks = stack::list_all_stacks(repo, config, &username)?;

    // Get current branch to highlight active stack
    let current_branch = git::current_branch_name(repo);
    let current_stack = current_branch
        .as_ref()
        .and_then(|b| git::parse_stack_branch(b))
        .map(|(_, name)| name);

    // Get base branch for commit listing
    let base_branch = git::find_base_branch(repo).unwrap_or_else(|_| "main".to_string());

    if json {
        let summaries = stacks
            .iter()
            .map(|stack_name| {
                let is_current = current_stack.as_deref() == Some(stack_name);
                let has_worktree = config
                    .get_stack(stack_name)
                    .and_then(|s| s.worktree_path.as_ref())
                    .is_some();

                let full_branch = git::format_stack_branch(&username, stack_name);
                let commits =
                    get_stack_commits_info(repo, &full_branch, &base_branch).unwrap_or_default();
                let commit_count = commits.len();
                let commits = commits
                    .into_iter()
                    .enumerate()
                    .map(|(i, (sha, title))| StackCommitJson {
                        position: i + 1,
                        sha,
                        title,
                    })
                    .collect();

                let stack_base = config
                    .get_base_for_stack(stack_name)
                    .unwrap_or(base_branch.as_str())
                    .to_string();

                StackSummaryJson {
                    name: stack_name.clone(),
                    base: stack_base.clone(),
                    commit_count,
                    is_current,
                    has_worktree,
                    behind_base: behind_count(repo, &stack_base),
                    commits,
                }
            })
            .collect();

        print_json(&AllStacksResponse {
            version: OUTPUT_VERSION,
            current_stack,
            stacks: summaries,
        });
        return Ok(());
    }

    if stacks.is_empty() {
        println!(
            "{}",
            style("No stacks found. Use `gg co <name>` to create one.").dim()
        );
        return Ok(());
    }

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

        let full_branch = git::format_stack_branch(&username, stack_name);
        let commits = get_stack_commits_info(repo, &full_branch, &base_branch);

        let commit_count = commits.as_ref().map(|c| c.len()).unwrap_or(0);
        let commit_info = format!(" ({} commits)", commit_count);
        let stack_base = config
            .get_base_for_stack(stack_name)
            .unwrap_or(base_branch.as_str());
        let behind_indicator = behind_indicator(repo, stack_base)
            .map(|s| format!(" {}", style(s).yellow()))
            .unwrap_or_default();

        println!();
        if is_current {
            println!(
                "{}{}{}{}{}",
                style(marker).cyan().bold(),
                style(stack_name).cyan().bold(),
                style(wt_indicator).yellow(),
                style(&commit_info).dim(),
                behind_indicator
            );
        } else {
            println!(
                "{}{}{}{}{}",
                marker,
                stack_name,
                style(wt_indicator).yellow(),
                style(&commit_info).dim(),
                behind_indicator
            );
        }

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
fn list_remote_stacks(repo: &git2::Repository, config: &Config, json: bool) -> Result<()> {
    let username = config
        .defaults
        .branch_username
        .clone()
        .or_else(|| Provider::detect(repo).ok().and_then(|p| p.whoami().ok()))
        .unwrap_or_else(|| "unknown".to_string());

    git::validate_branch_username(&username)?;

    if !json {
        println!("{}", style("Fetching from origin...").dim());
    }
    let _ = std::process::Command::new("git")
        .args(["fetch", "origin", "--prune"])
        .output();

    let local_stacks = stack::list_all_stacks(repo, config, &username)?;

    let mut remote_stacks: Vec<String> = Vec::new();
    let branches = repo.branches(Some(git2::BranchType::Remote))?;

    for branch_result in branches {
        let (branch, _) = branch_result?;
        if let Some(name) = branch.name()? {
            if let Some(branch_name) = name.strip_prefix("origin/") {
                if let Some((branch_user, stack_name)) = git::parse_stack_branch(branch_name) {
                    if branch_user == username
                        && !local_stacks.contains(&stack_name)
                        && !remote_stacks.contains(&stack_name)
                    {
                        remote_stacks.push(stack_name);
                    }
                } else if let Some((branch_user, stack_name, _entry_id)) =
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

    remote_stacks.sort();

    if json {
        let base_branch = git::find_base_branch(repo).unwrap_or_else(|_| "main".to_string());
        let stacks = remote_stacks
            .iter()
            .map(|stack_name| {
                let remote_branch = format!("origin/{}/{}", username, stack_name);
                let commit_count =
                    count_stack_commits(repo, &remote_branch, &base_branch).unwrap_or(0);

                let mut pr_numbers = config
                    .get_stack(stack_name)
                    .map(|stack_config| stack_config.mrs.values().copied().collect::<Vec<u64>>())
                    .unwrap_or_default();
                pr_numbers.sort_unstable();
                pr_numbers.dedup();

                RemoteStackJson {
                    name: stack_name.clone(),
                    commit_count,
                    pr_numbers,
                }
            })
            .collect();

        print_json(&RemoteStacksResponse {
            version: OUTPUT_VERSION,
            stacks,
        });
        return Ok(());
    }

    if remote_stacks.is_empty() {
        println!(
            "{}",
            style("No remote stacks found that aren't already checked out locally.").dim()
        );
        return Ok(());
    }

    let provider = Provider::detect(repo).ok();

    println!("{}", style("Remote stacks:").bold());
    println!();

    for stack_name in &remote_stacks {
        let remote_branch = format!("origin/{}/{}", username, stack_name);

        let commit_info = if let Ok(base) = git::find_base_branch(repo) {
            if let Ok(count) = count_stack_commits(repo, &remote_branch, &base) {
                format!(" ({} commits)", count)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

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

fn behind_count(repo: &git2::Repository, base_branch: &str) -> Option<usize> {
    let behind =
        git::count_commits_behind(repo, base_branch, &format!("origin/{}", base_branch)).ok()?;
    if behind > 0 {
        Some(behind)
    } else {
        None
    }
}

fn behind_indicator(repo: &git2::Repository, base_branch: &str) -> Option<String> {
    behind_count(repo, base_branch).map(|behind| format!("↓{}", behind))
}

/// Show detailed stack view
fn show_stack(stack: &Stack, json: bool) -> Result<()> {
    let synced = stack.synced_count();
    let total = stack.len();

    let repo = git::open_repo()?;

    if json {
        let current_pos = stack
            .current_position
            .unwrap_or(stack.len().saturating_sub(1));

        let entries = stack
            .entries
            .iter()
            .map(|entry| {
                let is_current = entry.position == current_pos + 1
                    || (stack.current_position.is_none() && entry.position == stack.len());

                StackEntryJson {
                    position: entry.position,
                    sha: entry.short_sha.clone(),
                    title: entry.title.clone(),
                    gg_id: entry.gg_id.clone(),
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

        print_json(&SingleStackResponse {
            version: OUTPUT_VERSION,
            stack: StackJson {
                name: stack.name.clone(),
                base: stack.base.clone(),
                total_commits: total,
                synced_commits: synced,
                current_position: stack.current_position.map(|p| p + 1),
                behind_base: behind_count(&repo, &stack.base),
                entries,
            },
        });

        return Ok(());
    }

    let behind = behind_indicator(&repo, &stack.base)
        .map(|s| format!(" {}", style(s).yellow()))
        .unwrap_or_default();

    println!(
        "{} ({} commits, {} synced){}",
        style(&stack.name).cyan().bold(),
        total,
        synced,
        behind
    );
    println!();

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

    let provider = Provider::detect(&repo).ok();
    let pr_prefix = provider
        .as_ref()
        .map(|p| p.pr_number_prefix())
        .unwrap_or("!");

    let current_pos = stack
        .current_position
        .unwrap_or(stack.len().saturating_sub(1));

    for entry in &stack.entries {
        let is_current = entry.position == current_pos + 1
            || (stack.current_position.is_none() && entry.position == stack.len());

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
        let gg_id = entry.gg_id.as_deref().unwrap_or("-");
        let mr_display = entry
            .mr_number
            .map(|n| format!("{}{}", pr_prefix, n))
            .unwrap_or_default();
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

        if !mr_display.is_empty() {
            let mut mr_line = mr_display.clone();

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
