// Safety: set_var is unsafe in multi-threaded context. This module is called
// during single-threaded init *and* then periodically by the credential
// refresher loop (see `super::refresher`). There are no other writers of these
// env vars after init — the refresher is a single serialized loop.
#![allow(unsafe_code)]

//! Credential fetching and application.
//!
//! Fetches credentials from the backend via gRPC `GetCredentials` RPC, sets
//! them as environment variables, and writes credential files needed by the
//! Claude CLI.

use std::collections::HashMap;
use std::time::Duration;

use moatless_types::credential_key::{GH_TOKEN, USER_AUTH_TOKEN};
use tokio::fs;
use tracing::{info, warn};

use super::ssh;
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
/// behind it. `gh` reaches GitHub through `github_proxy` instead, and git takes
/// its tokens from `SandboxState::git_credentials` and the broker.
const ENV_SET_BLOCKLIST: &[&str] = &[
    "GITHUB_TOKEN",
    GH_TOKEN,
    "GITLAB_TOKEN",
    "GITNESS_TOKEN",
    GIT_TOKEN,
];

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

/// Whether `fetch_and_apply_credentials` runs at sandbox init or from the
/// periodic refresher.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PersistMode {
    Initial,
    /// An existing Codex `auth.json` is left alone — `codex_watcher` owns that
    /// file post-startup, and overwriting it from `provider_tokens` would
    /// clobber a local rotation. A missing one is written, for a Task that
    /// switched to Codex after startup.
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
/// this is initial init or a periodic refresh, so write-once-only files (like
/// Codex `auth.json`) can be skipped when appropriate.
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

                // Map provider-specific token to GIT_TOKEN based on GIT_PROVIDER
                // so git.rs has one name for the pod provider's token.
                let git_provider = state.config.git_provider.as_str();
                let provider_token_key = match git_provider {
                    "gitlab" => "GITLAB_TOKEN",
                    "gitness" => "GITNESS_TOKEN",
                    _ => "GITHUB_TOKEN", // default to GitHub
                };
                if let Some(token) = resp.credentials.get(provider_token_key).cloned() {
                    resp.credentials.insert(GIT_TOKEN.to_string(), token);
                }
                *state.git_credentials.write().await =
                    GitCredentials::from_credentials(&resp.credentials);

                // Store credentials in state for passing to agent subprocess.
                // Filter out git-provider tokens so the long-running Claude CLI
                // subprocess doesn't snapshot a stale value at spawn time.
                *state.credentials.write().await = filter_agent_credentials(&resp.credentials);

                persist_agent_credentials(&state.config, &resp.credentials, mode).await;

                // Materialize SSH private keys into ~/.ssh so git/ssh can use
                // them for SSH remotes. Delivered on a dedicated channel and
                // never set as env vars — the agent uses the key without ever
                // reading its value.
                ssh::materialize_ssh_keys(&state.config, &resp.ssh_keys).await;

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
                        "backend rejected every auth token this sandbox holds; restart \
                         the task to re-provision (credential fetch failed: {e})"
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

/// Persist credential files used by local agent runtimes.
///
/// On `PersistMode::Refresh`, Codex's `auth.json` is left alone — see
/// [`PersistMode`] for why.
pub async fn persist_agent_credentials(
    config: &Config,
    credentials: &std::collections::HashMap<String, String>,
    mode: PersistMode,
) {
    if let Some(oauth_token) = credentials.get("CLAUDE_CODE_OAUTH_TOKEN") {
        write_claude_credentials(&config.claude_config_dir, oauth_token).await;
    }

    let write_codex = mode == PersistMode::Initial
        || !tokio::fs::try_exists(format!("{}/auth.json", config.codex_home_dir))
            .await
            .unwrap_or(true);
    let wrote_codex_auth = if write_codex {
        if let Some(auth_json) = credentials.get("CODEX_AUTH_JSON") {
            write_codex_auth_json(&config.codex_home_dir, auth_json).await
        } else {
            false
        }
    } else {
        false
    };

    // A routed Codex reads its key from `OPENAI_API_KEY` directly
    // (`requires_openai_auth = false`), so it needs no auth.json. Writing one
    // would hand the proxy key to the watcher, which mirrors the file back as
    // the user's stored Codex credential.
    if write_codex && !wrote_codex_auth && config.openai_base_url.is_none() {
        if let Some(api_key) = credentials.get("OPENAI_API_KEY") {
            write_codex_auth_from_api_key(&config.codex_home_dir, api_key).await;
        }
    }

    write_onboarding_flag(&config.claude_config_dir).await;
}

