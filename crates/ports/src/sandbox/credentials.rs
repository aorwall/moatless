use async_trait::async_trait;

/// Just-in-time Codex credential refresh for the identity a sandbox acts as.
///
/// Inverts the tasks-band edge, so this band names no tasks-band type.
/// `Ok(None)` is no credential at any scope; `Err` is a failure to report.
#[async_trait]
pub trait CodexCredentials: Send + Sync {
    async fn ensure_fresh_codex_for(&self, user_id: &str) -> Result<Option<String>, String>;
}
