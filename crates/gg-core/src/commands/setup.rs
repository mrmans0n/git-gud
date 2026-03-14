//! `gg setup` - Interactive config generator

use console::style;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, Select};

use crate::config::{Config, Defaults, UnstagedAction};
use crate::error::{GgError, Result};
use crate::git;
use crate::provider::Provider;

/// Print a styled group header for full setup mode
fn print_group_header(name: &str) {
    println!();
    println!("{}", style(format!("── {} ──", name)).cyan().bold());
}

/// Run the setup command
///
/// - Quick mode (all=false): Only essential settings (provider, base, username)
/// - Full mode (all=true): All settings organized into groups
pub fn run(all: bool) -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.commondir();
    let config_path = Config::config_path(git_dir);
    let mut config = Config::load(git_dir)?;
    let theme = ColorfulTheme::default();

    // Load global config to use as effective defaults
    let global = Config::load_global()?.unwrap_or_default();

    if config_path.exists() {
        let proceed = Confirm::with_theme(&theme)
            .with_prompt(format!(
                "Config already exists at {}. Update it?",
                config_path.display()
            ))
            .default(true)
            .interact()
            .unwrap_or(false);

        if !proceed {
            println!("{}", style("Setup cancelled.").dim());
            return Ok(());
        }
    } else {
        println!(
            "{}",
            style("Setting up git-gud for this repository...").bold()
        );
    }

    // Use global defaults as starting point when local config doesn't exist
    let effective_defaults = if config_path.exists() {
        &config.defaults
    } else {
        &global.defaults
    };

    let defaults = if all {
        prompt_defaults_full(&repo, effective_defaults, &theme)?
    } else {
        prompt_defaults_quick(&repo, effective_defaults, &theme)?
    };
    config.defaults = defaults;

    // worktree_base_path lives on Config, not Defaults
    if all {
        let effective_worktree = if config_path.exists() {
            config.worktree_base_path.as_deref()
        } else {
            global.worktree_base_path.as_deref()
        };
        config.worktree_base_path = prompt_worktree_base_path(effective_worktree, &theme)?;
    }

    config.save(git_dir)?;

    println!(
        "{} Wrote config to {}",
        style("OK").green().bold(),
        style(config_path.display()).cyan()
    );

    if !all {
        println!();
        println!(
            "{}",
            style(
                "Tip: Run 'gg setup --all' to configure advanced options (sync, land, lint, etc.)"
            )
            .dim()
        );
    }

    Ok(())
}

/// Quick mode: Only essential settings
fn prompt_defaults_quick(
    repo: &git2::Repository,
    existing: &Defaults,
    theme: &ColorfulTheme,
) -> Result<Defaults> {
    let mut defaults = existing.clone();

    defaults.provider = prompt_provider(repo, existing.provider.as_deref(), theme)?;
    defaults.base = prompt_base_branch(repo, existing.base.as_deref(), theme)?;
    defaults.branch_username = prompt_branch_username(
        existing.branch_username.as_deref(),
        defaults.provider.as_deref(),
        theme,
    )?;

    Ok(defaults)
}

/// Full mode: All settings organized into groups
fn prompt_defaults_full(
    repo: &git2::Repository,
    existing: &Defaults,
    theme: &ColorfulTheme,
) -> Result<Defaults> {
    let mut defaults = existing.clone();

    // ── General ──
    print_group_header("General");
    defaults.provider = prompt_provider(repo, existing.provider.as_deref(), theme)?;
    defaults.base = prompt_base_branch(repo, existing.base.as_deref(), theme)?;
    defaults.branch_username = prompt_branch_username(
        existing.branch_username.as_deref(),
        defaults.provider.as_deref(),
        theme,
    )?;
    defaults.auto_add_gg_ids = Confirm::with_theme(theme)
        .with_prompt("Automatically add GG-IDs to commits? (tracks commit-to-PR mapping)")
        .default(existing.auto_add_gg_ids)
        .interact()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;
    defaults.unstaged_action = prompt_unstaged_action(existing.unstaged_action, theme)?;

    // ── Sync ──
    print_group_header("Sync");
    defaults.sync_auto_rebase = Confirm::with_theme(theme)
        .with_prompt("Automatically rebase before sync when base is behind origin?")
        .default(existing.sync_auto_rebase)
        .interact()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;
    defaults.sync_behind_threshold = Input::with_theme(theme)
        .with_prompt("Number of commits behind origin before warning/rebase during sync")
        .default(existing.sync_behind_threshold)
        .interact_text()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;
    defaults.sync_draft = Confirm::with_theme(theme)
        .with_prompt("Create new PRs/MRs as drafts by default?")
        .default(existing.sync_draft)
        .interact()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;
    defaults.sync_update_descriptions = Confirm::with_theme(theme)
        .with_prompt("Update PR/MR descriptions on re-sync?")
        .default(existing.sync_update_descriptions)
        .interact()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;

    // ── Land ──
    print_group_header("Land");
    defaults.land_auto_clean = Confirm::with_theme(theme)
        .with_prompt("Automatically clean up stack after landing all PRs/MRs?")
        .default(existing.land_auto_clean)
        .interact()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;
    let effective_timeout = existing.land_wait_timeout_minutes.unwrap_or(30);
    let timeout: u64 = Input::with_theme(theme)
        .with_prompt("Timeout in minutes for `gg land --wait`")
        .default(effective_timeout)
        .interact_text()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;
    defaults.land_wait_timeout_minutes = Some(timeout);

    // ── Lint ──
    print_group_header("Lint");
    defaults.lint = prompt_lint_commands(repo, &existing.lint, theme)?;
    // Ask about auto-lint only if lint commands are configured
    if !defaults.lint.is_empty() {
        defaults.sync_auto_lint = prompt_sync_auto_lint(existing.sync_auto_lint, theme)?;
    }

    // ── GitLab ── (only if provider is GitLab)
    if defaults.provider.as_deref() == Some("gitlab") {
        print_group_header("GitLab");
        defaults.gitlab.auto_merge_on_land = Confirm::with_theme(theme)
            .with_prompt("Use GitLab auto-merge ('merge when pipeline succeeds') when landing?")
            .default(existing.gitlab.auto_merge_on_land)
            .interact()
            .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;
    }

    Ok(defaults)
}

