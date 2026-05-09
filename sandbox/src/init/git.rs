//! Git configuration and repository cloning.
//!
//! Provider-agnostic: uses `GIT_TOKEN` (set by credentials.rs based on
//! `GIT_PROVIDER`) and the full clone URL from `GIT_CLONE_URL`.

use std::path::Path;

use tokio::process::Command;
use tracing::{info, warn};

use crate::config::Config;

/// Configure git authentication.
///
/// For GitHub: uses `gh auth setup-git` + resolves user info via `gh api user`.
/// For other providers: configures token-based HTTPS auth via git credential helper.
pub async fn configure_git(config: &Config) -> Result<(), String> {
    let is_github = config.git_provider == "github";

    // GitHub-specific: use gh CLI for credential management
    if is_github {
        if std::env::var("GIT_TOKEN").is_err() {
            info!("No git token set — skipping git configuration");
            return Ok(());
        }

        // Configure gh CLI to use git credential helper.
        // hosts.yml is already written by credentials.rs; this wires git to use it.
        if let Err(e) = run_command("gh", &["auth", "setup-git"]).await {
            warn!(error = %e, "gh auth setup-git failed, continuing with manual config");
        }

        // Prepend the broker-backed helper ahead of `gh auth git-credential` so
        // git/gh push always pull a freshly-rotated token instead of the
        // up-to-15-min-stale value sitting in hosts.yml. The helper script
        // itself falls back to gh's helper if the broker is unreachable, so
        // worst case we degrade to today's behaviour.
        wire_broker_git_helper().await;

        // SSH to HTTPS conversion (GitHub only)
        let _ = run_command(
            "git",
            &[
                "config",
                "--global",
                "url.https://github.com/.insteadOf",
                "git@github.com:",
            ],
        )
        .await;
    } else if std::env::var("GIT_TOKEN").is_err() {
        info!(provider = %config.git_provider, "No git token set — skipping git configuration");
        return Ok(());
    }

    // Resolve git user info
    let user_name = std::env::var("GIT_USER_NAME").ok();
    let user_email = std::env::var("GIT_USER_EMAIL").ok();

    // Try to resolve user info from GitHub API (GitHub only)
    if is_github && (user_name.is_none() || user_email.is_none()) {
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

    // For non-GitHub providers, set a fallback user if nothing was configured
    if !is_github && user_name.is_none() && user_email.is_none() {
        let _ = run_command(
            "git",
            &["config", "--global", "user.name", "Moatless Agent"],
        )
        .await;
        let _ = run_command(
            "git",
            &["config", "--global", "user.email", "agent@moatless.ai"],
        )
        .await;
    }

    info!(provider = %config.git_provider, "Git authentication configured");
    Ok(())
}

/// Clone a repository using the clone URL directly.
///
/// For GitHub repos, tries `gh repo clone` first, then falls back to `git clone`.
/// For other providers, uses `git clone` with token-authenticated HTTPS URL.
pub async fn clone_repository(config: &Config) -> Result<(), String> {
    let Some(ref clone_url) = config.git_clone_url else {
        return Ok(());
    };

    // Skip if already cloned
    if Path::new(&config.workspace_dir).join(".git").exists() {
        info!("Repository already cloned, skipping");
        return Ok(());
    }

    let is_github = config.git_provider == "github";

    // Normalize empty branch to None
    let branch = config.git_branch.as_deref().filter(|b| !b.is_empty());

    info!(
        url = %clone_url,
        provider = %config.git_provider,
        branch = ?branch,
        "Cloning repository"
    );

    // GitHub: try `gh repo clone` first (handles auth automatically)
    if is_github {
        let repo_path = extract_github_repo_path(clone_url);
        let mut args = vec!["repo", "clone", &repo_path, &config.workspace_dir, "--"];
        args.push("--depth");
        args.push("1");

        if let Some(b) = branch {
            args.push("--branch");
            args.push(b);
        }

        match run_command("gh", &args).await {
            Ok(()) => {
                info!("Repository cloned successfully via gh CLI");
                return Ok(());
            }
            Err(e) => {
                warn!(error = %e, "gh repo clone failed, falling back to git clone");
                clean_workspace_dir(&config.workspace_dir).await;
            }
        }
    }

    // Try git clone with token auth (works for all providers)
    let git_token = std::env::var("GIT_TOKEN").ok();

    if let Some(token) = git_token {
        let auth_url = inject_token_into_url(clone_url, &token);
        let mut git_args = vec!["clone", "--depth", "1"];
        if let Some(b) = branch {
            git_args.push("--branch");
            git_args.push(b);
        }
        git_args.push(&auth_url);
        git_args.push(&config.workspace_dir);
        match run_command("git", &git_args).await {
            Ok(()) => {
                info!("Repository cloned successfully with token auth");
                return Ok(());
            }
            Err(e) => {
                warn!(error = %e, "git clone with token failed, trying without auth");
                clean_workspace_dir(&config.workspace_dir).await;
            }
        }
    }

    // Last resort: plain git clone (works for public repos)
    let mut git_args = vec!["clone", "--depth", "1"];
    if let Some(b) = branch {
        git_args.push("--branch");
        git_args.push(b);
    }
    git_args.push(clone_url);
    git_args.push(&config.workspace_dir);
    match run_command("git", &git_args).await {
        Ok(()) => {
            info!("Repository cloned successfully (plain git clone)");
            Ok(())
        }
        Err(e) => Err(format!("All clone methods failed. Last error: {e}")),
    }
}

/// Remove contents of the workspace directory without removing the directory itself.
///
/// This is needed between clone fallback attempts: a failed `git clone` may leave
/// partial data (`.git/`, etc.) that causes the next attempt to fail with
/// "directory already exists and is not an empty directory". The directory itself
/// is kept because it may be a mount point (e.g. a Kubernetes PVC).
async fn clean_workspace_dir(dir: &str) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let ft = match entry.file_type().await {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        let result = if ft.is_dir() {
            tokio::fs::remove_dir_all(&path).await
        } else {
            tokio::fs::remove_file(&path).await
        };
        if let Err(e) = result {
            warn!(path = %path.display(), error = %e, "Failed to clean workspace entry");
        }
    }
    info!(dir = %dir, "Cleaned workspace directory for next clone attempt");
}

