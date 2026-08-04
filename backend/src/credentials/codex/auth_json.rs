//! Validation + persistence helpers for user-supplied Codex `auth.json` blobs.
//!
//! Shared by the user-facing `save_codex_auth` handler, the admin endpoint
//! that seeds bot codex rows, the team manager save endpoint, and the global
//! admin endpoint. Keeping the parse / metadata / save logic here means every
//! scope produces an identical row shape and broadcast behaviour.

use std::sync::Arc;

use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::credentials::codex::scope::CodexScope;
use crate::credentials::{CredentialService, WriteSource};
use crate::provider_settings::error::ProviderSettingsError;
use crate::provider_settings::schemas::{CodexConfigResponse, SaveCodexAuthRequest};

/// Parse the user-supplied Codex `auth.json` body and validate it's a JSON
/// object. Used by both the user-facing `save_codex_auth` handler and the
/// admin endpoint that seeds bots' codex rows.
pub fn parse_codex_auth_json(raw: &str) -> Result<serde_json::Value, ProviderSettingsError> {
    let parsed: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| ProviderSettingsError::BadRequest(format!("Invalid Codex auth JSON: {e}")))?;

    if !parsed.is_object() {
        return Err(ProviderSettingsError::BadRequest(
            "Codex auth JSON must be a JSON object".to_string(),
        ));
    }

    Ok(parsed)
}

/// Build the canonical `provider_tokens.metadata` value for a parsed Codex
/// auth.json blob. Mirrors the snippet in `save_codex_auth` so the admin
/// endpoint records the same shape.
pub fn build_codex_metadata(parsed: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "auth_method": "auth_json",
        "has_openai_api_key": parsed
            .get("OPENAI_API_KEY")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "has_tokens": parsed
            .get("tokens")
            .is_some_and(serde_json::Value::is_object),
    })
}

/// Shared codex-blob save path used by every scope endpoint (user / admin global).
/// Parses + validates, upserts the row addressed by `scope`, and broadcasts
/// the rotation to running sandboxes whose effective user resolves through
/// this scope.
///
/// Centralising here keeps the scope-specific HTTP handlers thin and
/// guarantees they apply identical metadata, error mapping, and broadcast
/// behaviour.
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
        .upsert_codex_blob(
            &scope,
            &serialized,
            None,
            Some("auth_json"),
            Some(&metadata),
        )
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