fn prompt_sync_auto_lint(existing: bool, theme: &ColorfulTheme) -> Result<bool> {
    Confirm::with_theme(theme)
        .with_prompt("Run lint automatically before each sync?")
        .default(existing)
        .interact()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))
}

fn prompt_base_branch(
    repo: &git2::Repository,
    existing: Option<&str>,
    theme: &ColorfulTheme,
) -> Result<Option<String>> {
    let suggested = existing
        .map(|s| s.to_string())
        .or_else(|| git::find_base_branch(repo).ok());

    if let Some(suggested) = suggested {
        let prompt = if existing.is_some() {
            format!("Keep default base branch '{}'?", suggested)
        } else {
            format!("Use '{}' as the default base branch?", suggested)
        };

        let keep = Confirm::with_theme(theme)
            .with_prompt(prompt)
            .default(true)
            .interact()
            .unwrap_or(true);

        if keep {
            return Ok(Some(suggested));
        }

        let clear = Confirm::with_theme(theme)
            .with_prompt("Clear default base branch (auto-detect per repo)?")
            .default(false)
            .interact()
            .unwrap_or(false);

        if clear {
            return Ok(None);
        }

        let input: String = Input::with_theme(theme)
            .with_prompt("Default base branch")
            .interact_text()
            .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;

        let trimmed = input.trim();
        return Ok(if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        });
    }

    let set_base = Confirm::with_theme(theme)
        .with_prompt("Set a default base branch now?")
        .default(false)
        .interact()
        .unwrap_or(false);

    if !set_base {
        return Ok(None);
    }

    let input: String = Input::with_theme(theme)
        .with_prompt("Default base branch")
        .interact_text()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;

    let trimmed = input.trim();
    Ok(if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    })
}

fn prompt_provider(
    repo: &git2::Repository,
    existing: Option<&str>,
    theme: &ColorfulTheme,
) -> Result<Option<String>> {
    // Detect default based on remote URL
    let remote_url = repo
        .find_remote("origin")
        .ok()
        .and_then(|r| r.url().map(|s| s.to_string()));

    let detected_default = remote_url.as_ref().and_then(|url| {
        if url.contains("github.com") {
            Some(0usize) // GitHub
        } else if url.to_lowercase().contains("gitlab") {
            Some(1usize) // GitLab (any domain containing "gitlab")
        } else {
            None
        }
    });

    let providers = &["GitHub", "GitLab"];

    // If we have an existing value, use that as default
    let existing_index = existing.and_then(|p| match p.to_lowercase().as_str() {
        "github" => Some(0),
        "gitlab" => Some(1),
        _ => None,
    });

    let default_index = existing_index.or(detected_default);

    // Show URL for context if we couldn't auto-detect
    if default_index.is_none() {
        if let Some(url) = &remote_url {
            println!("{}", style(format!("Remote URL: {}", url)).dim());
            println!(
                "{}",
                style("Could not auto-detect provider from URL.").yellow()
            );
        }
    }

    let selection = if let Some(default) = default_index {
        Select::with_theme(theme)
            .with_prompt("Git hosting provider")
            .items(providers)
            .default(default)
            .interact()
            .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?
    } else {
        // No default - user must choose
        Select::with_theme(theme)
            .with_prompt("Git hosting provider (required)")
            .items(providers)
            .interact()
            .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?
    };

    let provider = match selection {
        0 => "github",
        1 => "gitlab",
        _ => unreachable!(),
    };

    Ok(Some(provider.to_string()))
}

