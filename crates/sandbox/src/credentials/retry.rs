//! Credential-recovery retries: when a turn fails because a credential went
//! stale, pull fresh values from the backend and re-drive the same prompt.

use tracing::{info, warn};

use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::credentials::Dispatcher;

/// Distinct from `MAX_RETRIES`, which counts transient API error retries.
///
/// Two covers the canonical race (winner + 1 loser per refresh cycle); more
/// usually means the credential itself is invalid. `pub(crate)` so the
/// harness-path [`CoordinatorService`] shares the same cap.
pub(crate) const MAX_CREDENTIAL_RECOVERY_RETRIES: u32 = 2;

/// Returns only keys refreshed and now different from disk — retrying an
/// unchanged value would just repeat the same failure.
///
/// `pub(crate)` so the harness-path [`CoordinatorService`] reuses this pull +
/// disk-dedup + dispatch primitive.
pub(crate) async fn try_recover_credentials(
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

/// `True` only when the backend value is genuinely fresher than what's on disk
/// / in-process.
///
/// Best-effort: when the prior value is unknown (bad key, missing file, parse
/// error) this conservatively returns `true` so the retry fires.
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
        use crate::config::test_support::minimal_config;
        Config {
            port: 0,
            grpc_port: 0,
            sandbox_id: "task-test".to_string(),
            task_id: "task-test".to_string(),
            repo_dir: "/tmp/workspace".to_string(),
            backend_grpc_url: "http://localhost:50051".to_string(),
            claude_cli_path: "/usr/bin/claude".to_string(),
            codex_cli_path: "/usr/bin/codex".to_string(),
            codex_home_dir,
            ..minimal_config()
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
