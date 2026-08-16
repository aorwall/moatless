//! Validation + metadata helpers for user-supplied Codex `auth.json` blobs.
//!
//! Parsing and the canonical `provider_tokens.metadata` shape live here so
//! every scope endpoint that seeds a codex row — the user save handler, the
//! admin seed endpoint, the global admin endpoint — produces an identical row
//! shape. The scope-aware save path that persists and broadcasts the blob
//! lives in `agent_harness_access::AgentHarnessAccess`.

use crate::auth::error::AuthError;

/// Parse the user-supplied Codex `auth.json` body and validate it's a JSON
/// object.
pub fn parse_codex_auth_json(raw: &str) -> Result<serde_json::Value, AuthError> {
    let parsed: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| AuthError::BadRequest(format!("Invalid Codex auth JSON: {e}")))?;

    if !parsed.is_object() {
        return Err(AuthError::BadRequest(
            "Codex auth JSON must be a JSON object".to_string(),
        ));
    }

    Ok(parsed)
}

/// Build the canonical `provider_tokens.metadata` value for a parsed Codex
/// auth.json blob.
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
