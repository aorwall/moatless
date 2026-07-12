//! Credential-recovery retries: when a turn fails because a credential went
//! stale, pull fresh values from the backend and re-drive the same prompt.

use moatless_events::AgentSubType;
use tokio::time::Instant;
use tracing::{info, warn};

use crate::agent::agent_state::AgentState;
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::credentials::recovery::CredentialFailureDetector;
use crate::credentials::Dispatcher;
use crate::events::GrpcEventPublisher;

use super::retry::{compute_backoff, RetryState};

/// Cap on how many times one prompt can be re-driven by a credential-recovery
/// retry — distinct from `MAX_RETRIES` which covers transient API errors.
/// Two retries is enough for the canonical race (winner + 1 loser per refresh
/// cycle); more usually means the credential itself is invalid.
const MAX_CREDENTIAL_RECOVERY_RETRIES: u32 = 2;

/// Drive a credential-recovery retry: take any flagged keys, pull fresh
/// values from the backend, apply via dispatcher, and (on success) schedule
/// the same prompt for retry. Caps at [`MAX_CREDENTIAL_RECOVERY_RETRIES`] per
/// prompt so a permanently invalid credential doesn't loop forever — after
/// that a user-facing error is emitted.
#[allow(clippy::too_many_arguments)] // reason: actor internals; mirrors existing retry helpers
pub(super) async fn try_credential_recovery_retry(
    failures: &CredentialFailureDetector,
    dispatcher: &Dispatcher,
    backend_client: &dyn BackendClient,
    config: &Config,
    task_id: &str,
    last_prompt: &Option<String>,
    last_model: &Option<String>,
    last_agent_type: &Option<String>,
    last_agent_mode: &Option<String>,
    retry_state: &mut Option<RetryState>,
    credential_recovery_attempts: &mut u32,
    consecutive_errors: &mut u32,
    agent_state: &AgentState,
    events: &GrpcEventPublisher,
) {
    let flagged = failures.take_all();
    if flagged.is_empty() {
        return;
    }

    let Some(prompt) = last_prompt.clone() else {
        warn!(
            ?flagged,
            "Credential failure detected but no prompt to retry"
        );
        return;
    };

    if *credential_recovery_attempts >= MAX_CREDENTIAL_RECOVERY_RETRIES {
        warn!(
            ?flagged,
            attempts = *credential_recovery_attempts,
            "Credential recovery exhausted — giving up"
        );
        let provider_hint = flagged
            .iter()
            .find_map(|k| match k.as_str() {
                "CODEX_AUTH_JSON" => Some("Codex"),
                "CLAUDE_CODE_OAUTH_TOKEN" => Some("Claude"),
                "GITHUB_TOKEN" => Some("GitHub"),
                _ => None,
            })
            .unwrap_or("provider");
        let user_msg = format!("Sign-in expired. Please reconnect {provider_hint} in Settings.");
        agent_state.set_error(user_msg.clone()).await;
        *consecutive_errors += 1;
        events
            .publish_agent_event(
                task_id,
                AgentSubType::Error,
                serde_json::json!({
                    "error": user_msg,
                    "provider": provider_hint,
                    "credential_recovery_exhausted": true,
                }),
            )
            .await;
        return;
    }

    let recovered =
        try_recover_credentials(backend_client, dispatcher, task_id, &flagged, config).await;
    if recovered.is_empty() {
        // Either the backend has nothing fresher than the on-disk value, or
        // the dispatcher rejected every value. Retrying just re-fails — go
        // straight to the user-facing error so the user can reconnect.
        let provider_hint = flagged
            .iter()
            .find_map(|k| match k.as_str() {
                "CODEX_AUTH_JSON" => Some("Codex"),
                "CLAUDE_CODE_OAUTH_TOKEN" => Some("Claude"),
                "GITHUB_TOKEN" => Some("GitHub"),
                _ => None,
            })
            .unwrap_or("provider");
        let user_msg = format!("Sign-in expired. Please reconnect {provider_hint} in Settings.");
        agent_state.set_error(user_msg.clone()).await;
        *consecutive_errors += 1;
        events
            .publish_agent_event(
                task_id,
                AgentSubType::Error,
                serde_json::json!({
                    "error": user_msg,
                    "provider": provider_hint,
                    "credential_recovery_exhausted": true,
                    "reason": "no_fresher_credential_available",
                }),
            )
            .await;
        return;
    }

    *credential_recovery_attempts += 1;
    // Undo the consecutive_errors increment caused by this failure since we're
    // about to retry. The retry path will increment again on its own failure.
    *consecutive_errors = consecutive_errors.saturating_sub(1);

    let backoff = compute_backoff(0);
    info!(
        ?recovered,
        backoff_secs = backoff.as_secs_f64(),
        attempt = *credential_recovery_attempts,
        "Scheduling credential-recovery retry"
    );

    *retry_state = Some(RetryState {
        prompt,
        model: last_model.clone(),
        agent_type: last_agent_type.clone(),
        agent_mode: last_agent_mode.clone(),
        attempt: 0,
        retry_at: Instant::now() + backoff,
    });
    events
        .publish_agent_event(
            task_id,
            AgentSubType::Retrying,
            serde_json::json!({
                "attempt": *credential_recovery_attempts,
                "max_retries": MAX_CREDENTIAL_RECOVERY_RETRIES,
                "backoff_secs": backoff.as_secs_f64(),
                "reason": "credential_recovery",
                "recovered_keys": recovered,
            }),
        )
        .await;
}

