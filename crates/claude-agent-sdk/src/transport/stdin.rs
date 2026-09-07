use super::StdinWriter;
use crate::error::Result;
use async_trait::async_trait;
use std::fmt::Write;

/// Wrapper around `Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>` that
/// implements `StdinWriter`.
pub(crate) struct ChildStdinWriter {
    pub(crate) inner: std::sync::Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
}

#[async_trait]
impl StdinWriter for ChildStdinWriter {
    async fn write_json_line(&self, json: &str) -> Result<()> {
        use crate::error::SdkError;
        use tokio::io::AsyncWriteExt;
        log_stdin_line(json);
        let mut stdin = self.inner.lock().await;
        stdin
            .write_all(json.as_bytes())
            .await
            .map_err(SdkError::SpawnError)?;
        stdin.write_all(b"\n").await.map_err(SdkError::SpawnError)?;
        stdin.flush().await.map_err(SdkError::SpawnError)?;
        Ok(())
    }
}

/// Log a stdin JSON line with a concise summary instead of the full JSON.
fn log_stdin_line(json: &str) {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(json) {
        let msg_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("?");
        let request_id = value.get("request_id").and_then(|v| v.as_str());
        let subtype = value
            .get("request")
            .and_then(|r| r.get("subtype"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                value
                    .get("response")
                    .and_then(|r| r.get("subtype"))
                    .and_then(|v| v.as_str())
            });

        let mut summary = format!("type={msg_type}");
        if let Some(rid) = request_id {
            let _ = write!(summary, " req={rid}");
        }
        if let Some(st) = subtype {
            let _ = write!(summary, " subtype={st}");
        }
        tracing::debug!("CLI stdin: {}", summary);
    } else if json.len() > 200 {
        tracing::debug!("CLI stdin: (non-JSON, {} bytes)", json.len());
    } else {
        tracing::debug!("CLI stdin: {}", json);
    }
}
