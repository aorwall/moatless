//! Git configuration and repository cloning.
//!
//! Provider-agnostic: the git identity/credential-helper wiring follows the
//! pod's own `GIT_PROVIDER`, while each of the workspace's checkouts
//! ([`Config::repos`]) clones with the token for its own provider.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use moatless_types::http::HttpTimeouts;
use moatless_workspace_types::workspace::RepoCheckout;
use tokio::net::TcpStream;
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::Config;
use crate::credentials::broker::BROKER_HELPER_PATH;

/// Configure git authentication: the broker-backed credential helper (plus
/// `gh api user` for identity on GitHub), a token-based helper on other
/// providers.
pub async fn configure_git(config: &Config, credentials: &GitCredentials) -> Result<(), String> {
    let is_ssh = crate::config::primary_checkout(&config.repos).is_some_and(|repo| repo.uses_ssh);
    let is_github = config.git_provider == "github";
    let proxy_origin = config.backend_api_origin();

    // Ahead of the no-token exits below, which a pod holding only proxied repos
    // would otherwise take before reaching this.
    for origin in broker_origins(config) {
        wire_broker_git_helper(&origin).await;
    }

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

/// The origins git asks the credential broker about
/// (`credentials::broker::get_git_credential`): github.com whatever the pod's
/// own provider, and the backend wherever it is known, so `moat repos clone`
/// reaches a repository the Workspace places no checkout for.
pub(crate) fn broker_origins(config: &Config) -> Vec<String> {
    let mut origins = vec![GITHUB_ORIGIN.to_string()];
    origins.extend(config.backend_api_origin());
    origins
}

const GITHUB_ORIGIN: &str = "https://github.com";

/// Strip broker-origin credentials from every git config the sandbox can
/// reach; see [`super::git_remotes`].
pub async fn scrub_remote_credentials(config: &Config) {
    super::git_remotes::scrub_workspace(Path::new(&config.workspace_dir), &broker_origins(config))
        .await;
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
    let broker_origins = broker_origins(config);
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
        let token = clone_token(config, credentials, repo);
        let source = CloneSource {
            token: token.as_deref(),
            github_upstream: config
                .github_upstreams
                .get(&repo.mount_name)
                .map(String::as_str),
            broker_origins: &broker_origins,
            stall_secs: CLONE_STALL_SECS,
        };
        clone_with_retry(repo, &dest, &source, config.http.git_probe, deadline).await?;
        summary.cloned.push(repo.mount_name.clone());
        summary.primary_recloned |= repo.is_primary;
    }
    Ok(summary)
}

/// A failed clone, and whether another attempt could get past it.
#[derive(Debug)]
pub struct CloneFailure {
    message: String,
    /// Another attempt could get past this: the failure reflects reachability
    /// or the transfer, not the service's verdict on the repo or its
    /// credentials — so init retries instead of reporting a permanent error.
    pub retryable: bool,
}

impl std::fmt::Display for CloneFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The provider token `repo` may clone with: none through the backend proxy,
/// which authenticates the pod's task token and would reject a provider one.
fn clone_token(
    config: &Config,
    credentials: &GitCredentials,
    repo: &RepoCheckout,
) -> Option<String> {
    if config.clones_through_proxy(repo) {
        return None;
    }
    credentials.provider_token(&repo.provider, &config.git_provider)
}

/// What one checkout clones with, beyond its own [`RepoCheckout`].
struct CloneSource<'a> {
    /// The provider token git may fall back to. `None` for a checkout cloning
    /// through the backend proxy, which would reject it as a task token.
    token: Option<&'a str>,
    /// The GitHub URL a proxied checkout stands in for, recorded as the
    /// [`GITHUB_REMOTE`] remote.
    github_upstream: Option<&'a str>,
    /// The origins the credential broker answers ([`broker_origins`]).
    broker_origins: &'a [String],
    /// [`CLONE_STALL_SECS`] outside tests.
    stall_secs: u64,
}

/// Wall-clock ceiling on clone retries, shared by every repo in one pass.
///
/// The deliberate loser of three startup budgets (credential fetch, workspace
/// restore, this): the probe may SIGKILL it mid-backoff since the restore's
/// report matters more, and the container restart is the outer retry.
const CLONE_RETRY_BUDGET: Duration = Duration::from_secs(45);

/// How long a clone may receive nothing before git abandons it.
///
/// A stall limit rather than a wall-clock cap, so a large checkout still
/// transferring is never cut off. Inside the default startup probe window
/// (150s), so a host that black-holes the clone fails here and init's error
/// reaches the termination log rather than the probe's SIGKILL taking it.
pub(crate) const CLONE_STALL_SECS: u64 = 60;
// The default window: `(K8S_POD_STARTUP_TIMEOUT_SECS / 5 + 6) * 5`.
const _: () = assert!(CLONE_STALL_SECS < (120 / 5 + 6) * 5);