/// Persist the long-lived user auth token to disk so a container restart can
/// reuse it (see [`Config::persisted_user_token_path`]).
///
/// Best-effort: a write failure just falls back to the initial token on the
/// next restart, so failures are logged, not fatal.
async fn persist_user_auth_token(config: &Config, token: &str) {
    purge_exposed_user_tokens(config).await;

    let path = config.persisted_user_token_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent).await {
            warn!(error = %e, "Failed to create dir for persisted user auth token");
            return;
        }
    }
    match fs::write(&path, token).await {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await;
            }
        }
        Err(e) => warn!(error = %e, "Failed to persist user auth token"),
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

/// Write Claude OAuth credentials file.
///
/// The Claude CLI reads `$CLAUDE_CONFIG_DIR/.credentials` for auth tokens.
async fn write_claude_credentials(claude_config_dir: &str, oauth_token: &str) {
    let creds_path = format!("{claude_config_dir}/.credentials");
    let creds_json = format!(
        r#"{{"claudeAiOauth":{{"accessToken":"{oauth_token}","refreshToken":"","expiresAt":"2099-01-01T00:00:00.000Z","scopes":["user:inference","user:profile"]}}}}"#,
    );

    match fs::write(&creds_path, &creds_json).await {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    fs::set_permissions(&creds_path, std::fs::Permissions::from_mode(0o600)).await;
            }
            info!("Wrote Claude OAuth credentials file");
        }
        Err(e) => {
            warn!(error = %e, "Failed to write Claude credentials file");
        }
    }
}

async fn write_codex_auth_json(codex_home_dir: &str, raw_auth_json: &str) -> bool {
    let parsed: serde_json::Value = match serde_json::from_str::<serde_json::Value>(raw_auth_json) {
        Ok(value) if value.is_object() => value,
        Ok(_) => {
            warn!("Codex auth JSON must be a JSON object");
            return false;
        }
        Err(e) => {
            warn!(error = %e, "Failed to parse Codex auth JSON");
            return false;
        }
    };

    write_codex_auth_file(codex_home_dir, &parsed.to_string()).await
}

/// Write Codex auth.json with an API key for non-interactive startup.
async fn write_codex_auth_from_api_key(codex_home_dir: &str, api_key: &str) {
    let auth_json = serde_json::json!({
        "OPENAI_API_KEY": api_key,
        "tokens": serde_json::Value::Null,
        "last_refresh": serde_json::Value::Null,
    });

    let _ = write_codex_auth_file(codex_home_dir, &auth_json.to_string()).await;
}

