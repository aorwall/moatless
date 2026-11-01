// Safety: set_var is unsafe in multi-threaded context. This module is called
// during single-threaded init *and* then periodically by the credential
// refresher loop (see `super::refresher`). There are no other writers of these
// env vars after init — the refresher is a single serialized loop.
#![allow(unsafe_code)]

//! Credential fetching and application.
//!
//! Fetches credentials from the backend via gRPC `GetCredentials` RPC and sets
//! them as environment variables. The backend sends no model credential: the
//! agent reaches every provider through its model gateway.

use std::collections::HashMap;
use std::time::Duration;

use moatless_types::credential_key::{
    ANTHROPIC_API_KEY, ANTHROPIC_AUTH_TOKEN, ANTHROPIC_BASE_URL, GH_TOKEN, USER_AUTH_TOKEN,
};
use tokio::fs;
use tracing::{info, warn};

use crate::config::Config;
use crate::init::git::{GIT_TOKEN, GitCredentials};
use crate::state::SandboxState;

/// Outcome of a failed credential fetch, so init can pick the right exit code.
#[derive(Debug)]
pub enum CredentialFetchError {
    /// Retriable — e.g. the backend is not yet routable, or a temporary error.
    /// K8s restarts the container and init tries again.
    Transient(String),
    /// Unrecoverable without re-provisioning: the backend refused every token
    /// this sandbox holds, and a retry or restart offers the same set. Surface
    /// an actionable error and stop the crash loop.
    Permanent(String),
}

impl std::fmt::Display for CredentialFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transient(msg) | Self::Permanent(msg) => f.write_str(msg),
        }
    }
}

/// Keys that must NOT be set in the sandbox process environment via `set_var`.
///
/// A value set there freezes for the pod's lifetime and outlives the token
/// behind it. `gh` and git reach GitHub through the backend's proxies, and git
/// takes a GitLab token from `SandboxState::git_credentials`.
const ENV_SET_BLOCKLIST: &[&str] = &["GITHUB_TOKEN", GH_TOKEN, "GITLAB_TOKEN", GIT_TOKEN];

/// Keys that must NOT be inherited by the agent subprocess, which would
/// snapshot a stale value at spawn time.
const AGENT_EXCLUDED_KEYS: &[&str] = ENV_SET_BLOCKLIST;

/// Wall-clock ceiling on the credential-fetch retry loop.
///
/// The startup probe SIGKILLs the container after `K8S_POD_STARTUP_TIMEOUT_SECS
/// / 5 + 6` failed `/health` checks (150s default); staying under that window
/// lets the loop report its own error before the probe kills it mid-backoff.
const RETRY_BUDGET: Duration = Duration::from_secs(90);

/// Longest single backoff between attempts.
const MAX_RETRY_DELAY: Duration = Duration::from_mins(1);