/// Pull fresh credentials from the backend and apply each flagged key through
/// the dispatcher. Returns the list of keys that were successfully refreshed
/// AND whose value differs from what's already on disk — pulling the same
/// (broken) value back and retrying just produces the same failure.
///
/// For `CODEX_AUTH_JSON` the on-disk file at `~/.codex/auth.json` is the
/// authoritative pre-failure value (codex writes it in-place after every
/// refresh). If the backend returns the same JSON, no other sandbox has
/// pushed a rotation back yet — recovery is futile and we should surface
/// the user-facing error instead of burning retries.
async fn try_recover_credentials(
    backend_client: &dyn BackendClient,
    dispatcher: &Dispatcher,
    task_id: &str,
    flagged_keys: &[String],
    config: &Config,
) -> Vec<String> {
    if flagged_keys.is_empty() {
        return vec![];
    }

    info!(
        ?flagged_keys,
        task_id, "Attempting credential recovery from backend"
    );

    let resp = match backend_client.get_credentials(task_id).await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "Credential recovery: get_credentials failed");
            return vec![];
        }
    };

    let mut recovered = Vec::new();
    for key in flagged_keys {
        let Some(new_value) = resp.credentials.get(key) else {
            warn!(key, "Credential recovery: backend did not return key");
            continue;
        };

        if !value_actually_changed(key, new_value, config).await {
            warn!(
                key,
                "Credential recovery: backend value unchanged from on-disk; skipping retry"
            );
            continue;
        }

        match dispatcher.dispatch(key, new_value).await {
            Ok(true) => recovered.push(key.clone()),
            Ok(false) => warn!(key, "Credential recovery: no sink for key"),
            Err(e) => warn!(error = %e, key, "Credential recovery: sink failed"),
        }
    }
    recovered
}