async fn write_codex_auth_file(codex_home_dir: &str, auth_json: &str) -> bool {
    let auth_path = format!("{codex_home_dir}/auth.json");

    if let Err(e) = fs::create_dir_all(codex_home_dir).await {
        warn!(error = %e, "Failed to create Codex home directory");
        return false;
    }

    match fs::write(&auth_path, auth_json).await {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    fs::set_permissions(&auth_path, std::fs::Permissions::from_mode(0o600)).await;
            }
            info!("Wrote Codex auth file");
            true
        }
        Err(e) => {
            warn!(error = %e, "Failed to write Codex auth file");
            false
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
        MAX_RETRY_DELAY, PersistMode, RETRY_BUDGET, exposed_user_token_paths, fallback_tokens,
        filter_agent_credentials, is_auth_rejection, persist_agent_credentials,
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

    #[tokio::test]
    async fn codex_auth_json_takes_precedence_over_openai_api_key() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let mut credentials = HashMap::new();
        credentials.insert(
            "CODEX_AUTH_JSON".to_string(),
            r#"{"tokens":{"access_token":"abc"},"OPENAI_API_KEY":"from-json"}"#.to_string(),
        );
        credentials.insert("OPENAI_API_KEY".to_string(), "fallback-key".to_string());

        persist_agent_credentials(&config, &credentials, PersistMode::Initial).await;

        let auth_path = tmp.path().join("home/.codex/auth.json");
        let auth_json = tokio::fs::read_to_string(auth_path).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&auth_json).unwrap();

        assert_eq!(
            parsed
                .get("tokens")
                .and_then(|value| value.get("access_token"))
                .and_then(serde_json::Value::as_str),
            Some("abc")
        );
        assert_eq!(
            parsed
                .get("OPENAI_API_KEY")
                .and_then(serde_json::Value::as_str),
            Some("from-json")
        );
    }

    #[test]
    fn filter_agent_credentials_strips_git_tokens() {
        let mut credentials = HashMap::new();
        credentials.insert("GITHUB_TOKEN".to_string(), "gh".to_string());
        credentials.insert("GH_TOKEN".to_string(), "gh2".to_string());
        credentials.insert("GIT_TOKEN".to_string(), "git".to_string());
        credentials.insert("GITLAB_TOKEN".to_string(), "gl".to_string());
        credentials.insert("GITNESS_TOKEN".to_string(), "gn".to_string());
        credentials.insert("ANTHROPIC_API_KEY".to_string(), "anth".to_string());
        credentials.insert("CLAUDE_CODE_OAUTH_TOKEN".to_string(), "claude".to_string());

        let filtered = filter_agent_credentials(&credentials);

        assert!(!filtered.contains_key("GITHUB_TOKEN"));
        assert!(!filtered.contains_key("GH_TOKEN"));
        assert!(!filtered.contains_key("GIT_TOKEN"));
        assert!(!filtered.contains_key("GITLAB_TOKEN"));
        assert!(!filtered.contains_key("GITNESS_TOKEN"));
        assert_eq!(filtered.get("ANTHROPIC_API_KEY"), Some(&"anth".to_string()));
        assert_eq!(
            filtered.get("CLAUDE_CODE_OAUTH_TOKEN"),
            Some(&"claude".to_string())
        );
    }

    #[test]
    fn filter_agent_credentials_noop_when_no_git_tokens() {
        let mut credentials = HashMap::new();
        credentials.insert("OPENAI_API_KEY".to_string(), "oai".to_string());

        let filtered = filter_agent_credentials(&credentials);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered.get("OPENAI_API_KEY"), Some(&"oai".to_string()));
    }

    #[tokio::test]
    async fn refresh_mode_does_not_overwrite_existing_codex_auth() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);

        // Simulate a locally-rotated auth.json (what the codex CLI would write
        // after a refresh).
        let codex_dir = tmp.path().join("home/.codex");
        tokio::fs::create_dir_all(&codex_dir).await.unwrap();
        let auth_path = codex_dir.join("auth.json");
        let local_rotated = r#"{"tokens":{"refresh_token":"rotated-locally"}}"#;
        tokio::fs::write(&auth_path, local_rotated).await.unwrap();

        // Backend still has the old (pre-rotation) blob, so a refresher cycle
        // would overwrite the local file unless we explicitly skip it.
        let mut credentials = HashMap::new();
        credentials.insert(
            "CODEX_AUTH_JSON".to_string(),
            r#"{"tokens":{"refresh_token":"stale-from-backend"}}"#.to_string(),
        );

        persist_agent_credentials(&config, &credentials, PersistMode::Refresh).await;

        let on_disk = tokio::fs::read_to_string(&auth_path).await.unwrap();
        assert_eq!(
            on_disk, local_rotated,
            "Refresh mode must not clobber a locally-rotated auth.json"
        );
    }

    /// A Task switched to Codex after startup has no `auth.json` to protect.
    #[tokio::test]
    async fn refresh_mode_writes_a_missing_codex_auth() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let mut credentials = HashMap::new();
        let auth = r#"{"tokens":{"refresh_token":"from-backend"}}"#;
        credentials.insert("CODEX_AUTH_JSON".to_string(), auth.to_string());

        persist_agent_credentials(&config, &credentials, PersistMode::Refresh).await;

        let on_disk = tokio::fs::read_to_string(tmp.path().join("home/.codex/auth.json"))
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&on_disk).unwrap(),
            serde_json::from_str::<serde_json::Value>(auth).unwrap()
        );
    }

    #[tokio::test]
    async fn invalid_codex_auth_json_falls_back_to_openai_api_key() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let mut credentials = HashMap::new();
        credentials.insert("CODEX_AUTH_JSON".to_string(), "{invalid".to_string());
        credentials.insert("OPENAI_API_KEY".to_string(), "fallback-key".to_string());

        persist_agent_credentials(&config, &credentials, PersistMode::Initial).await;

        let auth_path = tmp.path().join("home/.codex/auth.json");
        let auth_json = tokio::fs::read_to_string(auth_path).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&auth_json).unwrap();

        assert_eq!(
            parsed
                .get("OPENAI_API_KEY")
                .and_then(serde_json::Value::as_str),
            Some("fallback-key")
        );
        assert!(parsed.get("tokens").is_some());
    }
}
