//! Git configuration and repository cloning.
//!
//! Provider-agnostic: the git identity/credential-helper wiring follows the
//! pod's own `GIT_PROVIDER`, while each of the workspace's checkouts
//! ([`Config::repos`]) clones with the token for its own provider.

use std::path::Path;
use std::time::{Duration, Instant};

use moatless_types::workspace::RepoCheckout;
use tokio::net::TcpStream;
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
pub async fn clone_repositories(config: &Config) -> Result<CloneSummary, CloneFailure> {
    let deadline = Instant::now() + CLONE_RETRY_BUDGET;
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
        clone_with_retry(repo, &dest, &config.git_provider, deadline).await?;
        summary.cloned.push(repo.mount_name.clone());
        summary.primary_recloned |= repo.is_primary;
    }
    Ok(summary)
}

/// A failed clone, and whether another attempt could get past it.
#[derive(Debug)]
pub struct CloneFailure {
    message: String,
    /// Another attempt could get past this: the pod could not reach the host, or
    /// the service answered for its own outage. The attempt then says nothing
    /// about whether the repo exists or its credentials are accepted, so init
    /// retries rather than reporting a permanent config error.
    pub retryable: bool,
}

impl std::fmt::Display for CloneFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Wall-clock ceiling on clone retries, shared by every repo in one pass.
///
/// The pod's startup probe kills the container after
/// `K8S_POD_STARTUP_TIMEOUT_SECS / 5 + 6` failed `/health` checks at 5s spacing
/// — 150s at the default — and the credential fetch ahead of this can spend 90s
/// of that. A loop outlasting what is left is SIGKILLed mid-backoff and never
/// reports why it was failing; the container restart is the outer retry.
const CLONE_RETRY_BUDGET: Duration = Duration::from_secs(45);

/// Longest single backoff between clone attempts.
const MAX_CLONE_RETRY_DELAY: Duration = Duration::from_secs(8);

/// Ceiling on one reachability probe, so a black-holed host cannot spend the
/// whole retry budget inside a single connect.
const HOST_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Clone one repo, retrying for as long as its host is unreachable and
/// `deadline` allows.
///
/// A reachable host means the failure is about the repo or its credentials, so
/// this returns at once — retrying a 404 only burns the startup budget.
async fn clone_with_retry(
    repo: &RepoCheckout,
    dest: &str,
    pod_provider: &str,
    deadline: Instant,
) -> Result<(), CloneFailure> {
    let mut delay = Duration::from_secs(2);
    loop {
        let error = match clone_checkout(repo, dest, pod_provider).await {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };
        let message = format!("{} ({}): {error}", repo.mount_name, repo.clone_url);

        if !failure_is_retryable(repo, pod_provider).await {
            return Err(CloneFailure {
                message,
                retryable: false,
            });
        }

        // Stop once the next backoff would spend the rest of the budget: sleeping
        // to the deadline only to start an attempt with no time left is how the
        // startup probe gets to SIGKILL this mid-clone, taking the error with it.
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining <= delay {
            return Err(CloneFailure {
                message,
                retryable: true,
            });
        }

        warn!(
            mount = %repo.mount_name,
            error = %error,
            retry_in_secs = delay.as_secs(),
            "Clone failed on a condition another attempt could get past — retrying"
        );
        // git refuses to clone into a non-empty directory, so whatever the
        // failed attempt left behind has to go before the next one.
        clean_workspace_dir(dest).await;
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(MAX_CLONE_RETRY_DELAY);
    }
}

/// Whether a failed clone is worth another attempt.
///
/// Two measured signals, never git's stderr: the status the service answers for
/// this repo, and whether the host answers a TCP connect at all.
async fn failure_is_retryable(repo: &RepoCheckout, pod_provider: &str) -> bool {
    // A 5xx is the service having its own outage, which a reachable host would
    // otherwise be read as a verdict on the repo. This can only ever promote a
    // failure to retryable: a 4xx falls through to the reachability check rather
    // than deciding by itself, so a provider whose endpoint answers differently
    // cannot turn a working repo into a permanent config error.
    if let Some(status) = repo_advertisement_status(repo, pod_provider).await {
        if status.is_server_error() {
            return true;
        }
    }
    !host_reachable(&repo.clone_url).await
}

/// The status the repo's git smart-HTTP endpoint answers with, or `None` when
/// there is nothing to ask — an SSH remote, or a request that never completed.
async fn repo_advertisement_status(
    repo: &RepoCheckout,
    pod_provider: &str,
) -> Option<reqwest::StatusCode> {
    if is_ssh_url(&repo.clone_url) {
        return None;
    }
    let base = repo
        .clone_url
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let client = reqwest::Client::builder()
        .timeout(HOST_PROBE_TIMEOUT)
        .build()
        .ok()?;
    let mut request = client.get(format!("{base}/info/refs?service=git-upload-pack"));
    if let Some(token) = provider_git_token(&repo.provider, pod_provider) {
        request = request.basic_auth("x-access-token", Some(token));
    }
    request.send().await.ok().map(|response| response.status())
}