/// Produce the credential map passed to the agent subprocess env.
///
/// Strips git-provider tokens that would otherwise freeze at whatever value was
/// current when the agent subprocess was spawned.
pub fn filter_agent_credentials(credentials: &HashMap<String, String>) -> HashMap<String, String> {
    credentials
        .iter()
        .filter(|(k, _)| !AGENT_EXCLUDED_KEYS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// The agent subprocess's credential map for one `GetCredentials` answer:
/// [`filter_agent_credentials`], then [`route_claude_through_gateway`] when the
/// pod knows its backend.
pub fn agent_credentials(
    credentials: &HashMap<String, String>,
    backend_url: Option<&str>,
) -> HashMap<String, String> {
    let mut credentials = filter_agent_credentials(credentials);
    if let Some(backend_url) = backend_url {
        route_claude_through_gateway(&mut credentials, backend_url);
    }
    credentials
}

/// The Anthropic base URL of the backend at `backend_url`.
pub fn model_gateway_url(backend_url: &str) -> String {
    format!("{}/llm/anthropic", backend_url.trim_end_matches('/'))
}

/// Point Claude at [`model_gateway_url`], authenticated by the task token its
/// `apiKeyHelper` reads (see [`gateway_api_key_helper`]).
///
/// Both keys are blanked rather than removed: the CLI prefers either over the
/// helper, the pod's own environment may still hold one, and the CLI reads an
/// empty value as unset. A `CLAUDE_CODE_OAUTH_TOKEN` loses to the helper.
fn route_claude_through_gateway(credentials: &mut HashMap<String, String>, backend_url: &str) {
    credentials.insert(
        ANTHROPIC_BASE_URL.to_string(),
        model_gateway_url(backend_url),
    );
    credentials.insert(ANTHROPIC_API_KEY.to_string(), String::new());
    credentials.insert(ANTHROPIC_AUTH_TOKEN.to_string(), String::new());
}

/// The `apiKeyHelper` that hands Claude the current task token, when `env`
/// routes it through the backend's model gateway.
///
/// Read per call rather than set as `ANTHROPIC_API_KEY`: a persistent TUI keeps
/// its spawn-time env, so a token rotated by a driver switch would `401` every
/// later turn. The CLI re-runs the helper on a `401`.
pub fn gateway_api_key_helper(
    config: &Config,
    env: Option<&HashMap<String, String>>,
) -> Option<String> {
    let gateway = model_gateway_url(config.backend_api_url.as_deref()?);
    let base_url = env?.get(ANTHROPIC_BASE_URL)?;
    if *base_url != gateway {
        return None;
    }
    let path = config.persisted_user_token_path();
    let quoted = path.to_string_lossy().replace('\'', r"'\''");
    Some(format!("cat '{quoted}'"))
}

/// Whether `fetch_and_apply_credentials` runs at sandbox init or from the
/// periodic refresher.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PersistMode {
    /// Tries the fallback tokens on a refusal, and removes leftover model
    /// logins (see [`persist_agent_credentials`]).
    Initial,
    /// Keeps the token it authenticated with and removes nothing.
    Refresh,
}

/// Fetch credentials from backend and apply them.
///
/// Retries with exponential backoff inside [`RETRY_BUDGET`] since the backend
/// may not be routable when the sandbox pod first starts.
pub async fn fetch_and_apply_credentials(state: &SandboxState) -> Result<(), CredentialFetchError> {
    fetch_and_apply_credentials_with_mode(state, PersistMode::Initial).await
}

/// Like [`fetch_and_apply_credentials`] but lets the caller declare whether
/// this is initial init or a periodic refresh (see [`PersistMode`]).
pub async fn fetch_and_apply_credentials_with_mode(
    state: &SandboxState,
    mode: PersistMode,
) -> Result<(), CredentialFetchError> {
    let backend_client = &state.backend_client;
    let task_id = &state.config.task_id;
    let mut delay = Duration::from_secs(3);
    let started = std::time::Instant::now();
    let mut attempt = 1;

    let mut fallbacks = fallback_tokens(&state.config, mode);

    loop {
        match backend_client.get_credentials(task_id).await {
            Ok(mut resp) => {
                let credential_names: Vec<&str> = resp
                    .credentials
                    .keys()
                    .map(std::string::String::as_str)
                    .collect();
                info!(
                    task_id = %task_id,
                    credential_count = resp.credentials.len(),
                    ?credential_names,
                    attempt,
                    "Credentials fetched successfully"
                );

                // Consume the long-lived user auth token: swap it into the gRPC
                // client and persist it so a
                // container restart can reuse it even after the bootstrap token
                // has expired. Removed from the map so it is never set as a
                // process/agent env var or written to server.env.
                if let Some(user_token) = resp.credentials.remove(USER_AUTH_TOKEN) {
                    backend_client.update_auth_token(user_token.clone());
                    persist_user_auth_token(&state.config, &user_token).await;
                }

                for (key, value) in &resp.credentials {
                    if ENV_SET_BLOCKLIST.contains(&key.as_str()) {
                        continue;
                    }
                    // SAFETY: write is serialized — init runs first on the main thread,
                    // then the single-loop refresher is the only caller. See module-level
                    // note at the top of this file.
                    unsafe {
                        std::env::set_var(key, value);
                    }
                }

                // A GitLab pod's token is also its `GIT_TOKEN`, so git.rs has one
                // name for it. GitHub has none: it is reached through the proxy.
                if state.config.git_provider == "gitlab" {
                    if let Some(token) = resp.credentials.get("GITLAB_TOKEN").cloned() {
                        resp.credentials.insert(GIT_TOKEN.to_string(), token);
                    }
                }
                *state.git_credentials.write().await =
                    GitCredentials::from_credentials(&resp.credentials)
                        .with_task_token(backend_client.auth_token());

                // Store credentials in state for passing to agent subprocess.
                // Filter out git-provider tokens so the long-running Claude CLI
                // subprocess doesn't snapshot a stale value at spawn time.
                *state.credentials.write().await =
                    agent_credentials(&resp.credentials, state.config.backend_api_url.as_deref());

                persist_agent_credentials(&state.config, mode).await;

                return Ok(());
            }
            Err(e) => {
                // A rejection no retry can change: fail permanently with an
                // actionable message so the pod watcher stops the crash loop
                // instead of restarting into the same refusal.
                if let Some(reason) = terminal_rejection_reason(mode, e.code()) {
                    return Err(CredentialFetchError::Permanent(format!(
                        "{reason} (backend rejected credential fetch: {e})"
                    )));
                }
                if is_auth_rejection(mode, e.code()) {
                    // The token the client presented was refused. Which store
                    // holds a live one depends on the runtime, so try the rest
                    // before concluding anything.
                    if !fallbacks.is_empty() {
                        let next = fallbacks.remove(0);
                        warn!(
                            attempt,
                            remaining = fallbacks.len(),
                            error = %e,
                            "Backend refused the sandbox auth token, trying the next one"
                        );
                        backend_client.update_auth_token(next);
                        attempt += 1;
                        continue;
                    }
                    // Every token this pod holds has been refused, and all are
                    // fixed for its lifetime — a retry, or a K8s restart into
                    // the same env and file, presents the same set again. Only
                    // re-provisioning mints new ones, so fail permanently and
                    // let the pod watcher stop the crash loop.
                    return Err(CredentialFetchError::Permanent(format!(
                        "backend rejected every auth token this sandbox holds; redeploy \
                         the task (`moat tasks redeploy`) to re-provision it with fresh \
                         tokens (credential fetch failed: {e})"
                    )));
                }
                let elapsed = started.elapsed();
                if elapsed + delay >= RETRY_BUDGET {
                    return Err(CredentialFetchError::Transient(format!(
                        "Failed after {attempt} attempts in {}s: {e}",
                        elapsed.as_secs()
                    )));
                }
                warn!(
                    attempt,
                    error = %e,
                    delay_secs = delay.as_secs(),
                    "Credential fetch failed, retrying"
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(MAX_RETRY_DELAY);
                attempt += 1;
            }
        }
    }
}

/// The tokens init falls back to when the backend refuses the client's token.
///
/// The client boots on the first of [`Config::initial_auth_token_candidates`];
/// the fallbacks are the rest. A refresh has none — it keeps the token it
/// already authenticated with.
fn fallback_tokens(config: &Config, mode: PersistMode) -> Vec<String> {
    match mode {
        PersistMode::Initial => config
            .initial_auth_token_candidates()
            .into_iter()
            .skip(1)
            .collect(),
        PersistMode::Refresh => Vec::new(),
    }
}

/// Whether the backend refused the auth token on an *initial* fetch.
///
/// Refresh-mode failures are never treated this way — the refresher keeps its
/// already-authenticated token and retries next cycle regardless.
fn is_auth_rejection(mode: PersistMode, code: tonic::Code) -> bool {
    mode == PersistMode::Initial && code == tonic::Code::Unauthenticated
}

/// Why a failed credential fetch is unrecoverable, or `None` if retrying may
/// still succeed (→ [`CredentialFetchError::Permanent`] vs `Transient`).
///
/// `PermissionDenied` is an authorization decision either way: it won't change
/// on retry, and a restart reuses the same `ReplicaSet` identity.
fn terminal_rejection_reason(_mode: PersistMode, code: tonic::Code) -> Option<&'static str> {
    match code {
        tonic::Code::PermissionDenied => {
            Some("not authorized for this task; its sandbox runs as a user that cannot control it")
        }
        _ => None,
    }
}

/// Prepare the agent runtimes' config directories.
///
/// On [`PersistMode::Initial`] it removes any model login a pod built before
/// the gateway left on a reused volume, so no agent can read a credential the
/// backend no longer hands out.
pub async fn persist_agent_credentials(config: &Config, mode: PersistMode) {
    if mode == PersistMode::Initial {
        for path in [
            format!("{}/.credentials", config.claude_config_dir),
            format!("{}/auth.json", config.codex_home_dir),
        ] {
            match fs::remove_file(&path).await {
                Ok(()) => info!(%path, "Removed a leftover model login"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => warn!(error = %e, %path, "Failed to remove a leftover model login"),
            }
        }
    }
    write_onboarding_flag(&config.claude_config_dir).await;
}

/// Point the pod's global git identity at the one `credentials` name, so a
/// turn commits as the driver whose message started it rather than as
/// whoever drove the pod when init ran. Absent keys leave git as it is.
pub(crate) async fn apply_git_identity(credentials: &HashMap<String, String>) {
    for (key, setting) in [
        ("GIT_USER_NAME", "user.name"),
        ("GIT_USER_EMAIL", "user.email"),
    ] {
        let Some(value) = credentials.get(key).filter(|value| !value.is_empty()) else {
            continue;
        };
        let status = tokio::process::Command::new("git")
            .args(["config", "--global", setting, value])
            .status()
            .await;
        match status {
            Ok(status) if status.success() => {}
            Ok(status) => warn!(setting, %status, "git config exited unsuccessfully"),
            Err(e) => warn!(setting, error = %e, "Failed to run git config"),
        }
    }
}

/// Persist the long-lived user auth token to disk so a container restart can
/// reuse it (see [`Config::persisted_user_token_path`]).
///
/// Best-effort: a write failure just falls back to the initial token on the
/// next restart, so failures are logged, not fatal.
pub(crate) async fn persist_user_auth_token(config: &Config, token: &str) {
    purge_exposed_user_tokens(config).await;

    let path = config.persisted_user_token_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent).await {
            warn!(error = %e, "Failed to create dir for persisted user auth token");
            return;
        }
    }
    // Renamed into place: the agent's `apiKeyHelper` reads this file at any
    // moment, and a write in place can hand it a truncated token.
    let staged = path.with_extension("tmp");
    if let Err(e) = fs::write(&staged, token).await {
        warn!(error = %e, "Failed to persist user auth token");
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600)).await;
    }
    if let Err(e) = fs::rename(&staged, &path).await {
        warn!(error = %e, "Failed to persist user auth token");
    }
}

