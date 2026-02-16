//! Git configuration and repository cloning.

use std::path::Path;

use tokio::process::Command;
use tracing::{info, warn};

use crate::config::Config;

/// Configure git authentication using `gh auth setup-git`.
///
/// Also resolves git user name/email from the GitHub API via `gh api user`.
pub async fn configure_git(_config: &Config) -> Result<(), String> {
    // Only configure if GITHUB_TOKEN is available
    if std::env::var("GITHUB_TOKEN").is_err() {
        info!("GITHUB_TOKEN not set — skipping git configuration");
        return Ok(());
    }

    // Configure gh CLI to use git credential helper
    run_command("gh", &["auth", "setup-git"]).await?;

    // SSH to HTTPS conversion
    run_command(
        "git",
        &[
            "config",
            "--global",
            "url.https://github.com/.insteadOf",
            "git@github.com:",
        ],
    )
    .await?;

    // Resolve git user info from GitHub API
    let user_name = std::env::var("GIT_USER_NAME").ok();
    let user_email = std::env::var("GIT_USER_EMAIL").ok();

    if user_name.is_none() || user_email.is_none() {
        match resolve_github_user().await {
            Ok((name, email)) => {
                if user_name.is_none() {
                    if let Some(ref n) = name {
                        run_command("git", &["config", "--global", "user.name", n]).await?;
                    }
                }
                if user_email.is_none() {
                    if let Some(ref e) = email {
                        run_command("git", &["config", "--global", "user.email", e]).await?;
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to resolve GitHub user info");
            }
        }
    }

    // Set explicit user name/email if provided via env vars
    if let Some(ref name) = user_name {
        run_command("git", &["config", "--global", "user.name", name]).await?;
    }
    if let Some(ref email) = user_email {
        run_command("git", &["config", "--global", "user.email", email]).await?;
    }

    info!("Git authentication configured");
    Ok(())
}

/// Clone a repository, trying `gh repo clone` first then falling back to `git clone`.
pub async fn clone_repository(config: &Config) -> Result<(), String> {
    let Some(ref clone_url) = config.git_clone_url else {
        return Ok(());
    };

    // Skip if already cloned
    if Path::new(&config.workspace_dir).join(".git").exists() {
        info!("Repository already cloned, skipping");
        return Ok(());
    }

    // Extract owner/repo from URL
    let repo_path = clone_url
        .replace("https://github.com/", "")
        .replace("git@github.com:", "")
        .trim_end_matches(".git")
        .to_string();

    info!(repo = %repo_path, branch = ?config.git_branch, "Cloning repository");

    // Try gh repo clone first
    let mut args = vec!["repo", "clone", &repo_path, &config.workspace_dir, "--"];
    args.push("--depth");
    args.push("1");

    let branch_owned;
    if let Some(ref branch) = config.git_branch {
        args.push("--branch");
        branch_owned = branch.clone();
        args.push(&branch_owned);
    }

    match run_command("gh", &args).await {
        Ok(()) => {
            info!("Repository cloned successfully");
            return Ok(());
        }
        Err(e) => {
            warn!(error = %e, "gh repo clone failed, falling back to git clone");
        }
    }

    // Fallback: git clone with HTTPS token auth (if available)
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        let https_url = format!("https://x-access-token:{token}@github.com/{repo_path}.git");
        let mut git_args = vec!["clone", "--depth", "1"];
        if let Some(ref branch) = config.git_branch {
            git_args.push("--branch");
            git_args.push(branch);
        }
        git_args.push(&https_url);
        git_args.push(&config.workspace_dir);
        run_command("git", &git_args).await?;
        info!("Repository cloned successfully (git fallback with token)");
        return Ok(());
    }

    // Last resort: plain git clone (works for public repos)
    let plain_url = format!("https://github.com/{repo_path}.git");
    let mut git_args = vec!["clone", "--depth", "1"];
    if let Some(ref branch) = config.git_branch {
        git_args.push("--branch");
        git_args.push(branch);
    }
    git_args.push(&plain_url);
    git_args.push(&config.workspace_dir);
    match run_command("git", &git_args).await {
        Ok(()) => {
            info!("Repository cloned successfully (plain git clone)");
            Ok(())
        }
        Err(e) => {
            Err(format!("All clone methods failed. Last error: {e}"))
        }
    }
}

/// Resolve git user name and email from the GitHub API.
async fn resolve_github_user() -> Result<(Option<String>, Option<String>), String> {
    let output = Command::new("gh")
        .args(["api", "user"])
        .output()
        .await
        .map_err(|e| format!("Failed to run gh api user: {e}"))?;

    if !output.status.success() {
        return Err("gh api user failed".to_string());
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("Failed to parse: {e}"))?;

    let name = json
        .get("name")
        .and_then(|v| v.as_str())
        .or_else(|| json.get("login").and_then(|v| v.as_str()))
        .map(String::from);

    let email = json
        .get("email")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            json.get("login")
                .and_then(|v| v.as_str())
                .map(|login| format!("{login}@users.noreply.github.com"))
        });

    Ok((name, email))
}

/// Run a command and return an error if it fails.
async fn run_command(program: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("Failed to run {program}: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{program} failed: {stderr}"));
    }

    Ok(())
}
