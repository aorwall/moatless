//! Git configuration and repository cloning.
//!
//! Provider-agnostic: the git identity/credential-helper wiring follows the
//! pod's own `GIT_PROVIDER`, while each of the workspace's checkouts
//! ([`Config::repos`]) clones with the token for its own provider.

use std::path::Path;

use moatless_api_schemas::sandbox::RepoCheckout;
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::Config;

/// Configure git authentication.
///
/// For GitHub: uses `gh auth setup-git` + resolves user info via `gh api user`.
/// For other providers: configures token-based HTTPS auth via git credential helper.
pub async fn configure_git(config: &Config) -> Result<(), String> {
    let is_ssh = config.git_clone_url.as_deref().is_some_and(is_ssh_url);
    let is_github = config.git_provider == "github";

    if is_ssh {
        // SSH remotes authenticate with the private key(s) materialized into
        // ~/.ssh by credentials.rs — nothing to wire here. Skip the
        // token/gh/insteadOf setup (the GitHub `insteadOf` rewrite would even
        // rewrite our SSH URL back to HTTPS) and fall through to git identity.
        info!("SSH remote detected — using SSH key auth from ~/.ssh");
    } else if is_github {
        // GitHub-specific: use gh CLI for credential management
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

    // Try to resolve user info from GitHub API (GitHub HTTPS only — an SSH
    // remote has no `gh` token to query the API with).
    if is_github && !is_ssh && (user_name.is_none() || user_email.is_none()) {
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

    // For non-GitHub providers (and SSH remotes, which have no `gh` identity
    // to resolve), set a fallback user if nothing was configured.
    if (!is_github || is_ssh) && user_name.is_none() && user_email.is_none() {
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

/// The outcome of cloning the workspace's repositories.
pub struct CloneSummary {
    /// Mount names freshly cloned in this pass; empty when every repo was
    /// already on disk.
    pub cloned: Vec<String>,
    /// Whether the *primary* repo was freshly cloned rather than found in place.
    /// When true the pre-restart working tree is gone, which a resuming agent is
    /// told about.
    pub primary_recloned: bool,
}

/// Clone every repository the task's workspace places in this sandbox.
///
/// Each checkout lands in its own `{workspace_dir}/{mount_name}` directory and
/// authenticates with its own provider's token, so a workspace can mix providers.
/// A repo whose directory already holds a `.git` is left alone, which makes this
/// safe to run on a pod whose volume survived a restart and lets a repo added to
/// the workspace afterwards be picked up on the next start.
///
/// Every declared repo is required: a workspace names the repos the task's work
/// spans, so silently starting the agent without one produces work against the
/// wrong tree. The first failure aborts.
pub async fn clone_repositories(config: &Config) -> Result<CloneSummary, String> {
    let mut summary = CloneSummary {
        cloned: Vec::new(),
        primary_recloned: false,
    };
    for repo in &config.repos {
        let dest = repo.dir(&config.workspace_dir);
        // Skip if already cloned. For the primary, the workspace-root check also
        // covers sandboxes created before SSH clones moved to `repo_dir` — their
        // checkout lives directly in `workspace_dir` on a persistent volume.
        if Path::new(&dest).join(".git").exists()
            || (repo.is_primary && Path::new(&config.workspace_dir).join(".git").exists())
        {
            info!(mount = %repo.mount_name, "Repository already cloned, skipping");
            continue;
        }
        clone_checkout(repo, &dest, &config.git_provider)
            .await
            .map_err(|e| format!("{} ({}): {e}", repo.mount_name, repo.clone_url))?;
        summary.cloned.push(repo.mount_name.clone());
        summary.primary_recloned |= repo.is_primary;
    }
    Ok(summary)
}

/// Clone one repository into `dest`.
///
/// For GitHub repos, tries `gh repo clone` first, then falls back to `git clone`.
/// For other providers, uses `git clone` with token-authenticated HTTPS URL.
async fn clone_checkout(
    repo: &RepoCheckout,
    dest: &str,
    pod_provider: &str,
) -> Result<(), String> {
    let clone_url = &repo.clone_url;
    let is_ssh = is_ssh_url(clone_url);
    let is_github = repo.provider == "github";

    // Normalize empty branch to None
    let branch = repo.branch.as_deref().filter(|b| !b.is_empty());

    info!(
        url = %clone_url,
        provider = %repo.provider,
        mount = %repo.mount_name,
        ssh = is_ssh,
        branch = ?branch,
        "Cloning repository"
    );

    // SSH remote: clone the URL directly. Authentication and host-key checking
    // come from the identities/config written to ~/.ssh by credentials.rs — no
    // token injection or `gh` involved (checked first so an SSH URL to a host
    // with a configured provider still takes the SSH path). Clones into
    // `repo_dir` like the other providers — `workspace_dir` is no longer empty
    // at clone time (e.g. the persisted user token lives under it), and git
    // refuses to clone into a non-empty directory.
    if is_ssh {
        let mut git_args = vec!["clone", "--depth", "1"];
        if let Some(b) = branch {
            git_args.push("--branch");
            git_args.push(b);
        }
        git_args.push(clone_url);
        git_args.push(dest);
        return match run_command("git", &git_args).await {
            Ok(()) => {
                info!("Repository cloned successfully over SSH");
                Ok(())
            }
            Err(e) => Err(format!("SSH clone failed: {e}")),
        };
    }

    // GitHub: try `gh repo clone` first (handles auth automatically)
    if is_github {
        let repo_path = extract_github_repo_path(clone_url);
        let mut args = vec!["repo", "clone", &repo_path, dest, "--"];
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
                clean_workspace_dir(dest).await;
            }
        }
    }

    // Try git clone with the token for *this* repo's provider (works for all
    // providers), so a workspace mixing e.g. a GitHub and a GitLab repo
    // authenticates each with the right credential.
    if let Some(token) = provider_git_token(&repo.provider, pod_provider) {
        let auth_url = inject_token_into_url(clone_url, &token);
        let mut git_args = vec!["clone", "--depth", "1"];
        if let Some(b) = branch {
            git_args.push("--branch");
            git_args.push(b);
        }
        git_args.push(&auth_url);
        git_args.push(dest);
        match run_command("git", &git_args).await {
            Ok(()) => {
                info!("Repository cloned successfully with token auth");
                return Ok(());
            }
            Err(e) => {
                warn!(error = %e, "git clone with token failed, trying without auth");
                clean_workspace_dir(dest).await;
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
    git_args.push(dest);
    match run_command("git", &git_args).await {
        Ok(()) => {
            info!("Repository cloned successfully (plain git clone)");
            Ok(())
        }
        Err(e) => Err(format!("All clone methods failed. Last error: {e}")),
    }
}

/// Resolve the clone token for one provider from the credentials the sandbox
/// fetched at init.
///
/// `credentials.rs` sets each provider's token in the process env, except
/// GitHub's — `GITHUB_TOKEN`/`GH_TOKEN` are deliberately kept out so `gh` reads
/// the refresher-owned `hosts.yml` instead. `GIT_TOKEN` therefore backs GitHub
/// up, but only when this checkout uses the *pod's* provider: `GIT_TOKEN` holds
/// that provider's token, and injecting it into another host's clone URL would
/// hand the credential to a third party. When it doesn't match, the `gh repo
/// clone` attempt (GitHub) or an unauthenticated clone is the remaining path.
fn provider_git_token(provider: &str, pod_provider: &str) -> Option<String> {
    resolve_provider_git_token(provider, pod_provider, |key| {
        std::env::var(key).ok().filter(|value| !value.is_empty())
    })
}

/// The env-free half of [`provider_git_token`], so the "never lend the pod token
/// to a foreign host" rule is testable without touching the process env.
fn resolve_provider_git_token(
    provider: &str,
    pod_provider: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let key = match provider {
        "gitlab" => "GITLAB_TOKEN",
        "gitness" => "GITNESS_TOKEN",
        _ => "GITHUB_TOKEN",
    };
    lookup(key).or_else(|| {
        if provider == pod_provider {
            lookup("GIT_TOKEN")
        } else {
            None
        }
    })
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

/// Whether a clone URL uses SSH transport.
///
/// Matches both `ssh://…` URLs and scp-like `[user@]host:path` syntax
/// (e.g. `git@github.com:owner/repo.git`), while excluding HTTP(S) URLs.
pub(crate) fn is_ssh_url(url: &str) -> bool {
    if url.starts_with("ssh://") || url.starts_with("git+ssh://") {
        return true;
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return false;
    }
    // scp-like syntax: user@host:path — an '@' before a ':', and the ':' is not
    // part of a URL scheme.
    match (url.find('@'), url.find(':')) {
        (Some(at), Some(colon)) => colon > at,
        _ => false,
    }
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

    use crate::config::test_support::minimal_config;

    fn checkout(mount: &str, provider: &str, is_primary: bool) -> RepoCheckout {
        RepoCheckout {
            clone_url: format!("https://{provider}.example/acme/{mount}.git"),
            branch: None,
            provider: provider.to_string(),
            mount_name: mount.to_string(),
            is_primary,
        }
    }

    fn run_git_sync(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("git command runs");
        assert!(status.success(), "git {args:?} failed");
    }

    fn init_local_source_repo(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        run_git_sync(path, &["init", "--initial-branch", "main"]);
        std::fs::write(path.join("README.md"), "secondary\n").unwrap();
        run_git_sync(path, &["add", "README.md"]);
        run_git_sync(
            path,
            &[
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test User",
                "commit",
                "-m",
                "init",
            ],
        );
    }

    /// A repo whose directory already holds a `.git` is left alone, so a restart
    /// onto a persistent volume neither re-clones nor reports a reclone.
    #[tokio::test]
    async fn already_cloned_repos_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().display().to_string();
        let mut config = minimal_config();
        config.workspace_dir = workspace.clone();
        config.repos = vec![checkout("api", "github", true)];
        config.repo_dir = format!("{workspace}/api");
        std::fs::create_dir_all(format!("{workspace}/api/.git")).unwrap();

        let summary = clone_repositories(&config).await.unwrap();
        assert!(summary.cloned.is_empty());
        assert!(!summary.primary_recloned);
    }

    /// A full S3 restore can contain an older workspace shape: the primary repo
    /// is present, but a repo added to the workspace later is absent. The init
    /// flow still runs this clone pass after restore, so only the missing sibling
    /// should be cloned and the primary should not count as recloned.
    #[tokio::test]
    async fn restored_primary_does_not_prevent_new_sibling_clone() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let source = tmp.path().join("source-secondary");
        init_local_source_repo(&source);

        let workspace_str = workspace.display().to_string();
        let primary = workspace.join("primary");
        std::fs::create_dir_all(primary.join(".git")).unwrap();

        let mut config = minimal_config();
        config.workspace_dir = workspace_str.clone();
        config.repo_dir = primary.display().to_string();
        config.git_provider = "local".to_string();
        config.repos = vec![
            RepoCheckout {
                clone_url: "https://example.test/acme/primary.git".to_string(),
                branch: None,
                provider: "local".to_string(),
                mount_name: "primary".to_string(),
                is_primary: true,
            },
            RepoCheckout {
                clone_url: source.display().to_string(),
                branch: None,
                provider: "local".to_string(),
                mount_name: "secondary".to_string(),
                is_primary: false,
            },
        ];

        let summary = clone_repositories(&config).await.unwrap();

        assert_eq!(summary.cloned, vec!["secondary"]);
        assert!(!summary.primary_recloned);
        assert!(workspace.join("secondary").join(".git").is_dir());
        assert!(workspace.join("secondary").join("README.md").is_file());
    }

    /// Each checkout authenticates with its own provider's token, and the pod's
    /// `GIT_TOKEN` only backs up a checkout on the pod's own provider — injecting
    /// it into another host's clone URL would hand the credential to a third
    /// party.
    #[test]
    fn each_checkout_uses_its_own_providers_token() {
        let env = |key: &str| match key {
            "GITLAB_TOKEN" => Some("glpat".to_string()),
            "GIT_TOKEN" => Some("pod-github-token".to_string()),
            _ => None,
        };
        assert_eq!(
            resolve_provider_git_token("gitlab", "github", env),
            Some("glpat".to_string())
        );
        // GitHub has no env token (kept out for `gh`), so it uses the pod's.
        assert_eq!(
            resolve_provider_git_token("github", "github", env),
            Some("pod-github-token".to_string())
        );
        // A foreign provider with no token of its own gets nothing.
        assert_eq!(resolve_provider_git_token("gitness", "github", env), None);
    }

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

    /// The backend now hands the sandbox a credential-free clone URL (userinfo
    /// stripped in `pod_builder::redact_url_userinfo`). Cloning still authenticates
    /// by injecting the provider token from the credential channel (`GIT_TOKEN`).
    /// Verify both the GitHub and GitLab token flows produce an authenticated URL
    /// from a redacted input.
    #[test]
    fn redacted_url_authenticates_via_injected_token() {
        // GitHub flow.
        assert_eq!(
            inject_token_into_url("https://github.com/owner/repo.git", "ghs_tok"),
            "https://x-access-token:ghs_tok@github.com/owner/repo.git"
        );
        // GitLab flow.
        assert_eq!(
            inject_token_into_url("https://gitlab.com/group/repo.git", "glpat_tok"),
            "https://x-access-token:glpat_tok@gitlab.com/group/repo.git"
        );
    }

    #[test]
    fn inject_token_ssh_passthrough() {
        let ssh_url = "git@github.com:owner/repo.git";
        assert_eq!(inject_token_into_url(ssh_url, "tok"), ssh_url);
    }

    #[test]
    fn detects_ssh_urls() {
        assert!(is_ssh_url("git@github.com:owner/repo.git"));
        assert!(is_ssh_url("git@gitlab.example.com:group/sub/repo.git"));
        assert!(is_ssh_url("ssh://git@example.com/owner/repo.git"));
        assert!(is_ssh_url("deploy@10.0.0.1:/srv/git/repo.git"));
    }

    #[test]
    fn rejects_non_ssh_urls() {
        assert!(!is_ssh_url("https://github.com/owner/repo.git"));
        assert!(!is_ssh_url("http://gitlab.example.com/owner/repo.git"));
        assert!(!is_ssh_url("https://user:tok@github.com/owner/repo.git"));
    }
}
