// Safety: set_var is unsafe in multi-threaded context. This module is called
// during single-threaded init *and* then periodically by the credential
// refresher loop (see `super::refresher`). There are no other writers of these
// env vars after init — the refresher is a single serialized loop.
#![allow(unsafe_code)]

//! Credential fetching and application.
//!
//! Fetches credentials from the backend via gRPC `GetCredentials` RPC,
//! sets them as environment variables, and writes credential files
//! needed by the Claude CLI.

use std::collections::HashMap;
use std::time::Duration;

use tokio::fs;
use tracing::{info, warn};

use super::gh_hosts;
use super::server_env;
use crate::config::Config;
use crate::state::SandboxState;

/// Keys that must NOT be set in the sandbox process environment via `set_var`.
///
/// `gh` (and `gh auth git-credential`, which git uses for push) checks env vars
/// before `~/.config/gh/hosts.yml`. Setting these here would freeze a stale
/// value into every subprocess spawned for the lifetime of the pod. Auth for
/// these credentials is handled exclusively via `~/.config/gh/hosts.yml`, which
/// the credential refresher rewrites on every cycle.
const ENV_SET_BLOCKLIST: &[&str] = &["GITHUB_TOKEN", "GH_TOKEN"];

/// Keys that must NOT be inherited by the agent subprocess.
///
/// `GIT_TOKEN` is still set in the sandbox process env (for init-time git clone)
/// but must not reach the long-lived agent subprocess for the same staleness
/// reason as the keys above.
const AGENT_EXCLUDED_KEYS: &[&str] = &["GITHUB_TOKEN", "GH_TOKEN", "GIT_TOKEN"];

/// Produce the credential map passed to the agent subprocess env.
///
/// Strips git-provider tokens that would otherwise freeze to whatever value was
/// current when the agent subprocess was spawned.
pub fn filter_agent_credentials(credentials: &HashMap<String, String>) -> HashMap<String, String> {
    credentials
        .iter()
        .filter(|(k, _)| !AGENT_EXCLUDED_KEYS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Whether `fetch_and_apply_credentials` is being called for the first time
/// (during sandbox init) or by the periodic refresher.
///
/// On `Refresh`, Codex's `auth.json` is left alone — the watcher in
/// `codex_watcher` is the source of truth for that file post-startup, and a
/// refresher that re-wrote a stale blob from `provider_tokens` would clobber
/// any rotation that happened locally.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PersistMode {
    Initial,
    Refresh,
}

/// Fetch credentials from backend and apply them.
///
/// Retries with exponential backoff (3s, 6s, 12s... up to 10 attempts).
/// The backend may not be routable when the sandbox pod first starts.
pub async fn fetch_and_apply_credentials(state: &SandboxState) -> Result<(), String> {
    fetch_and_apply_credentials_with_mode(state, PersistMode::Initial).await
}

/// Like [`fetch_and_apply_credentials`] but lets the caller declare whether
/// this is initial init or a periodic refresh, so write-once-only files (like
/// Codex `auth.json`) can be skipped when appropriate.
pub async fn fetch_and_apply_credentials_with_mode(
    state: &SandboxState,
    mode: PersistMode,
) -> Result<(), String> {
    let backend_client = &state.backend_client;
    let task_id = &state.config.task_id;
    let max_retries = 10;
    let mut delay = Duration::from_secs(3);

    for attempt in 1..=max_retries {
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

                // Set each credential as an env var for the sandbox process itself
                // (init-time git clone, `gh` invocations, etc.).
                // GITHUB_TOKEN and GH_TOKEN are skipped — see ENV_SET_BLOCKLIST.
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
                // so git.rs can use a single env var for clone authentication.
                let git_provider = state.config.git_provider.as_str();
                let provider_token_key = match git_provider {
                    "gitlab" => "GITLAB_TOKEN",
                    "gitness" => "GITNESS_TOKEN",
                    _ => "GITHUB_TOKEN", // default to GitHub
                };
                if let Some(token) = resp.credentials.get(provider_token_key).cloned() {
                    resp.credentials
                        .insert("GIT_TOKEN".to_string(), token.clone());
                    // SAFETY: see note at top of this file.
                    unsafe {
                        std::env::set_var("GIT_TOKEN", &token);
                    }
                }

                // Alias GITHUB_TOKEN → GH_TOKEN in the credentials map for
                // filter_agent_credentials to strip. Neither is set in the process
                // env — see ENV_SET_BLOCKLIST above.
                if let Some(token) = resp.credentials.get("GITHUB_TOKEN").cloned() {
                    resp.credentials
                        .insert("GH_TOKEN".to_string(), token.clone());

                    // Write into `~/.config/gh/hosts.yml` — the sole source of truth
                    // for GitHub auth inside this pod.
                    if let Err(e) =
                        gh_hosts::write_github_token(&state.config.home_dir, &token).await
                    {
                        warn!(error = %e, "Failed to refresh gh hosts.yml");
                    }
                }

                // Store credentials in state for passing to agent subprocess.
                // Filter out git-provider tokens so the long-running Claude CLI
                // subprocess doesn't snapshot a stale value at spawn time.
                *state.credentials.write().await = filter_agent_credentials(&resp.credentials);

                persist_agent_credentials(&state.config, &resp.credentials, mode).await;

                server_env::write_server_env_file(&state.config, &resp.server_env).await;

                return Ok(());
            }
            Err(e) => {
                if attempt == max_retries {
                    return Err(format!("Failed after {max_retries} attempts: {e}"));
                }
                warn!(
                    attempt,
                    max_retries,
                    error = %e,
                    delay_secs = delay.as_secs(),
                    "Credential fetch failed, retrying"
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(60));
            }
        }
    }

    unreachable!()
}

