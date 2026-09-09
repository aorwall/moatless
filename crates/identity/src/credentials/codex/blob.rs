//! The stored Codex `auth.json` row: read it, replace it, remove it.

use moatless_types::agent::CodexScope;

use crate::credentials::{CredentialError, CredentialService};

/// `provider_tokens.token_type` every Codex row carries.
const CODEX_TOKEN_TYPE: &str = "auth_json";

/// The stored Codex credential at one scope.
pub struct CodexCredential {
    /// The serialized `auth.json`.
    pub blob: String,
    pub token_type: Option<String>,
    /// Connection facts the settings UI reads: `auth_method`, `email`,
    /// `plan_type`, and `refresh_failure` when a refresh gave up for good.
    pub metadata: Option<serde_json::Value>,
}

impl CredentialService {
    /// The stored Codex credential at `scope`, without refreshing it.
    pub async fn codex_credential(
        &self,
        scope: &CodexScope,
    ) -> Result<Option<CodexCredential>, CredentialError> {
        Ok(self
            .provider_tokens
            .find_codex_blob(scope)
            .await?
            .map(|row| CodexCredential {
                blob: row.access_token,
                token_type: row.token_type,
                metadata: row.metadata,
            }))
    }

    /// Replace the Codex credential at `scope`.
    ///
    /// The caller fans the new blob out afterwards; this only persists.
    pub async fn store_codex_credential(
        &self,
        scope: &CodexScope,
        blob: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), CredentialError> {
        self.provider_tokens
            .upsert_codex_blob(scope, blob, None, Some(CODEX_TOKEN_TYPE), Some(metadata))
            .await?;
        Ok(())
    }

    /// Remove the Codex credential at `scope`.
    pub async fn delete_codex_credential(&self, scope: &CodexScope) -> Result<(), CredentialError> {
        self.provider_tokens.delete_codex_blob(scope).await?;
        Ok(())
    }
}
