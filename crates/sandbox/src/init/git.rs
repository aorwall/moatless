//! Git configuration and repository cloning.
//!
//! Provider-agnostic: the git identity/credential-helper wiring follows the
//! pod's own `GIT_PROVIDER`, while each of the workspace's checkouts
//! ([`Config::repos`]) clones with the token for its own provider.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use moatless_types::http::HttpTimeouts;
use moatless_workspace_types::workspace::{PROXIED_PROVIDER, RepoCheckout};
use tokio::net::TcpStream;
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::Config;

/// Configure git authentication: the broker-backed credential helper (plus
/// `gh api user` for identity on GitHub), a token-based helper on other
/// providers.
pub async fn configure_git(config: &Config, credentials: &GitCredentials) -> Result<(), String> {
    let is_ssh = crate::config::primary_checkout(&config.repos).is_some_and(|repo| repo.uses_ssh);
    let is_github = config.git_provider == "github";

    // The broker answers the backend origin with the pod's task token. Wired
    // wherever that origin is known, and not only where a checkout clones from
    // it: `moat repos clone <id>` fetches a repository the Workspace places no
    // checkout for. Ahead of the no-token exits below, which a pod holding only
    // proxied repos would otherwise take before reaching this.
    let proxy_origin = config.backend_api_origin();
    if let Some(origin) = &proxy_origin {
        wire_broker_git_helper(origin).await;
    }

    // The broker answers github.com with the task's `GITHUB_TOKEN` whatever the
    // pod's own provider (`credentials::broker::get_git_credential`), so a
    // Forgejo or SSH pod reaches `moat repos clone <github_id>` too.
    wire_broker_git_helper("https://github.com").await;

    if is_ssh {
        // SSH remotes authenticate with the private key(s) materialized into
        // ~/.ssh by credentials.rs — nothing to wire here. Skip the
        // token/gh/insteadOf setup (the GitHub `insteadOf` rewrite would even
        // rewrite our SSH URL back to HTTPS) and fall through to git identity.
        info!("SSH remote detected — using SSH key auth from ~/.ssh");
    } else if is_github {
        // Do not run `gh auth setup-git`: `gh` holds only a placeholder token
        // (`credentials::gh_config`), and its helper would answer github.com
        // with that instead of the broker's token.
        if credentials.get(GIT_TOKEN).is_none() {
            info!("No git token set — skipping the git identity");
            return Ok(());
        }

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
    } else if credentials.get(GIT_TOKEN).is_none() && proxy_origin.is_none() {
        info!(provider = %config.git_provider, "No git token set — skipping the provider helper and the git identity");
        return Ok(());
    }

    // Resolve git user info
    let user_name = credentials.get("GIT_USER_NAME");
    let user_email = credentials.get("GIT_USER_EMAIL");

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

    // Set explicit user name/email if the credentials carry them
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
    /// Whether the *primary* repo was freshly cloned rather than found in
    /// place. When true the pre-restart working tree is gone, which a resuming
    /// agent is told about.
    pub primary_recloned: bool,
}

/// Clone every repository the task's workspace names, each into its own dir.
///
/// Uses each repo's own provider token; a dir already holding `.git` is left
/// alone (safe across a restart), and every repo is required, so the first
/// failure aborts.
pub async fn clone_repositories(
    config: &Config,
    credentials: &GitCredentials,
) -> Result<CloneSummary, CloneFailure> {
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
        clone_with_retry(
            repo,
            &dest,
            &config.git_provider,
            credentials,
            config.http.git_probe,
            deadline,
        )
        .await?;
        summary.cloned.push(repo.mount_name.clone());
        summary.primary_recloned |= repo.is_primary;
    }
    Ok(summary)
}

/// A failed clone, and whether another attempt could get past it.
#[derive(Debug)]
pub struct CloneFailure {
    message: String,
    /// Another attempt could get past this: the failure reflects reachability,
    /// not the service's verdict on the repo or its credentials — so init
    /// retries instead of reporting a permanent error.
    pub retryable: bool,
}

impl std::fmt::Display for CloneFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Wall-clock ceiling on clone retries, shared by every repo in one pass.
///
/// The deliberate loser of three startup budgets (credential fetch, workspace
/// restore, this): the probe may SIGKILL it mid-backoff since the restore's
/// report matters more, and the container restart is the outer retry.
const CLONE_RETRY_BUDGET: Duration = Duration::from_secs(45);