/// Whether the pod can currently reach the host `clone_url` points at.
///
/// Asks the resolver and the TCP stack directly instead of reading git's stderr:
/// git exits 128 for every fatal error, and a host that does not resolve or
/// refuses a connection is why the clone failed whatever it printed.
///
/// A URL with no host to probe answers `true` — nothing was learned about the
/// pod's network, and the clone failure has to stand on its own.
async fn host_reachable(clone_url: &str) -> bool {
    let Some((host, port)) = clone_host_port(clone_url) else {
        return true;
    };
    let probe = async {
        // Every resolved address counts, not just the first: a pod with no IPv6
        // route reaches a dual-stack host only on the A record it is handed
        // second, and calling that host unreachable would send a genuine 404
        // into a crash loop. `HOST_PROBE_TIMEOUT` bounds the walk as a whole.
        for addr in tokio::net::lookup_host((host.as_str(), port)).await.ok()? {
            if TcpStream::connect(addr).await.is_ok() {
                return Some(());
            }
        }
        None
    };
    matches!(
        tokio::time::timeout(HOST_PROBE_TIMEOUT, probe).await,
        Ok(Some(()))
    )
}

/// Host and port to probe for a clone URL: the scheme's default port unless the
/// URL names its own.
fn clone_host_port(clone_url: &str) -> Option<(String, u16)> {
    let (rest, default_port) = if let Some(rest) = clone_url.strip_prefix("https://") {
        (rest, 443)
    } else if let Some(rest) = clone_url.strip_prefix("http://") {
        (rest, 80)
    } else if let Some(rest) = clone_url.strip_prefix("ssh://") {
        (rest, 22)
    } else if let Some(rest) = clone_url.strip_prefix("git+ssh://") {
        (rest, 22)
    } else if is_ssh_url(clone_url) {
        // scp-like `user@host:path`, which carries no scheme.
        (clone_url, 22)
    } else {
        // A local path or anything else with no host of its own.
        return None;
    };

    // Cut the path first so a scp-like URL's `:` path separator cannot be read
    // as a port, then drop any userinfo an injected token left in front.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if authority.is_empty() {
        return None;
    }

    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => {
            Some((host.to_string(), port.parse().unwrap_or(default_port)))
        }
        _ => Some((authority.to_string(), default_port)),
    }
}

