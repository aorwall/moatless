use std::sync::Arc;

use tracing::warn;

use moatless_identity::credentials::{CredentialError, CredentialService};
use moatless_types::agent::CodexScope;

/// Just-in-time Codex refresh over the pure [`CredentialService`].
///
/// The backend is the only refresher of a Codex login: no pod holds one, so
/// nothing here yields to a sandbox or pushes a rotated blob to one.
pub struct CredentialSync {
    credentials: Arc<CredentialService>,
}

impl CredentialSync {
    /// Build the orchestrator over a pure credential service.
    pub(crate) fn new(credentials: Arc<CredentialService>) -> Self {
        Self { credentials }
    }

    /// Just-in-time Codex auth.json refresh, addressed by `scope`, through
    /// [`CredentialService::refresh_codex_locked`].
    pub async fn ensure_fresh_codex(&self, scope: CodexScope) -> Result<String, CredentialError> {
        let outcome = self.credentials.refresh_codex_locked(&scope).await?;
        Ok(outcome.into_blob())
    }

    /// Resolve `CODEX_AUTH_JSON` with `user > global` precedence in one call.
    ///
    /// The walk lives here so "most-specific wins" is enforced centrally, not
    /// by caller ordering; a `NotFound` scope falls through silently, but a
    /// transient error is logged and the walk continues rather than failing on
    /// one scope's outage.
    pub async fn ensure_fresh_codex_for(&self, user_id: &str) -> Result<String, CredentialError> {
        let scopes = [CodexScope::user(user_id), CodexScope::Global];

        let mut last_err: Option<CredentialError> = None;
        for scope in &scopes {
            match self.ensure_fresh_codex(scope.clone()).await {
                Ok(blob) => return Ok(blob),
                Err(CredentialError::NotFound(_)) => continue,
                Err(e) => {
                    warn!(
                        error = %e,
                        scope = %scope.display(),
                        "ensure_fresh_codex_for: transient failure, trying next scope"
                    );
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            CredentialError::NotFound(format!(
                "No codex provider_token found at any scope (user={user_id})"
            ))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_identity::credentials::test_support::{InMemoryTokens, credentials_over};

    #[tokio::test]
    async fn ensure_fresh_codex_returns_the_stored_blob_with_no_refresher_wired() {
        // No db/http, so `refresh_codex_locked` falls back to `read_raw_codex`.
        let sync = CredentialSync::new(credentials_over(InMemoryTokens::with_codex_blob(
            "seeded-blob",
        )));

        let blob = sync
            .ensure_fresh_codex(CodexScope::user("u1"))
            .await
            .unwrap();
        assert_eq!(blob, "seeded-blob");
    }
}
