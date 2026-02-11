//! Credential fetching and application.
//!
//! Fetches credentials from the backend via gRPC `GetCredentials` RPC,
//! sets them as environment variables, and writes credential files
//! needed by the Claude CLI.

use std::time::Duration;

use tokio::fs;
use tracing::{info, warn};

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
                let credential_names: Vec<&str> = resp.credentials.keys().map(|k| k.as_str()).collect();
                info!(
                    task_id = %task_id,
                    credential_count = resp.credentials.len(),
                    ?credential_names,
                    attempt,
                    "Credentials fetched successfully"
                );

                // Set each credential as an env var
                for (key, value) in &resp.credentials {
                    // Safety: set_var is unsafe in multi-threaded context but we're
                    // in the init phase before any agent execution starts
                    unsafe {
                        std::env::set_var(key, value);
                    }
                }

                // Alias GITHUB_TOKEN → GH_TOKEN for `gh` CLI compatibility
                if let Some(token) = resp.credentials.get("GITHUB_TOKEN").cloned() {
                    resp.credentials.entry("GH_TOKEN".to_string()).or_insert_with(|| {
                        unsafe {
                            std::env::set_var("GH_TOKEN", &token);
                        }
                        token
                    });
                }

                // Store credentials in state for passing to agent subprocess
                *state.credentials.write().await = resp.credentials.clone();

                // Write Claude CLI credential files if CLAUDE_CODE_OAUTH_TOKEN is present
                if let Some(oauth_token) = resp.credentials.get("CLAUDE_CODE_OAUTH_TOKEN") {
                    write_claude_credentials(&state.config.claude_config_dir, oauth_token).await;
                }

                // Write onboarding flag
                write_onboarding_flag(&state.config.home_dir).await;

                return Ok(());
            }
            Err(e) => {
                if attempt == max_retries {
                    return Err(format!(
                        "Failed after {max_retries} attempts: {e}"
                    ));
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
                let _ = fs::set_permissions(
                    &creds_path,
                    std::fs::Permissions::from_mode(0o600),
                )
                .await;
            }
            info!("Wrote Claude OAuth credentials file");
        }
        Err(e) => {
            warn!(error = %e, "Failed to write Claude credentials file");
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
