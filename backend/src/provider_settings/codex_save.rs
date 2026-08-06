//! Scope-aware Codex `auth.json` save path.
//!
//! Persists the blob addressed by `scope` and broadcasts the rotation to
//! running sandboxes whose effective user resolves through this scope. Parsing
//! and metadata construction live in `auth::codex_auth_json`; centralising the
//! save here keeps the scope-specific HTTP handlers thin and guarantees they
//! apply identical metadata, error mapping, and broadcast behaviour.

use std::sync::Arc;

use moatless_api_schemas::provider_settings::{CodexConfigResponse, SaveCodexAuthRequest};

use crate::auth::codex_auth_json::{build_codex_metadata, parse_codex_auth_json};
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::credentials::codex::scope::CodexScope;
use crate::credentials::{CredentialService, WriteSource};
use crate::provider_settings::error::ProviderSettingsError;

/// Shared codex-blob save path used by every scope endpoint (user / admin global).
/// Parses + validates, upserts the row addressed by `scope`, and broadcasts
/// the rotation to running sandboxes whose effective user resolves through
/// this scope.
pub async fn save_codex_auth_for_scope(
    provider_tokens: &Arc<dyn ProviderTokenDao>,
    credentials_svc: &Arc<CredentialService>,
    scope: CodexScope,
    body: SaveCodexAuthRequest,
) -> Result<CodexConfigResponse, ProviderSettingsError> {
    let parsed = parse_codex_auth_json(&body.auth_json)?;
    let metadata = build_codex_metadata(&parsed);
    let serialized = parsed.to_string();

    provider_tokens
        .upsert_codex_blob(&scope, &serialized, None, Some("auth_json"), Some(&metadata))
        .await?;

    // Fan the rotation out to every running sandbox whose effective user
    // resolves through this scope. For user scope we route via `store` so
    // the standard dedup path runs; for global we call the scope-aware
    // broadcast directly because `store` is hard-wired to user-scoped
    // provider_tokens.
    if let Err(e) = match &scope {
        CodexScope::User(user_id) => {
            credentials_svc
                .store(
                    user_id,
                    "CODEX_AUTH_JSON",
                    &serialized,
                    WriteSource::UserUiSave,
                )
                .await
        }
        CodexScope::Global => {
            credentials_svc
                .broadcast_codex_rotation(&scope, &serialized)
                .await;
            Ok(())
        }
    } {
        tracing::warn!(
            error = %e,
            scope = %scope,
            "Failed to broadcast Codex auth save to running sandboxes"
        );
    }

    Ok(CodexConfigResponse {
        connected: true,
        token_type: Some("auth_json".to_string()),
        auth_method: Some("auth_json".to_string()),
        email: None,
        plan_type: None,
        needs_reconnect: Some(false),
    })
}