/// Longest single backoff between clone attempts.
const MAX_CLONE_RETRY_DELAY: Duration = Duration::from_secs(8);

/// Clone one repo, retrying while its host is unreachable and `deadline`
/// allows.
///
/// A reachable host means the failure is the repo's own, so this returns at
/// once — retrying a 404 only burns the startup budget.
async fn clone_with_retry(
    repo: &RepoCheckout,
    dest: &str,
    pod_provider: &str,
    credentials: &GitCredentials,
    probe: HttpTimeouts,
    deadline: Instant,
) -> Result<(), CloneFailure> {
    let mut delay = Duration::from_secs(2);
    loop {
        let error = match clone_checkout(repo, dest, pod_provider, credentials).await {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };
        let message = format!("{} ({}): {error}", repo.mount_name, repo.clone_url);

        if !failure_is_retryable(repo, pod_provider, credentials, probe).await {
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
/// Two measured signals, never git's stderr: the service's status for this
/// repo, and whether the host answers a TCP connect.
async fn failure_is_retryable(
    repo: &RepoCheckout,
    pod_provider: &str,
    credentials: &GitCredentials,
    probe: HttpTimeouts,
) -> bool {
    // A 5xx is the service having its own outage, which a reachable host would
    // otherwise be read as a verdict on the repo. This can only ever promote a
    // failure to retryable: a 4xx falls through to the reachability check rather
    // than deciding by itself, so a provider whose endpoint answers differently
    // cannot turn a working repo into a permanent config error.
    if let Some(status) = repo_advertisement_status(repo, pod_provider, credentials, probe).await {
        if status.is_server_error() {
            return true;
        }
    }
    !host_reachable(&repo.clone_url, repo.uses_ssh, probe).await
}

/// The status the repo's git smart-HTTP endpoint answers with, or `None` when
/// there is nothing to ask — an SSH remote, or a request that never completed.
async fn repo_advertisement_status(
    repo: &RepoCheckout,
    pod_provider: &str,
    credentials: &GitCredentials,
    probe: HttpTimeouts,
) -> Option<reqwest::StatusCode> {
    if repo.uses_ssh {
        return None;
    }
    let base = repo
        .clone_url
        .trim_end_matches('/')
        .trim_end_matches(".git");
    #[expect(
        clippy::disallowed_methods,
        reason = "sets the timeouts `Config::http` holds"
    )]
    let client = reqwest::Client::builder()
        .connect_timeout(probe.connect)
        .timeout(probe.total)
        .build()
        .ok()?;
    let mut request = client.get(format!("{base}/info/refs?service=git-upload-pack"));
    if let Some(token) = credentials.provider_token(&repo.provider, pod_provider) {
        request = request.basic_auth("x-access-token", Some(token));
    }
    request.send().await.ok().map(|response| response.status())
}

/// Whether the pod can currently reach the host `clone_url` points at.
///
/// Probes the resolver and TCP directly since git exits 128 for every fatal
/// error; a URL with no host to probe answers `true`, leaving the clone failure
/// to stand on its own.
async fn host_reachable(clone_url: &str, uses_ssh: bool, probe: HttpTimeouts) -> bool {
    let Some((host, port)) = clone_host_port(clone_url, uses_ssh) else {
        return true;
    };
    let walk = async {
        // Every resolved address counts, not just the first: a pod with no IPv6
        // route reaches a dual-stack host only on the A record it is handed
        // second, and calling that host unreachable would send a genuine 404
        // into a crash loop. `probe.total` bounds the walk as a whole.
        for addr in tokio::net::lookup_host((host.as_str(), port)).await.ok()? {
            if TcpStream::connect(addr).await.is_ok() {
                return Some(());
            }
        }
        None
    };
    matches!(tokio::time::timeout(probe.total, walk).await, Ok(Some(())))
}

/// Host and port to probe for a clone URL: the scheme's default port unless the
/// URL names its own.
fn clone_host_port(clone_url: &str, uses_ssh: bool) -> Option<(String, u16)> {
    let (rest, default_port) = if let Some(rest) = clone_url.strip_prefix("https://") {
        (rest, 443)
    } else if let Some(rest) = clone_url.strip_prefix("http://") {
        (rest, 80)
    } else if let Some(rest) = clone_url.strip_prefix("ssh://") {
        (rest, 22)
    } else if let Some(rest) = clone_url.strip_prefix("git+ssh://") {
        (rest, 22)
    } else if uses_ssh {
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

/// Fetch blobs on demand instead of shallowing the clone.
///
/// Deliberately not `--depth 1`, which implies `--single-branch`: the
/// `remote.origin.fetch` that writes covers the clone branch alone, so a
/// feature branch gets no remote-tracking ref (`git push --force-with-lease`
/// is rejected `stale info`) and `origin/main...HEAD` has no merge base.
const CLONE_FILTER: &str = "--filter=blob:none";

/// `git clone` arguments for one checkout, blobless and with every branch.
///
/// `--` closes the option list: nothing validates a stored `remote_url`, so a
/// row starting one with `-` would otherwise arrive here as a flag.
fn git_clone_args<'a>(url: &'a str, dest: &'a str, branch: Option<&'a str>) -> Vec<&'a str> {
    let mut args = vec!["clone", CLONE_FILTER];
    if let Some(branch) = branch {
        args.push("--branch");
        args.push(branch);
    }
    args.push("--");
    args.push(url);
    args.push(dest);
    args
}

/// Clone one repository into `dest`.
///
/// GitHub repos try `gh repo clone` first, falling back to `git clone`; other
/// providers use `git clone` with a token-authenticated HTTPS URL.
async fn clone_checkout(
    repo: &RepoCheckout,
    dest: &str,
    pod_provider: &str,
    credentials: &GitCredentials,
) -> Result<(), String> {
    let clone_url = &repo.clone_url;
    let is_ssh = repo.uses_ssh;
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
        let git_args = git_clone_args(clone_url, dest, branch);
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
        args.push(CLONE_FILTER);

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
    if let Some(token) = credentials.provider_token(&repo.provider, pod_provider) {
        let auth_url = inject_token_into_url(clone_url, &token);
        let git_args = git_clone_args(&auth_url, dest, branch);
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
    let git_args = git_clone_args(clone_url, dest, branch);
    match run_command("git", &git_args).await {
        Ok(()) => {
            info!("Repository cloned successfully (plain git clone)");
            Ok(())
        }
        Err(e) => Err(format!("All clone methods failed. Last error: {e}")),
    }
}

pub(crate) use moatless_types::credential_key::GIT_TOKEN;

/// The git keys of one credential fetch: provider tokens, `GIT_TOKEN` and the
/// author identity.
///
/// Handed to [`configure_git`] and [`clone_repositories`] rather than exported
/// into the process env, where a value would freeze at spawn time.
#[derive(Clone, Debug, Default)]
pub struct GitCredentials(HashMap<String, String>);

impl GitCredentials {
    const KEYS: [&str; 6] = [
        "GITHUB_TOKEN",
        "GITLAB_TOKEN",
        "GITNESS_TOKEN",
        GIT_TOKEN,
        "GIT_USER_NAME",
        "GIT_USER_EMAIL",
    ];

    /// Keep the git keys of a fetched credential map, dropping empty values.
    pub fn from_credentials(credentials: &HashMap<String, String>) -> Self {
        Self(
            Self::KEYS
                .iter()
                .filter_map(|key| {
                    let value = credentials.get(*key).filter(|value| !value.is_empty())?;
                    Some(((*key).to_string(), value.clone()))
                })
                .collect(),
        )
    }

    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }

    /// One provider's clone token. `GIT_TOKEN` backs a checkout only when it
    /// matches the *pod's own* provider, never lending it to a foreign host.
    fn provider_token(&self, provider: &str, pod_provider: &str) -> Option<String> {
        // A proxied checkout addresses the backend, which holds the credential and
        // attaches it upstream. Injecting the pod's own provider token into that URL
        // would hand it to a host that never asked for it.
        if provider == PROXIED_PROVIDER {
            return None;
        }
        let key = match provider {
            "gitlab" => "GITLAB_TOKEN",
            "gitness" => "GITNESS_TOKEN",
            "github" => "GITHUB_TOKEN",
            _ => return None,
        };
        self.get(key).or_else(|| {
            if provider == pod_provider {
                self.get(GIT_TOKEN)
            } else {
                None
            }
        })
    }
}

/// Remove the workspace directory's contents but keep the directory itself.
///
/// Needed between clone fallback attempts: leftover `.git/` from a failed clone
/// makes git refuse a non-empty dir. The dir itself is kept since it may be a
/// mount point (e.g. a PVC).
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

/// Extract "owner/repo" from a GitHub URL, handling both the
/// `https://github.com/owner/repo.git` and `git@github.com:owner/repo.git`
/// forms.
fn extract_github_repo_path(url: &str) -> String {
    url.replace("https://github.com/", "")
        .replace("git@github.com:", "")
        .trim_end_matches(".git")
        .to_string()
}

/// Inject a token into an HTTPS clone URL: `https://host/path` becomes
/// `https://x-access-token:{token}@host/path`, replacing any embedded
/// credentials.
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

/// Sets the broker-backed credential helper as `origin`'s only one, via
/// `--replace-all`.
async fn wire_broker_git_helper(origin: &str) {
    if !std::path::Path::new(BROKER_HELPER_PATH).exists() {
        info!(
            path = BROKER_HELPER_PATH,
            "Broker credential helper script not present — git has no credential helper"
        );
        return;
    }

    let helper_value = format!("!{BROKER_HELPER_PATH}");
    let key = format!("credential.{origin}.helper");
    if let Err(e) = run_command(
        "git",
        &["config", "--global", "--replace-all", &key, &helper_value],
    )
    .await
    {
        warn!(error = %e, origin, "Failed to wire moatless-git-cred — git has no credential helper");
        return;
    }

    info!(
        helper = BROKER_HELPER_PATH,
        origin, "Wired broker-backed git credential helper"
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
    /// concurrently-running test cannot be handed it and make the connect
    /// succeed.
    const DEAD_PORT: u16 = 1;

    fn checkout(mount: &str, provider: &str, is_primary: bool) -> RepoCheckout {
        RepoCheckout {
            uses_ssh: false,
            clone_url: format!("https://{provider}.example/acme/{mount}.git"),
            repository_id: String::new(),
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

    /// A repo whose directory already holds a `.git` is left alone, so a
    /// restart onto a persistent volume neither re-clones nor reports a
    /// reclone.
    #[tokio::test]
    async fn already_cloned_repos_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().display().to_string();
        let mut config = minimal_config();
        config.workspace_dir = workspace.clone();
        config.repos = vec![checkout("api", "github", true)];
        config.repo_dir = format!("{workspace}/api");
        std::fs::create_dir_all(format!("{workspace}/api/.git")).unwrap();

        let summary = clone_repositories(&config, &GitCredentials::default())
            .await
            .unwrap();
        assert!(summary.cloned.is_empty());
        assert!(!summary.primary_recloned);
    }

    /// A restore can carry an older workspace shape: the primary repo present
    /// but a later-added sibling absent.
    ///
    /// The clone pass after restore must clone only the missing sibling, and
    /// not count the primary as recloned.
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
                uses_ssh: false,
                clone_url: "https://example.test/acme/primary.git".to_string(),
                repository_id: String::new(),
                branch: None,
                provider: "local".to_string(),
                mount_name: "primary".to_string(),
                is_primary: true,
            },
            RepoCheckout {
                uses_ssh: false,
                clone_url: source.display().to_string(),
                repository_id: String::new(),
                branch: None,
                provider: "local".to_string(),
                mount_name: "secondary".to_string(),
                is_primary: false,
            },
        ];

        let summary = clone_repositories(&config, &GitCredentials::default())
            .await
            .unwrap();

        assert_eq!(summary.cloned, vec!["secondary"]);
        assert!(!summary.primary_recloned);
        assert!(workspace.join("secondary").join(".git").is_dir());
        assert!(workspace.join("secondary").join("README.md").is_file());
    }

    #[test]
    fn a_clone_is_blobless_rather_than_shallow() {
        assert_eq!(
            git_clone_args("https://example.test/acme/api.git", "/w/api", Some("dev")),
            vec![
                "clone",
                "--filter=blob:none",
                "--branch",
                "dev",
                "--",
                "https://example.test/acme/api.git",
                "/w/api",
            ],
        );
        // `--depth` back at any site restores the single-branch refspec that
        // breaks `--force-with-lease` and `origin/main...HEAD`.
        assert!(!git_clone_args("u", "d", None).contains(&"--depth"));
    }

    /// The three things a `--depth 1` clone took away: a branch other than the
    /// clone branch, a lease-eligible tracking ref, and a merge base.
    #[tokio::test]
    async fn a_cloned_repo_carries_every_branch_and_a_merge_base() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let source = tmp.path().join("source");
        init_local_source_repo(&source);
        run_git_sync(&source, &["branch", "feature"]);

        let workspace_str = workspace.display().to_string();
        let mut config = minimal_config();
        config.workspace_dir = workspace_str.clone();
        config.repo_dir = workspace.join("api").display().to_string();
        config.git_provider = "local".to_string();
        config.repos = vec![RepoCheckout {
            uses_ssh: false,
            clone_url: source.display().to_string(),
            repository_id: String::new(),
            branch: Some("main".to_string()),
            provider: "local".to_string(),
            mount_name: "api".to_string(),
            is_primary: true,
        }];

        clone_repositories(&config, &GitCredentials::default())
            .await
            .unwrap();

        let clone = workspace.join("api");
        let read = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .current_dir(&clone)
                .args(args)
                .output()
                .expect("git command runs");
            assert!(out.status.success(), "git {args:?} failed");
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        assert_eq!(read(&["rev-parse", "--is-shallow-repository"]), "false");
        assert_eq!(
            read(&["config", "--get-all", "remote.origin.fetch"]),
            "+refs/heads/*:refs/remotes/origin/*"
        );
        // A branch the clone did not check out still has a tracking ref, which
        // is what a bare `--force-with-lease` leases against.
        read(&["rev-parse", "--verify", "refs/remotes/origin/feature"]);
        read(&["merge-base", "origin/main", "HEAD"]);
    }

    /// Each checkout authenticates with its own provider's token; the pod's
    /// `GIT_TOKEN` backs up only a checkout on the pod's own provider.
    ///
    /// Injecting it into another host's clone URL would hand the credential to
    /// a third party.
    #[test]
    fn each_checkout_uses_its_own_providers_token() {
        let credentials =
            credentials(&[("GITLAB_TOKEN", "glpat"), (GIT_TOKEN, "pod-github-token")]);
        assert_eq!(
            credentials.provider_token("gitlab", "github"),
            Some("glpat".to_string())
        );
        // No `GITHUB_TOKEN` was delivered, so GitHub uses the pod's.
        assert_eq!(
            credentials.provider_token("github", "github"),
            Some("pod-github-token".to_string())
        );
        // A foreign provider with no token of its own gets nothing.
        assert_eq!(credentials.provider_token("gitness", "github"), None);
    }

    /// A proxied checkout's URL points at the backend, so no provider token may
    /// be injected into it — not even the pod's own.
    #[test]
    fn a_proxied_checkout_is_given_no_provider_token() {
        let credentials = credentials(&[("GITHUB_TOKEN", "pod-token"), (GIT_TOKEN, "pod-token")]);
        assert_eq!(credentials.provider_token("forgejo", "forgejo"), None);
        assert_eq!(credentials.provider_token("forgejo", "github"), None);
    }

    fn credentials(pairs: &[(&str, &str)]) -> GitCredentials {
        GitCredentials::from_credentials(
            &pairs
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
        )
    }

    /// The probe answers from the resolver and the TCP stack, so both verdicts
    /// are exercised against a real local socket rather than the public
    /// network.
    ///
    /// That means a listener that exists, and a port nothing is bound to.
    #[tokio::test]
    async fn probe_reports_reachable_and_unreachable_hosts() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let live = listener.local_addr().unwrap().port();
        assert!(
            host_reachable(
                &format!("https://127.0.0.1:{live}/acme/api.git"),
                false,
                probe()
            )
            .await
        );

        // Nothing is bound to `DEAD_PORT`, so the connect is refused outright.
        assert!(
            !host_reachable(
                &format!("https://127.0.0.1:{DEAD_PORT}/acme/api.git"),
                false,
                probe()
            )
            .await
        );
    }

    /// A multi-address host is reachable when *any* of its addresses connects.
    ///
    /// On a dual-stack runner, `localhost` resolves `::1` before `127.0.0.1`,
    /// so a `127.0.0.1`-only listener needs walking past the first address.
    #[tokio::test]
    async fn probe_walks_every_resolved_address() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(
            host_reachable(
                &format!("https://localhost:{port}/acme/api.git"),
                false,
                probe()
            )
            .await
        );
    }

    /// The retry loop must not sleep the budget away and then start an attempt
    /// it cannot afford — the startup probe SIGKILLs the container mid-clone
    /// and the error never reaches the termination log.
    #[tokio::test]
    async fn retry_stops_before_an_attempt_it_cannot_afford() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = RepoCheckout {
            uses_ssh: false,
            clone_url: format!("https://127.0.0.1:{DEAD_PORT}/acme/api.git"),
            repository_id: String::new(),
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
            &GitCredentials::default(),
            probe(),
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
            uses_ssh: false,
            clone_url: format!("http://127.0.0.1:{port}/acme/api.git"),
            repository_id: String::new(),
            branch: None,
            provider: "local".to_string(),
            mount_name: "api".to_string(),
            is_primary: true,
        }
    }

    /// A service answering 5xx is reporting its own outage, not passing a
    /// verdict on the repo. Reachability alone would call that permanent and
    /// kill the sandbox for the length of a provider incident.
    #[tokio::test]
    async fn service_side_5xx_stays_retryable() {
        let port = serve_status("503 Service Unavailable").await;
        assert!(
            failure_is_retryable(&repo_at(port), "local", &GitCredentials::default(), probe())
                .await
        );
    }

    /// A 4xx never decides on its own — it falls through to reachability — but
    /// a host that answers leaves the failure the repo's own, and permanent.
    #[tokio::test]
    async fn client_error_from_a_reachable_host_is_not_retryable() {
        let port = serve_status("404 Not Found").await;
        assert!(
            !failure_is_retryable(&repo_at(port), "local", &GitCredentials::default(), probe())
                .await
        );
    }

    /// A URL with no host to probe teaches nothing about the pod's network, so
    /// the clone failure has to stand on its own and stay permanent.
    #[tokio::test]
    async fn hostless_url_counts_as_reachable() {
        assert!(host_reachable("/srv/git/local-repo.git", false, probe()).await);
    }

    /// The probe targets the port the clone would actually use, and an injected
    /// token in the URL's userinfo must not be mistaken for the host.
    #[test]
    fn clone_urls_resolve_to_the_host_and_port_the_clone_uses() {
        let port = |url: &str| clone_host_port(url, false);
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
            clone_host_port("git@github.com:owner/repo.git", true),
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

    /// The incident shape: the host is unreachable and the retry budget is
    /// gone, so the failure is reported as environmental (exit 1, the restart
    /// retries) rather than as a permanent config error.
    #[tokio::test]
    async fn exhausted_budget_against_an_unreachable_host_stays_retryable() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = RepoCheckout {
            uses_ssh: false,
            clone_url: format!("https://127.0.0.1:{DEAD_PORT}/acme/api.git"),
            repository_id: String::new(),
            branch: None,
            provider: "local".to_string(),
            mount_name: "api".to_string(),
            is_primary: true,
        };

        let err = clone_with_retry(
            &repo,
            &tmp.path().join("api").display().to_string(),
            "local",
            &GitCredentials::default(),
            probe(),
            Instant::now(),
        )
        .await
        .expect_err("a clone against a dead port must fail");

        assert!(err.retryable);
        assert!(err.to_string().contains("api"), "message was: {err}");
    }

    /// A repo the pod can reach is judged on the clone itself: no amount of
    /// retrying makes a missing repository appear, so it must not be reported
    /// as a network fault.
    #[tokio::test]
    async fn reachable_host_reports_the_failure_as_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = RepoCheckout {
            uses_ssh: false,
            // No host to probe, so the probe cannot excuse the failure.
            clone_url: tmp.path().join("does-not-exist").display().to_string(),
            repository_id: String::new(),
            branch: None,
            provider: "local".to_string(),
            mount_name: "api".to_string(),
            is_primary: true,
        };

        let err = clone_with_retry(
            &repo,
            &tmp.path().join("api").display().to_string(),
            "local",
            &GitCredentials::default(),
            probe(),
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

    /// The backend hands the sandbox a credential-free clone URL (userinfo
    /// stripped by `pod_builder::redact_url_userinfo`); cloning
    /// re-authenticates by injecting the provider token.
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

    fn probe() -> HttpTimeouts {
        crate::config::SandboxHttpConfig::default().git_probe
    }
}