/// Extract "owner/repo" from a GitHub URL.
///
/// Handles:
/// - `https://github.com/owner/repo.git`
/// - `git@github.com:owner/repo.git`
fn extract_github_repo_path(url: &str) -> String {
    url.replace("https://github.com/", "")
        .replace("git@github.com:", "")
        .trim_end_matches(".git")
        .to_string()
}

/// Inject a token into an HTTPS clone URL.
///
/// Converts `https://host/path.git` → `https://x-access-token:{token}@host/path.git`.
/// If the URL already has credentials embedded, replaces them.
pub(crate) fn inject_token_into_url(url: &str, token: &str) -> String {
    if let Some(rest) = url.strip_prefix("https://") {
        // Strip any existing credentials (user:pass@)
        let host_and_path = if let Some(at_pos) = rest.find('@') {
            &rest[at_pos + 1..]
        } else {
            rest
        };
        format!("https://x-access-token:{token}@{host_and_path}")
    } else {
        // Not an HTTPS URL, return as-is (ssh, etc.)
        url.to_string()
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

/// Path to the credential-broker-backed git helper script. Shipped in the
/// sandbox image alongside the sandbox binary.
const BROKER_HELPER_PATH: &str = "/usr/local/bin/moatless-git-cred";

/// Configure git to use the broker-backed credential helper as the primary
/// source for `https://github.com` credentials, with `gh auth git-credential`
/// retained as a fallback (added inside the helper script itself).
///
/// `--replace-all` wipes any previous helper entries (including the one
/// installed by `gh auth setup-git`) so the broker helper is the only one
/// configured. The helper script handles fallback to `gh auth git-credential`
/// internally, which keeps the existing hosts.yml-backed path working when the
/// broker is unreachable.
async fn wire_broker_git_helper() {
    if !std::path::Path::new(BROKER_HELPER_PATH).exists() {
        info!(
            path = BROKER_HELPER_PATH,
            "Broker credential helper script not present — leaving gh auth git-credential as the credential source"
        );
        return;
    }

    let helper_value = format!("!{BROKER_HELPER_PATH}");
    if let Err(e) = run_command(
        "git",
        &[
            "config",
            "--global",
            "--replace-all",
            "credential.https://github.com.helper",
            &helper_value,
        ],
    )
    .await
    {
        warn!(error = %e, "Failed to wire moatless-git-cred — git push may use stale hosts.yml token");
        return;
    }

    info!(
        helper = BROKER_HELPER_PATH,
        "Wired broker-backed git credential helper for github.com"
    );
}

/// Run a command and return an error if it fails.
pub(crate) async fn run_command(program: &str, args: &[&str]) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_github_repo_path_https() {
        assert_eq!(
            extract_github_repo_path("https://github.com/owner/repo.git"),
            "owner/repo"
        );
    }

    #[test]
    fn extract_github_repo_path_ssh() {
        assert_eq!(
            extract_github_repo_path("git@github.com:owner/repo.git"),
            "owner/repo"
        );
    }

    #[test]
    fn inject_token_basic() {
        assert_eq!(
            inject_token_into_url("https://github.com/owner/repo.git", "tok123"),
            "https://x-access-token:tok123@github.com/owner/repo.git"
        );
    }

    #[test]
    fn inject_token_gitness() {
        assert_eq!(
            inject_token_into_url("https://gitness.example.com/org/repo.git", "eyJ"),
            "https://x-access-token:eyJ@gitness.example.com/org/repo.git"
        );
    }

    #[test]
    fn inject_token_replaces_existing_credentials() {
        assert_eq!(
            inject_token_into_url("https://old:cred@github.com/owner/repo.git", "new_token"),
            "https://x-access-token:new_token@github.com/owner/repo.git"
        );
    }

    #[test]
    fn inject_token_ssh_passthrough() {
        let ssh_url = "git@github.com:owner/repo.git";
        assert_eq!(inject_token_into_url(ssh_url, "tok"), ssh_url);
    }
}
