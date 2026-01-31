//! `gg setup` - Interactive config generator

use console::style;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input};

use crate::config::{Config, Defaults};
use crate::error::{GgError, Result};
use crate::git;
use crate::provider::{self, ProviderType};

/// Run the setup command
pub fn run() -> Result<()> {
    let repo = git::open_repo()?;
    let git_dir = repo.path();
    let config_path = Config::config_path(git_dir);
    let mut config = Config::load(git_dir)?;
    let theme = ColorfulTheme::default();

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

    let defaults = prompt_defaults(&repo, &config.defaults, &theme)?;
    config.defaults = defaults;
    config.save(git_dir)?;

    println!(
        "{} Wrote config to {}",
        style("OK").green().bold(),
        style(config_path.display()).cyan()
    );

    Ok(())
}

fn prompt_defaults(
    repo: &git2::Repository,
    existing: &Defaults,
    theme: &ColorfulTheme,
) -> Result<Defaults> {
    let mut defaults = existing.clone();

    defaults.base = prompt_base_branch(repo, existing.base.as_deref(), theme)?;
    defaults.provider = prompt_provider(repo, existing.provider.as_ref(), theme)?;

    // Get provider for whoami fallback - use configured provider or detect from remote
    let provider_for_whoami = defaults
        .provider
        .clone()
        .or_else(|| provider::detect_provider(repo));

    defaults.branch_username =
        prompt_branch_username(existing.branch_username.as_deref(), provider_for_whoami.as_ref(), theme)?;
    defaults.lint = prompt_lint_commands(repo, &existing.lint, theme)?;

    Ok(defaults)
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
    existing: Option<&ProviderType>,
    theme: &ColorfulTheme,
) -> Result<Option<ProviderType>> {
    let detected = provider::detect_provider(repo);

    // Build options
    let options = ["Auto-detect", "GitHub", "GitLab"];
    let default_idx = match existing.or(detected.as_ref()) {
        Some(ProviderType::GitHub) => 1,
        Some(ProviderType::GitLab) => 2,
        None => 0,
    };

    let detection_hint = match &detected {
        Some(ProviderType::GitHub) => " (detected: GitHub)",
        Some(ProviderType::GitLab) => " (detected: GitLab)",
        None => " (could not detect from remote)",
    };

    let selection = dialoguer::Select::with_theme(theme)
        .with_prompt(format!("Git hosting provider{}", detection_hint))
        .items(options)
        .default(default_idx)
        .interact()
        .map_err(|e| GgError::Other(format!("Prompt failed: {}", e)))?;

    Ok(match selection {
        0 => None, // Auto-detect
        1 => Some(ProviderType::GitHub),
        2 => Some(ProviderType::GitLab),
        _ => None,
    })
}

fn prompt_branch_username(
    existing: Option<&str>,
    provider_type: Option<&ProviderType>,
    theme: &ColorfulTheme,
) -> Result<Option<String>> {
    // Try to get username from provider
    let whoami_result = provider_type.and_then(|pt| {
        let provider: Box<dyn provider::Provider> = match pt {
            ProviderType::GitHub => Box::new(provider::github::GitHubProvider),
            ProviderType::GitLab => Box::new(provider::gitlab::GitLabProvider),
        };
        provider.whoami().ok()
    });

    let suggested = existing.map(|s| s.to_string()).or(whoami_result);

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