/// Paths that must never hold the user auth token, since the agent and the
/// server sidecars' repo code can reach them.
fn exposed_user_token_paths(config: &Config) -> [std::path::PathBuf; 2] {
    [
        std::path::Path::new(&config.workspace_dir)
            .join(".moatless")
            .join(".user_auth_token"),
        std::path::Path::new(&config.data_dir).join(".user_auth_token"),
    ]
}

/// Drop any user auth token sitting somewhere it can be read. Best-effort.
async fn purge_exposed_user_tokens(config: &Config) {
    for path in exposed_user_token_paths(config) {
        match fs::remove_file(&path).await {
            Ok(()) => info!(path = %path.display(), "Removed exposed user auth token"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                warn!(error = %e, path = %path.display(), "Failed to remove exposed user auth token");
            }
        }
    }
}

/// Seed Claude's user config so the interactive TUI (tmux transport) skips
/// first-run onboarding.
///
/// Must write `$CLAUDE_CONFIG_DIR/.claude.json`, not `$HOME/.claude.json`.
/// Seeds both `hasCompletedOnboarding` and a default `theme`.
async fn write_onboarding_flag(claude_config_dir: &str) {
    let flag_path = format!("{claude_config_dir}/.claude.json");
    if fs::metadata(&flag_path).await.is_ok() {
        return; // Claude already wrote its config — don't clobber it.
    }

    if let Err(e) = fs::create_dir_all(claude_config_dir).await {
        warn!(error = %e, "Failed to create Claude config dir for onboarding flag");
        return;
    }

    if let Err(e) = fs::write(
        &flag_path,
        r#"{"hasCompletedOnboarding":true,"theme":"dark"}"#,
    )
    .await
    {
        warn!(error = %e, "Failed to write onboarding flag");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::{
        ANTHROPIC_BASE_URL, MAX_RETRY_DELAY, PersistMode, RETRY_BUDGET, agent_credentials,
        exposed_user_token_paths, fallback_tokens, filter_agent_credentials,
        gateway_api_key_helper, is_auth_rejection, persist_agent_credentials,
        persist_user_auth_token, purge_exposed_user_tokens, terminal_rejection_reason,
    };
    use crate::config::{Config, test_support::minimal_config};

    /// Write a persisted token where `Config::persisted_user_token_path` reads
    /// it.
    fn write_persisted_token(config: &Config, token: &str) {
        let path = config.persisted_user_token_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, token).unwrap();
    }

    #[test]
    fn a_stale_persisted_token_does_not_mask_a_freshly_minted_env_token() {
        // Docker mints a new USER_AUTH_TOKEN for every container it creates, so
        // a persisted file that survived can be older than the env token. Being
        // tried first must not be the last word.
        let tmp = TempDir::new().unwrap();
        let mut config = test_config(&tmp);
        config.user_auth_token = Some("freshly-minted-env".to_string());
        config.bootstrap_token = Some("bootstrap".to_string());
        write_persisted_token(&config, "expired-from-previous-container\n");

        assert_eq!(
            config.resolve_initial_auth_token().as_deref(),
            Some("expired-from-previous-container"),
        );
        assert_eq!(
            fallback_tokens(&config, PersistMode::Initial),
            vec!["freshly-minted-env".to_string(), "bootstrap".to_string()],
            "the env and bootstrap tokens must remain reachable after a rejection"
        );
    }

    #[test]
    fn a_refresh_never_falls_back_off_its_working_token() {
        let tmp = TempDir::new().unwrap();
        let mut config = test_config(&tmp);
        config.user_auth_token = Some("env".to_string());
        config.bootstrap_token = Some("bootstrap".to_string());
        write_persisted_token(&config, "persisted");

        assert!(fallback_tokens(&config, PersistMode::Refresh).is_empty());
    }

    #[test]
    fn auth_rejection_is_recognised_only_on_initial_unauthenticated() {
        use tonic::Code;

        assert!(is_auth_rejection(
            PersistMode::Initial,
            Code::Unauthenticated,
        ));

        // Non-auth failures (e.g. backend unreachable) are ordinary retries.
        assert!(!is_auth_rejection(PersistMode::Initial, Code::Unavailable));

        // A refresh keeps the token the sandbox already authenticated with.
        assert!(!is_auth_rejection(
            PersistMode::Refresh,
            Code::Unauthenticated,
        ));
    }

    /// The backend refuses with `PermissionDenied` a task the sandbox's user
    /// can't control.
    ///
    /// That's a decision, not a blip: retrying burns the startup-probe budget,
    /// and a restart presents the same identity. Terminal in either mode.
    #[test]
    fn permission_denied_is_terminal_in_both_modes() {
        use tonic::Code;

        for mode in [PersistMode::Initial, PersistMode::Refresh] {
            assert!(
                terminal_rejection_reason(mode, Code::PermissionDenied)
                    .is_some_and(|r| r.contains("not authorized")),
                "expected terminal for {mode:?}"
            );
        }
    }

    /// The startup probe kills the container at `K8S_POD_STARTUP_TIMEOUT_SECS /
    /// 5 + 6` failures × 5s — 150s at the default, ending a longer-lived retry
    /// loop mid-backoff with no reported failure.
    ///
    /// Assert the whole schedule fits with room to spare.
    #[test]
    fn the_retry_schedule_finishes_before_the_startup_probe_gives_up() {
        const PROBE_PERIOD_SECS: u64 = 5;
        const DEFAULT_STARTUP_TIMEOUT_SECS: u64 = 120;
        let smallest_probe_window = Duration::from_secs(
            (DEFAULT_STARTUP_TIMEOUT_SECS / PROBE_PERIOD_SECS + 6) * PROBE_PERIOD_SECS,
        );

        let mut elapsed = Duration::ZERO;
        let mut delay = Duration::from_secs(3);
        let mut attempts = 1;
        while elapsed + delay < RETRY_BUDGET {
            elapsed += delay;
            delay = (delay * 2).min(MAX_RETRY_DELAY);
            attempts += 1;
        }

        assert!(elapsed < RETRY_BUDGET, "backoff exceeded its own budget");
        assert!(
            RETRY_BUDGET < smallest_probe_window,
            "the loop must give up and report before the probe kills the container"
        );
        assert!(attempts > 1, "the loop must still retry a transient blip");
    }

    #[tokio::test]
    async fn persisting_the_token_purges_copies_the_agent_could_read() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);

        for stale in exposed_user_token_paths(&config) {
            tokio::fs::create_dir_all(stale.parent().unwrap())
                .await
                .unwrap();
            tokio::fs::write(&stale, "token-from-another-container")
                .await
                .unwrap();
        }

        purge_exposed_user_tokens(&config).await;

        for stale in exposed_user_token_paths(&config) {
            assert!(
                !stale.exists(),
                "{} is reachable by the agent or the server sidecars",
                stale.display()
            );
        }
        // Absent is the normal case and must stay quiet.
        purge_exposed_user_tokens(&config).await;
    }

    #[tokio::test]
    async fn the_persisted_token_lands_outside_every_exposed_directory() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);

        persist_user_auth_token(&config, "live-token").await;

        let path = config.persisted_user_token_path();
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "live-token"
        );
        assert!(
            !exposed_user_token_paths(&config).contains(&path),
            "the live token must not sit where the purge sweeps"
        );
        assert!(!path.starts_with(&config.workspace_dir));
        assert!(!path.starts_with(&config.data_dir));
    }

    fn test_config(tmp: &TempDir) -> Config {
        let home_dir = tmp.path().join("home");
        let codex_home_dir = home_dir.join(".codex");
        let claude_config_dir = home_dir.join(".claude");

        Config {
            sandbox_id: "task-1".to_string(),
            task_id: "task-1".to_string(),
            workspace_dir: tmp.path().join("workspace").display().to_string(),
            repo_dir: tmp.path().join("workspace").display().to_string(),
            data_dir: tmp.path().join("data").display().to_string(),
            runtime_dir: tmp.path().join("run").display().to_string(),
            upload_dir: tmp.path().join("uploads").display().to_string(),
            home_dir: home_dir.display().to_string(),
            codex_home_dir: codex_home_dir.display().to_string(),
            claude_config_dir: claude_config_dir.display().to_string(),
            ..minimal_config()
        }
    }

    /// A login a pre-gateway pod wrote must not outlive it on a reused volume.
    #[tokio::test]
    async fn only_an_initial_persist_removes_leftover_model_logins() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let logins = [
            tmp.path().join("home/.claude/.credentials"),
            tmp.path().join("home/.codex/auth.json"),
        ];
        for login in &logins {
            std::fs::create_dir_all(login.parent().unwrap()).unwrap();
            std::fs::write(login, "{}").unwrap();
        }

        persist_agent_credentials(&config, PersistMode::Refresh).await;
        assert!(logins.iter().all(|login| login.exists()));

        persist_agent_credentials(&config, PersistMode::Initial).await;
        assert!(logins.iter().all(|login| !login.exists()));
        assert!(tmp.path().join("home/.claude/.claude.json").exists());
    }

    #[test]
    fn filter_agent_credentials_strips_git_tokens() {
        let mut credentials = HashMap::new();
        credentials.insert("GITHUB_TOKEN".to_string(), "gh".to_string());
        credentials.insert("GH_TOKEN".to_string(), "gh2".to_string());
        credentials.insert("GIT_TOKEN".to_string(), "git".to_string());
        credentials.insert("GITLAB_TOKEN".to_string(), "gl".to_string());
        credentials.insert("ANTHROPIC_API_KEY".to_string(), "anth".to_string());
        credentials.insert("CLAUDE_CODE_OAUTH_TOKEN".to_string(), "claude".to_string());

        let filtered = filter_agent_credentials(&credentials);

        assert!(!filtered.contains_key("GITHUB_TOKEN"));
        assert!(!filtered.contains_key("GH_TOKEN"));
        assert!(!filtered.contains_key("GIT_TOKEN"));
        assert!(!filtered.contains_key("GITLAB_TOKEN"));
        assert_eq!(filtered.get("ANTHROPIC_API_KEY"), Some(&"anth".to_string()));
        assert_eq!(
            filtered.get("CLAUDE_CODE_OAUTH_TOKEN"),
            Some(&"claude".to_string())
        );
    }

    #[test]
    fn a_gateway_routed_claude_calls_the_backend_with_no_key_of_its_own() {
        let credentials = HashMap::from([
            ("ANTHROPIC_API_KEY".to_string(), "sk-user".to_string()),
            ("ANTHROPIC_AUTH_TOKEN".to_string(), "hub".to_string()),
        ]);

        let routed = agent_credentials(&credentials, Some("http://backend:8080/"));

        assert_eq!(
            routed.get("ANTHROPIC_BASE_URL").map(String::as_str),
            Some("http://backend:8080/llm/anthropic")
        );
        assert_eq!(
            routed.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("")
        );
        assert_eq!(
            routed.get("ANTHROPIC_AUTH_TOKEN").map(String::as_str),
            Some("")
        );
    }

    #[test]
    fn a_gateway_routed_claude_reads_the_task_token_through_its_key_helper() {
        let tmp = TempDir::new().unwrap();
        let config = Config {
            backend_api_url: Some("http://backend:8080".to_string()),
            runtime_dir: "/run/moat's".to_string(),
            ..test_config(&tmp)
        };
        let env = |base_url: &str| {
            HashMap::from([(ANTHROPIC_BASE_URL.to_string(), base_url.to_string())])
        };

        assert_eq!(
            gateway_api_key_helper(&config, Some(&env("http://backend:8080/llm/anthropic"))),
            Some(r"cat '/run/moat'\''s/.user_auth_token'".to_string())
        );
        assert_eq!(
            gateway_api_key_helper(&config, Some(&env("https://api.anthropic.com"))),
            None
        );
        assert_eq!(gateway_api_key_helper(&config, Some(&HashMap::new())), None);
    }

    #[test]
    fn filter_agent_credentials_noop_when_no_git_tokens() {
        let mut credentials = HashMap::new();
        credentials.insert("OPENAI_API_KEY".to_string(), "oai".to_string());

        let filtered = filter_agent_credentials(&credentials);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered.get("OPENAI_API_KEY"), Some(&"oai".to_string()));
    }
}
