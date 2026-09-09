//! Implements the sandbox credential capability.

use super::CredentialSync;
use moatless_identity::credentials::CredentialError;

/// The sandbox band's view of this orchestrator: one `auth.json`, with "no
/// credential anywhere" told apart from a failure.
#[async_trait::async_trait]
impl moatless_ports::sandbox::CodexCredentials for CredentialSync {
    async fn ensure_fresh_codex_for(
        &self,
        user_id: &str,
        except_task_id: Option<&str>,
    ) -> Result<Option<String>, String> {
        match CredentialSync::ensure_fresh_codex_for(self, user_id, except_task_id).await {
            Ok(blob) => Ok(Some(blob)),
            Err(CredentialError::NotFound(_)) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}