/// Longest single backoff between clone attempts.
const MAX_CLONE_RETRY_DELAY: Duration = Duration::from_secs(8);

/// Clone one repo, retrying while [`failure_is_retryable`] holds and
/// `deadline` allows.
///
/// Any other failure is the repo's own, so this returns at once — retrying a
/// 404 only burns the startup budget.
async fn clone_with_retry(
    repo: &RepoCheckout,
    dest: &str,
    source: &CloneSource<'_>,
    probe: HttpTimeouts,
    deadline: Instant,
) -> Result<(), CloneFailure> {
    let mut delay = Duration::from_secs(2);
    loop {
        let error = match clone_checkout(repo, dest, source).await {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };
        let message = format!("{} ({}): {error}", repo.mount_name, repo.clone_url);

        if !failure_is_retryable(repo, source.token, probe).await {
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
/// Two measured signals, never git's stderr: the service's advertisement for
/// this repo, and whether the host answers a TCP connect.
async fn failure_is_retryable(
    repo: &RepoCheckout,
    token: Option<&str>,
    probe: HttpTimeouts,
) -> bool {
    // A 5xx is the service's own outage; a 2xx advertising the branch leaves
    // only the transfer to blame (e.g. GitHub cancelling mid-pack). Only ever
    // promotes a failure: a 4xx falls through to reachability, so a
    // differently-answering endpoint can't make a working repo permanent.
    if let Some(advertisement) = repo_advertisement(repo, token, probe).await {
        if advertisement.status.is_server_error() {
            return true;
        }
        let branch = repo.branch.as_deref().filter(|b| !b.is_empty());
        if advertisement.status.is_success()
            && advertisement.is_upload_pack()
            && branch.is_none_or(|branch| advertisement.names_ref(branch))
        {
            return true;
        }
    }
    !host_reachable(&repo.clone_url, repo.uses_ssh, probe).await
}

/// What `GET <repo>/info/refs?service=git-upload-pack` answered.
struct RepoAdvertisement {
    status: reqwest::StatusCode,
    /// The `info/refs` body: one pkt-line per ref, or empty when the body
    /// could not be read.
    body: String,
}

impl RepoAdvertisement {
    /// Whether the body is git's ref list, which opens with the
    /// `# service=git-upload-pack` line, rather than an SSO login page a
    /// redirect landed on with a 200.
    fn is_upload_pack(&self) -> bool {
        self.body.starts_with("001e# service=git-upload-pack")
    }

    /// Whether the advertisement lists `name` as a branch or a tag, the two
    /// kinds `git clone --branch` accepts.
    fn names_ref(&self, name: &str) -> bool {
        let wanted = [format!("refs/heads/{name}"), format!("refs/tags/{name}")];
        self.body.lines().any(|line| {
            // A ref line is `<len><oid> <ref>`, optionally followed by `\0`
            // and the capability list on the first one.
            let line = line.split('\0').next().unwrap_or(line);
            line.rsplit_once(' ')
                .is_some_and(|(_, refname)| wanted.iter().any(|w| w == refname))
        })
    }
}

/// The repo's [`RepoAdvertisement`], or `None` when there is nothing to ask:
/// an SSH remote, or a request that never completed.
async fn repo_advertisement(
    repo: &RepoCheckout,
    token: Option<&str>,
    probe: HttpTimeouts,
) -> Option<RepoAdvertisement> {
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
    if let Some(token) = token {
        request = request.basic_auth("x-access-token", Some(token));
    }
    let response = request.send().await.ok()?;
    let status = response.status();
    let body = response.text().await.unwrap_or_else(|error| {
        warn!(mount = %repo.mount_name, %error, "Could not read the repo's ref advertisement");
        String::new()
    });
    Some(RepoAdvertisement { status, body })
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

/// The remote naming the GitHub repository a proxied checkout's `origin`
/// reaches.
///
/// Plain `gh` resolves its repository from a remote on a GitHub host and finds
/// none in `origin`, and the push watcher claims a push to `origin` under this
/// remote's `owner/repo`.
pub(crate) const GITHUB_REMOTE: &str = "github";

/// `git clone --config` values adding [`GITHUB_REMOTE`]: fetched from
/// `upstream`, pushed through `proxy_url`, so a `gh pr checkout` branch pushes
/// through the proxy. `gh` refuses a branch without the tracking refspec;
/// `skipFetchAll` keeps `git fetch --all` on the proxy.
fn github_remote_config(upstream: &str, proxy_url: &str) -> [String; 4] {
    [
        format!("remote.{GITHUB_REMOTE}.url={upstream}"),
        format!("remote.{GITHUB_REMOTE}.pushurl={proxy_url}"),
        format!("remote.{GITHUB_REMOTE}.fetch=+refs/heads/*:refs/remotes/{GITHUB_REMOTE}/*"),
        format!("remote.{GITHUB_REMOTE}.skipFetchAll=true"),
    ]
}

/// Clone one repository into `dest`.
///
/// A repo whose origin the broker answers clones from its plain URL, so the
/// token never lands in `origin`; any other HTTPS repo clones with its
/// provider token in the URL, since no helper would answer its host later.
async fn clone_checkout(
    repo: &RepoCheckout,
    dest: &str,
    source: &CloneSource<'_>,
) -> Result<(), String> {
    let clone_url = &repo.clone_url;
    let is_ssh = repo.uses_ssh;

    let branch = repo.branch.as_deref().filter(|b| !b.is_empty());

    info!(
        url = %clone_url,
        provider = %repo.provider,
        mount = %repo.mount_name,
        ssh = is_ssh,
        branch = ?branch,
        "Cloning repository"
    );

    // SSH remote: clone the URL directly. Auth and host-key checking come
    // from the identities/config credentials.rs wrote to ~/.ssh, no token
    // involved (checked first, so an SSH URL to a configured-provider host
    // still takes the SSH path). Clones into `repo_dir`: `workspace_dir`
    // is no longer empty by clone time, and git refuses a non-empty dir.
    if is_ssh {
        let git_args = git_clone_args(clone_url, dest, branch);
        return match run_clone(&git_args, None, source.stall_secs).await {
            Ok(()) => {
                info!("Repository cloned successfully over SSH");
                Ok(())
            }
            Err(e) => Err(format!("SSH clone failed: {e}")),
        };
    }

    let token = source.token;
    let broker_answers = super::git_remotes::url_origin(clone_url).is_some_and(|origin| {
        source
            .broker_origins
            .iter()
            .any(|known| known.eq_ignore_ascii_case(&origin))
    });
    if broker_answers {
        // Never fall back to a token in the URL: it outranks the broker's
        // helper and keeps being sent after it expires.
        let github_remote = source
            .github_upstream
            .map(|upstream| github_remote_config(upstream, clone_url));
        let mut git_args = git_clone_args(clone_url, dest, branch);
        for setting in github_remote.iter().flatten() {
            git_args.insert(1, setting);
            git_args.insert(1, "--config");
        }
        return match run_clone(&git_args, token, source.stall_secs).await {
            Ok(()) => {
                info!("Repository cloned successfully through the credential broker");
                Ok(())
            }
            Err(e) => Err(format!("git clone failed: {e}")),
        };
    }

    // Try git clone with the token for *this* repo's provider (works for all
    // providers), so a workspace mixing e.g. a GitHub and a GitLab repo
    // authenticates each with the right credential.
    if let Some(token) = token {
        let auth_url = inject_token_into_url(clone_url, token);
        let git_args = git_clone_args(&auth_url, dest, branch);
        match run_clone(&git_args, None, source.stall_secs).await {
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
    match run_clone(&git_args, None, source.stall_secs).await {
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
    const KEYS: [&str; 5] = [
        "GITHUB_TOKEN",
        "GITLAB_TOKEN",
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
    ///
    /// Callers ask only for a checkout that does not clone through the backend
    /// proxy ([`Config::clones_through_proxy`]).
    fn provider_token(&self, provider: &str, pod_provider: &str) -> Option<String> {
        let key = match provider {
            "gitlab" => "GITLAB_TOKEN",
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

/// The variable [`CLONE_TOKEN_HELPER`] reads the clone token from.
const CLONE_TOKEN_ENV: &str = "MOATLESS_CLONE_TOKEN";

/// A helper answering with the token in [`CLONE_TOKEN_ENV`]. `-c` puts it after
/// the configured helpers, so it answers only when the broker does not, and
/// nothing writes it into the checkout's config.
const CLONE_TOKEN_HELPER: &str = "credential.helper=!f() { test \"$1\" = get || exit 0; \
     echo username=x-access-token; echo \"password=$MOATLESS_CLONE_TOKEN\"; }; f";

/// Run `git <git_args>`, offering `token` to git only for this process, and
/// failing once an HTTP transfer moves under 1 KiB/s for `stall_secs`.
///
/// SSH stalls are bounded in the ssh config. No `GIT_SSH_COMMAND` here: it
/// overrides `core.sshCommand`, and SSH clones lose their identities.
async fn run_clone(git_args: &[&str], token: Option<&str>, stall_secs: u64) -> Result<(), String> {
    let mut command = Command::new("git");
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_HTTP_LOW_SPEED_LIMIT", "1000")
        .env("GIT_HTTP_LOW_SPEED_TIME", stall_secs.to_string());
    if let Some(token) = token {
        command
            .args(["-c", CLONE_TOKEN_HELPER])
            .env(CLONE_TOKEN_ENV, token);
    }
    let output = command
        .args(git_args)
        .output()
        .await
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(())
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
mod tests;
