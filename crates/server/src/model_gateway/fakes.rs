//! Test doubles shared by the gateway's provider tests.

use async_trait::async_trait;
use moatless_ports::sandbox::CodexCredentials;

/// A Codex refresher that hands every identity the same login, or none.
pub struct FixedCodex(pub Option<&'static str>);

#[async_trait]
impl CodexCredentials for FixedCodex {
    async fn ensure_fresh_codex_for(&self, _user_id: &str) -> Result<Option<String>, String> {
        Ok(self.0.map(str::to_string))
    }
}

/// A Codex refresher a test expects never to be asked.
pub struct UnusedCodex;

#[async_trait]
impl CodexCredentials for UnusedCodex {
    async fn ensure_fresh_codex_for(&self, user_id: &str) -> Result<Option<String>, String> {
        panic!("refreshed the Codex login of {user_id}");
    }
}