/// Clone one repository into `dest`.
///
/// For GitHub repos, tries `gh repo clone` first, then falls back to `git clone`.
/// For other providers, uses `git clone` with token-authenticated HTTPS URL.
async fn clone_checkout(repo: &RepoCheckout, dest: &str, pod_provider: &str) -> Result<(), String> {
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

    /// A port nothing listens on. Well below the ephemeral range, so a
    /// concurrently-running test cannot be handed it and make the connect succeed.
    const DEAD_PORT: u16 = 1;

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

    /// The probe answers from the resolver and the TCP stack, so both verdicts
    /// are exercised against a real local socket rather than the public network:
    /// a listener that exists, and a port nothing is bound to.
    #[tokio::test]
    async fn probe_reports_reachable_and_unreachable_hosts() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let live = listener.local_addr().unwrap().port();
        assert!(host_reachable(&format!("https://127.0.0.1:{live}/acme/api.git")).await);

        // Nothing is bound to `DEAD_PORT`, so the connect is refused outright.
        assert!(!host_reachable(&format!("https://127.0.0.1:{DEAD_PORT}/acme/api.git")).await);
    }

    /// A host resolving to several addresses is reachable when *any* of them
    /// connects. On a dual-stack runner `localhost` yields `::1` before
    /// `127.0.0.1`, and a listener bound only to `127.0.0.1` is reached solely by
    /// walking past the first address.
    #[tokio::test]
    async fn probe_walks_every_resolved_address() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(host_reachable(&format!("https://localhost:{port}/acme/api.git")).await);
    }

    /// The retry loop must not sleep the budget away and then start an attempt it
    /// cannot afford — the startup probe SIGKILLs the container mid-clone and the
    /// error never reaches the termination log.
    #[tokio::test]
    async fn retry_stops_before_an_attempt_it_cannot_afford() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = RepoCheckout {
            clone_url: format!("https://127.0.0.1:{DEAD_PORT}/acme/api.git"),
            branch: None,
            provider: "local".to_string(),
            mount_name: "api".to_string(),
            is_primary: true,
        };

        // A budget just over the first backoff: one retry is affordable, a second
        // is not. The production budget would spend 45s proving the same thing.
        let budget = Duration::from_secs(3);
        let started = Instant::now();
        let err = clone_with_retry(
            &repo,
            &tmp.path().join("api").display().to_string(),
            "local",
            started + budget,
        )
        .await
        .expect_err("a clone against a dead port must fail");

        assert!(err.retryable);
        // Returned within the budget rather than overrunning it on a last attempt.
        assert!(
            started.elapsed() <= budget,
            "retry overran its budget: {:?}",
            started.elapsed()
        );
    }

    /// Answer every request with `status`, so a clone failure can be classified
    /// against a service whose behaviour the test controls.
    async fn serve_status(status: &'static str) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut discard = [0u8; 1024];
                let _ = socket.read(&mut discard).await;
                let response =
                    format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        port
    }

    fn repo_at(port: u16) -> RepoCheckout {
        RepoCheckout {
            clone_url: format!("http://127.0.0.1:{port}/acme/api.git"),
            branch: None,
            provider: "local".to_string(),
            mount_name: "api".to_string(),
            is_primary: true,
        }
    }

    /// A service answering 5xx is reporting its own outage, not passing a verdict
    /// on the repo. Reachability alone would call that permanent and kill the
    /// sandbox for the length of a provider incident.
    #[tokio::test]
    async fn service_side_5xx_stays_retryable() {
        let port = serve_status("503 Service Unavailable").await;
        assert!(failure_is_retryable(&repo_at(port), "local").await);
    }

    /// A 4xx never decides on its own — it falls through to reachability — but a
    /// host that answers leaves the failure the repo's own, and permanent.
    #[tokio::test]
    async fn client_error_from_a_reachable_host_is_not_retryable() {
        let port = serve_status("404 Not Found").await;
        assert!(!failure_is_retryable(&repo_at(port), "local").await);
    }

    /// A URL with no host to probe teaches nothing about the pod's network, so
    /// the clone failure has to stand on its own and stay permanent.
    #[tokio::test]
    async fn hostless_url_counts_as_reachable() {
        assert!(host_reachable("/srv/git/local-repo.git").await);
    }

    /// The probe targets the port the clone would actually use, and an injected
    /// token in the URL's userinfo must not be mistaken for the host.
    #[test]
    fn clone_urls_resolve_to_the_host_and_port_the_clone_uses() {
        let port = |url: &str| clone_host_port(url);
        assert_eq!(
            port("https://github.com/owner/repo.git"),
            Some(("github.com".into(), 443))
        );
        assert_eq!(
            port("http://gitlab.internal/owner/repo.git"),
            Some(("gitlab.internal".into(), 80))
        );
        assert_eq!(
            port("https://gitness.example.com:8443/org/repo.git"),
            Some(("gitness.example.com".into(), 8443))
        );
        // scp-like: the ':' separates a path, not a port.
        assert_eq!(
            port("git@github.com:owner/repo.git"),
            Some(("github.com".into(), 22))
        );
        assert_eq!(
            port("ssh://git@example.com:2222/owner/repo.git"),
            Some(("example.com".into(), 2222))
        );
        // An injected token must not be read as the host.
        assert_eq!(
            port("https://x-access-token:ghs_tok@github.com/owner/repo.git"),
            Some(("github.com".into(), 443))
        );
        // Nothing to probe.
        assert_eq!(port("/srv/git/local-repo.git"), None);
    }

    /// The incident shape: the host is unreachable and the retry budget is gone,
    /// so the failure is reported as environmental (exit 1, the restart retries)
    /// rather than as a permanent config error.
    #[tokio::test]
    async fn exhausted_budget_against_an_unreachable_host_stays_retryable() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = RepoCheckout {
            clone_url: format!("https://127.0.0.1:{DEAD_PORT}/acme/api.git"),
            branch: None,
            provider: "local".to_string(),
            mount_name: "api".to_string(),
            is_primary: true,
        };

        let err = clone_with_retry(
            &repo,
            &tmp.path().join("api").display().to_string(),
            "local",
            Instant::now(),
        )
        .await
        .expect_err("a clone against a dead port must fail");

        assert!(err.retryable);
        assert!(err.to_string().contains("api"), "message was: {err}");
    }

    /// A repo the pod can reach is judged on the clone itself: no amount of
    /// retrying makes a missing repository appear, so it must not be reported as
    /// a network fault.
    #[tokio::test]
    async fn reachable_host_reports_the_failure_as_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = RepoCheckout {
            // No host to probe, so the probe cannot excuse the failure.
            clone_url: tmp.path().join("does-not-exist").display().to_string(),
            branch: None,
            provider: "local".to_string(),
            mount_name: "api".to_string(),
            is_primary: true,
        };

        let err = clone_with_retry(
            &repo,
            &tmp.path().join("api").display().to_string(),
            "local",
            Instant::now() + Duration::from_mins(1),
        )
        .await
        .expect_err("cloning a path that does not exist must fail");

        assert!(!err.retryable);
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