/// Compare a backend-returned credential value against the sandbox's current
/// on-disk / in-process value. Returns `true` only when the new value is
/// genuinely fresher than what the failed turn was just using.
///
/// Best-effort: when we can't determine the prior value (unknown key, file
/// missing, parse error) we conservatively return `true` so the retry path
/// still fires.
async fn value_actually_changed(key: &str, new_value: &str, config: &Config) -> bool {
    match key {
        "CODEX_AUTH_JSON" => {
            let path = format!("{}/auth.json", config.codex_home_dir);
            let current = match tokio::fs::read_to_string(&path).await {
                Ok(s) => s,
                Err(_) => return true,
            };
            // Compare via JSON canonicalization so whitespace differences
            // don't fool the dedup.
            match (
                serde_json::from_str::<serde_json::Value>(&current),
                serde_json::from_str::<serde_json::Value>(new_value),
            ) {
                (Ok(a), Ok(b)) => a != b,
                _ => current.trim() != new_value.trim(),
            }
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config_with_codex_dir(codex_home_dir: String) -> Config {
        use crate::config::{AgentConfig, ProxyConfig};
        Config {
            port: 0,
            grpc_port: 0,
            task_id: "task-test".to_string(),
            workspace_dir: "/tmp/workspace".to_string(),
            repo_dir: "/tmp/workspace".to_string(),
            data_dir: "/tmp/data".to_string(),
            upload_dir: "/tmp/uploads".to_string(),
            log_level: "info".to_string(),
            backend_grpc_url: "http://localhost:50051".to_string(),
            backend_api_url: None,
            user_auth_token: None,
            bootstrap_token: None,
            agent_config: AgentConfig::default(),
            claude_cli_path: "/usr/bin/claude".to_string(),
            codex_cli_path: "/usr/bin/codex".to_string(),
            proxy: ProxyConfig::default(),
            servers: vec![],
            frontend_url: None,
            default_model: None,
            default_codex_model: None,
            use_tmux: false,
            permission_mode: claude_agent_sdk::PermissionMode::BypassPermissions,
            workspace_sync_enabled: false,
            home_dir: "/tmp/home".to_string(),
            codex_home_dir,
            claude_config_dir: "/tmp/home/.claude".to_string(),
            git_clone_url: None,
            git_branch: None,
            git_provider: "github".to_string(),
            repo_config_raw: None,
            setup_commands: vec![],
            plugin_ids: vec![],
            plugin_repos: vec![],
        }
    }

    #[tokio::test]
    async fn value_actually_changed_returns_false_for_identical_codex_auth() {
        let tmp = tempfile::TempDir::new().unwrap();
        let codex_dir = tmp.path().join(".codex");
        tokio::fs::create_dir_all(&codex_dir).await.unwrap();
        let on_disk = r#"{"tokens":{"refresh_token":"reused"}}"#;
        tokio::fs::write(codex_dir.join("auth.json"), on_disk)
            .await
            .unwrap();

        let config = test_config_with_codex_dir(codex_dir.display().to_string());

        // Backend returns the same value (whitespace-different) — should
        // still detect as unchanged.
        let backend_value = r#"{"tokens": {"refresh_token": "reused"}}"#;
        let changed = value_actually_changed("CODEX_AUTH_JSON", backend_value, &config).await;
        assert!(
            !changed,
            "identical (canonicalized) JSON should not count as changed"
        );
    }

    #[tokio::test]
    async fn value_actually_changed_returns_true_for_rotated_codex_auth() {
        let tmp = tempfile::TempDir::new().unwrap();
        let codex_dir = tmp.path().join(".codex");
        tokio::fs::create_dir_all(&codex_dir).await.unwrap();
        let on_disk = r#"{"tokens":{"refresh_token":"old"}}"#;
        tokio::fs::write(codex_dir.join("auth.json"), on_disk)
            .await
            .unwrap();

        let config = test_config_with_codex_dir(codex_dir.display().to_string());

        let backend_value = r#"{"tokens":{"refresh_token":"new"}}"#;
        let changed = value_actually_changed("CODEX_AUTH_JSON", backend_value, &config).await;
        assert!(changed, "different refresh_token should count as changed");
    }

    #[tokio::test]
    async fn value_actually_changed_returns_true_when_no_local_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Note: no auth.json written — file missing.
        let config = test_config_with_codex_dir(tmp.path().join(".codex").display().to_string());

        let changed = value_actually_changed("CODEX_AUTH_JSON", r#"{"tokens":{}}"#, &config).await;
        assert!(changed, "missing on-disk file should fall through to retry");
    }

    #[tokio::test]
    async fn value_actually_changed_returns_true_for_unknown_keys() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = test_config_with_codex_dir(tmp.path().join(".codex").display().to_string());
        // Unknown keys conservatively retry.
        let changed = value_actually_changed("WHATEVER", "value", &config).await;
        assert!(changed);
    }
}