fn prompt_branch_username(
    existing: Option<&str>,
    provider: Option<&str>,
    theme: &ColorfulTheme,
) -> Result<Option<String>> {
    let suggested = existing.map(|s| s.to_string()).or_else(|| {
        // Try to get username from the configured provider
        provider
            .and_then(|p| Provider::from_name(p).ok())
            .and_then(|p| p.whoami().ok())
    });

    let input: String = if let Some(suggested) = suggested {
        Input::with_theme(theme)
            .with_prompt("Branch username (used for <user>/<stack> branches)")
            .default(suggested)
            .interact_text()
            .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?
    } else {
        Input::with_theme(theme)
            .with_prompt("Branch username (used for <user>/<stack> branches)")
            .interact_text()
            .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?
    };

    let trimmed = input.trim();
    Ok(if trimmed.is_empty() {
        None
    } else {
        git::validate_branch_username(trimmed)?;
        Some(trimmed.to_string())
    })
}

fn prompt_lint_commands(
    repo: &git2::Repository,
    existing: &[String],
    theme: &ColorfulTheme,
) -> Result<Vec<String>> {
    if !existing.is_empty() {
        println!("{}", style("Current lint commands:").dim());
        for cmd in existing {
            println!("  {}", style(cmd).dim());
        }

        let update = Confirm::with_theme(theme)
            .with_prompt("Update lint commands?")
            .default(false)
            .interact()
            .unwrap_or(false);

        if !update {
            return Ok(existing.to_vec());
        }
    }

    let mut lint = Vec::new();

    let suggestions = detect_lint_suggestions(repo);
    if !suggestions.is_empty() {
        println!("{}", style("Suggested lint commands:").dim());
        for cmd in &suggestions {
            println!("  {}", style(cmd).dim());
        }

        let include = Confirm::with_theme(theme)
            .with_prompt("Add suggested lint commands?")
            .default(true)
            .interact()
            .unwrap_or(false);

        if include {
            lint.extend(suggestions);
        }
    }

    let mut add_more = Confirm::with_theme(theme)
        .with_prompt("Add a lint command to run per commit?")
        .default(true)
        .interact()
        .unwrap_or(false);

    while add_more {
        let cmd: String = Input::with_theme(theme)
            .with_prompt("Lint command")
            .interact_text()
            .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;

        let trimmed = cmd.trim();
        if !trimmed.is_empty() && !lint.iter().any(|c| c == trimmed) {
            lint.push(trimmed.to_string());
        }

        add_more = Confirm::with_theme(theme)
            .with_prompt("Add another lint command?")
            .default(false)
            .interact()
            .unwrap_or(false);
    }

    Ok(lint)
}

fn prompt_unstaged_action(
    existing: UnstagedAction,
    theme: &ColorfulTheme,
) -> Result<UnstagedAction> {
    let options = &[
        "ask",
        "add (stage all)",
        "stash",
        "continue (ignore unstaged)",
        "abort",
    ];

    let default_index = match existing {
        UnstagedAction::Ask => 0,
        UnstagedAction::Add => 1,
        UnstagedAction::Stash => 2,
        UnstagedAction::Continue => 3,
        UnstagedAction::Abort => 4,
    };

    let selection = Select::with_theme(theme)
        .with_prompt("Default action for `gg amend` with unstaged changes")
        .items(options)
        .default(default_index)
        .interact()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;

    Ok(match selection {
        0 => UnstagedAction::Ask,
        1 => UnstagedAction::Add,
        2 => UnstagedAction::Stash,
        3 => UnstagedAction::Continue,
        4 => UnstagedAction::Abort,
        _ => unreachable!(),
    })
}

fn prompt_worktree_base_path(
    existing: Option<&str>,
    theme: &ColorfulTheme,
) -> Result<Option<String>> {
    print_group_header("Worktrees");

    let input: String = Input::with_theme(theme)
        .with_prompt(
            "Base path template for stack worktrees (variables: {repo}, {stack}, leave empty to disable)",
        )
        .default(existing.unwrap_or("").to_string())
        .allow_empty(true)
        .interact_text()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;

    let trimmed = input.trim();
    Ok(if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    })
}

fn detect_lint_suggestions(repo: &git2::Repository) -> Vec<String> {
    let mut suggestions = Vec::new();

    let Some(workdir) = repo.workdir() else {
        return suggestions;
    };

    if workdir.join("Cargo.toml").exists() {
        suggestions.push("cargo fmt --check".to_string());
        suggestions.push("cargo clippy -- -D warnings".to_string());
    }

    suggestions
}