/// Persist credential files used by local agent runtimes.
///
/// On `PersistMode::Refresh`, Codex's `auth.json` is intentionally left alone.
/// Codex rotates its OAuth refresh token on every refresh; the in-pod watcher
/// (`codex_watcher`) is the source of truth for that file post-startup. A
/// refresher that re-wrote a stale blob from `provider_tokens` would actively
/// undo rotation and cause `refresh_token_reused` failures.
pub async fn persist_agent_credentials(
    config: &Config,
    credentials: &std::collections::HashMap<String, String>,
    mode: PersistMode,
) {
    if let Some(oauth_token) = credentials.get("CLAUDE_CODE_OAUTH_TOKEN") {
        write_claude_credentials(&config.claude_config_dir, oauth_token).await;
    }

    let wrote_codex_auth = if mode == PersistMode::Initial {
        if let Some(auth_json) = credentials.get("CODEX_AUTH_JSON") {
            write_codex_auth_json(&config.codex_home_dir, auth_json).await
        } else {
            false
        }
    } else {
        // Refresh path: skip Codex entirely so we don't clobber a locally-rotated
        // auth.json with the (now stale) backend copy.
        false
    };

    if mode == PersistMode::Initial && !wrote_codex_auth {
        if let Some(api_key) = credentials.get("OPENAI_API_KEY") {
            write_codex_auth_from_api_key(&config.codex_home_dir, api_key).await;
        }
    }

    write_onboarding_flag(&config.claude_config_dir).await;
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
            // Set file permissions to 600
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
/// Claude reads `$CLAUDE_CONFIG_DIR/.claude.json` — NOT `$HOME/.claude.json`.
/// Writing the flag to `$HOME` (the prior behavior) had no effect, so the TUI
/// blocked on the theme picker and `SessionStart` never fired (the subprocess
/// `-p` transport is unaffected because `--print` skips onboarding entirely).
/// We seed both `hasCompletedOnboarding` and a default `theme` so the picker is
/// skipped on a cold config.
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

    use tempfile::TempDir;

    use super::{filter_agent_credentials, persist_agent_credentials, PersistMode};
    use crate::config::{AgentConfig, Config, ProxyConfig};

    fn test_config(tmp: &TempDir) -> Config {
        let home_dir = tmp.path().join("home");
        let codex_home_dir = home_dir.join(".codex");
        let claude_config_dir = home_dir.join(".claude");

        Config {
            port: 8001,
            grpc_port: 50052,
            task_id: "task-1".to_string(),
            workspace_dir: tmp.path().join("workspace").display().to_string(),
            data_dir: tmp.path().join("data").display().to_string(),
            upload_dir: tmp.path().join("uploads").display().to_string(),
            log_level: "info".to_string(),
            backend_grpc_url: "http://backend:50051".to_string(),
            backend_api_url: None,
            user_auth_token: None,
            agent_config: AgentConfig::default(),
            claude_cli_path: "/usr/local/bin/claude".to_string(),
            codex_cli_path: "/usr/local/bin/codex".to_string(),
            proxy: ProxyConfig::default(),
            servers: Vec::new(),
            frontend_url: None,
            use_tmux: false,
            workspace_sync_enabled: false,
            default_model: None,
            default_codex_model: None,
            home_dir: home_dir.display().to_string(),
            codex_home_dir: codex_home_dir.display().to_string(),
            claude_config_dir: claude_config_dir.display().to_string(),
            git_clone_url: None,
            git_branch: None,
            git_provider: "github".to_string(),
            repo_config_raw: None,
            setup_commands: Vec::new(),
            plugin_ids: Vec::new(),
            plugin_repos: Vec::new(),
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
        credentials.insert("ANTHROPIC_API_KEY".to_string(), "anth".to_string());
        credentials.insert("CLAUDE_CODE_OAUTH_TOKEN".to_string(), "claude".to_string());

        let filtered = filter_agent_credentials(&credentials);

        assert!(!filtered.contains_key("GITHUB_TOKEN"));
        assert!(!filtered.contains_key("GH_TOKEN"));
        assert!(!filtered.contains_key("GIT_TOKEN"));
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
