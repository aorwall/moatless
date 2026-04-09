// Safety: set_var is unsafe in multi-threaded context but this module runs
// during single-threaded init phase before any agent execution starts.
#![allow(unsafe_code)]

//! Credential fetching and application.
//!
//! Fetches credentials from the backend via gRPC `GetCredentials` RPC,
//! sets them as environment variables, and writes credential files
//! needed by the Claude CLI.

use std::time::Duration;

use tokio::fs;
use tracing::{info, warn};

use crate::config::Config;
use crate::state::SandboxState;

/// Fetch credentials from backend and apply them.
///
/// Retries with exponential backoff (3s, 6s, 12s... up to 10 attempts).
/// The backend may not be routable when the sandbox pod first starts.
pub async fn fetch_and_apply_credentials(state: &SandboxState) -> Result<(), String> {
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

                // Set each credential as an env var
                for (key, value) in &resp.credentials {
                    // SAFETY: called during single-threaded init phase before
                    // any agent tasks or async workers are spawned.
                    unsafe {
                        std::env::set_var(key, value);
                    }
                }

                // Map provider-specific token to GIT_TOKEN based on GIT_PROVIDER
                // so git.rs can use a single env var for clone authentication.
                // Skip if GIT_TOKEN is already set (e.g. injected by pod builder).
                if std::env::var("GIT_TOKEN").is_err() {
                    let git_provider = state.config.git_provider.as_str();
                    let provider_token_key = match git_provider {
                        "gitlab" => "GITLAB_TOKEN",
                        "gitness" => "GITNESS_TOKEN",
                        _ => "GITHUB_TOKEN", // default to GitHub
                    };
                    if let Some(token) = resp.credentials.get(provider_token_key).cloned() {
                        resp.credentials
                            .entry("GIT_TOKEN".to_string())
                            .or_insert_with(|| {
                                // SAFETY: called during single-threaded init phase.
                                unsafe {
                                    std::env::set_var("GIT_TOKEN", &token);
                                }
                                token.clone()
                            });
                    }
                }

                // Alias GITHUB_TOKEN → GH_TOKEN for `gh` CLI compatibility
                if let Some(token) = resp.credentials.get("GITHUB_TOKEN").cloned() {
                    resp.credentials
                        .entry("GH_TOKEN".to_string())
                        .or_insert_with(|| {
                            // SAFETY: called during single-threaded init phase.
                            unsafe {
                                std::env::set_var("GH_TOKEN", &token);
                            }
                            token
                        });
                }

                // Store credentials in state for passing to agent subprocess
                *state.credentials.write().await = resp.credentials.clone();

                persist_agent_credentials(&state.config, &resp.credentials).await;

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
pub async fn persist_agent_credentials(
    config: &Config,
    credentials: &std::collections::HashMap<String, String>,
) {
    if let Some(oauth_token) = credentials.get("CLAUDE_CODE_OAUTH_TOKEN") {
        write_claude_credentials(&config.claude_config_dir, oauth_token).await;
    }

    let wrote_codex_auth = if let Some(auth_json) = credentials.get("CODEX_AUTH_JSON") {
        write_codex_auth_json(&config.codex_home_dir, auth_json).await
    } else {
        false
    };

    if !wrote_codex_auth {
        if let Some(api_key) = credentials.get("OPENAI_API_KEY") {
            write_codex_auth_from_api_key(&config.codex_home_dir, api_key).await;
        }
    }

    write_onboarding_flag(&config.home_dir).await;
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

/// Write the onboarding flag so Claude CLI skips interactive setup.
async fn write_onboarding_flag(home_dir: &str) {
    let flag_path = format!("{home_dir}/.claude.json");
    if fs::metadata(&flag_path).await.is_ok() {
        return; // Already exists
    }

    if let Err(e) = fs::write(&flag_path, r#"{"hasCompletedOnboarding":true}"#).await {
        warn!(error = %e, "Failed to write onboarding flag");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::TempDir;

    use super::persist_agent_credentials;
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
            default_model: None,
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

        persist_agent_credentials(&config, &credentials).await;

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

    #[tokio::test]
    async fn invalid_codex_auth_json_falls_back_to_openai_api_key() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let mut credentials = HashMap::new();
        credentials.insert("CODEX_AUTH_JSON".to_string(), "{invalid".to_string());
        credentials.insert("OPENAI_API_KEY".to_string(), "fallback-key".to_string());

        persist_agent_credentials(&config, &credentials).await;

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
