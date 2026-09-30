//! Concrete [`CredentialSink`] implementations.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::fs;
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::CredentialSink;
use crate::credentials::codex_watcher::WatcherHandle;
use crate::credentials::gh_hosts;

/// Atomic-writes `~/.codex/auth.json`.
///
/// After write, seeds the watcher cache so the resulting filesystem event is
/// recognised as a backend-originating write and not echoed back as a fresh
/// `UpdateCredential` push.
pub struct CodexAuthJsonSink {
    pub codex_home_dir: String,
    pub watcher_handle: Arc<RwLock<Option<WatcherHandle>>>,
}

#[async_trait]
impl CredentialSink for CodexAuthJsonSink {
    async fn apply(&self, value: &str) -> Result<(), String> {
        // Validate JSON before writing. A malformed blob would break codex on
        // its next launch — better to reject than to corrupt the file.
        let parsed: serde_json::Value = serde_json::from_str(value)
            .map_err(|e| format!("CODEX_AUTH_JSON is not valid JSON: {e}"))?;
        if !parsed.is_object() {
            return Err("CODEX_AUTH_JSON must be a JSON object".to_string());
        }
        let canonical = parsed.to_string();

        if let Err(e) = fs::create_dir_all(&self.codex_home_dir).await {
            return Err(format!("create_dir_all({}): {e}", self.codex_home_dir));
        }
        let final_path = PathBuf::from(format!("{}/auth.json", self.codex_home_dir));
        let tmp_path = PathBuf::from(format!("{}/auth.json.tmp", self.codex_home_dir));

        fs::write(&tmp_path, canonical.as_bytes())
            .await
            .map_err(|e| format!("write({}): {e}", tmp_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) =
                fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600)).await
            {
                warn!(error = %e, "Failed to chmod 0600 on Codex auth tmp file");
            }
        }

        fs::rename(&tmp_path, &final_path)
            .await
            .map_err(|e| format!("rename: {e}"))?;

        // Seed the watcher cache BEFORE returning so any filesystem events
        // for this write get short-circuited by the dedup check.
        if let Some(handle) = self.watcher_handle.read().await.clone() {
            handle.set_last_value(canonical).await;
        }

        info!("Wrote rotated Codex auth.json from backend push");
        Ok(())
    }
}

/// Refresh the GitHub token used by `gh auth git-credential` (and therefore
/// every `git push`/`gh` call inside the agent). Wraps the existing
/// `gh_hosts::write_github_token` flow.
pub struct GitTokenSink {
    pub home_dir: String,
    pub github: moatless_types::http::HttpTimeouts,
}

#[async_trait]
impl CredentialSink for GitTokenSink {
    async fn apply(&self, value: &str) -> Result<(), String> {
        gh_hosts::write_github_token(&self.home_dir, value, self.github)
            .await
            .map_err(|e| format!("write gh hosts.yml: {e}"))?;

        info!("Refreshed GitHub token via gh hosts.yml");
        Ok(())
    }
}

/// Stub sink for `CLAUDE_CODE_OAUTH_TOKEN`. Today these tokens don't rotate, so
/// this is a no-op kept registered for forward compatibility — when Claude adds
/// rotation, the file-write logic moves here.
pub struct ClaudeOauthSink;

#[async_trait]
impl CredentialSink for ClaudeOauthSink {
    async fn apply(&self, _value: &str) -> Result<(), String> {
        info!("ClaudeOauthSink: refresh received but rotation not yet supported");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::codex_watcher::WatcherHandle;
    use tempfile::TempDir;

    #[tokio::test]
    async fn codex_sink_writes_atomically_and_seeds_watcher() {
        let tmp = TempDir::new().unwrap();
        let codex_dir = tmp.path().join(".codex");
        let watcher_handle = Arc::new(tokio::sync::RwLock::new(None::<WatcherHandle>));

        // Seed a watcher with a known cache value so we can assert the sink
        // updates it after writing the file.
        let last_value = Arc::new(tokio::sync::Mutex::new("OLD".to_string()));
        *watcher_handle.write().await = Some(WatcherHandle {
            last_value: Arc::clone(&last_value),
        });

        let sink = CodexAuthJsonSink {
            codex_home_dir: codex_dir.display().to_string(),
            watcher_handle,
        };

        let payload = r#"{"tokens":{"access_token":"new"}}"#;
        sink.apply(payload).await.unwrap();

        // File written at the expected path.
        let on_disk = tokio::fs::read_to_string(codex_dir.join("auth.json"))
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&on_disk).unwrap();
        assert_eq!(parsed["tokens"]["access_token"], "new");

        // .tmp file cleaned up by rename.
        assert!(!codex_dir.join("auth.json.tmp").exists());

        // Watcher cache seeded with the canonical written form.
        let cached = last_value.lock().await.clone();
        let parsed: serde_json::Value = serde_json::from_str(&cached).unwrap();
        assert_eq!(parsed["tokens"]["access_token"], "new");
    }

    #[tokio::test]
    async fn codex_sink_rejects_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let codex_dir = tmp.path().join(".codex");

        let sink = CodexAuthJsonSink {
            codex_home_dir: codex_dir.display().to_string(),
            watcher_handle: Arc::new(tokio::sync::RwLock::new(None)),
        };

        let err = sink.apply("{not json").await.expect_err("should reject");
        assert!(err.contains("not valid JSON"));
        // No file written.
        assert!(!codex_dir.join("auth.json").exists());
    }

    #[tokio::test]
    async fn codex_sink_rejects_non_object() {
        let tmp = TempDir::new().unwrap();
        let codex_dir = tmp.path().join(".codex");

        let sink = CodexAuthJsonSink {
            codex_home_dir: codex_dir.display().to_string(),
            watcher_handle: Arc::new(tokio::sync::RwLock::new(None)),
        };

        let err = sink.apply("\"a string\"").await.expect_err("should reject");
        assert!(err.contains("must be a JSON object"));
    }
}
