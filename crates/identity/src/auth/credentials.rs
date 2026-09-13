//! The OAuth stash.
//!
//! Issuing a token is [`crate::git_host_tokens`]. This module holds only the
//! stash that lets a PAT override be removed without losing the OAuth
//! credential it covered. GitHub keys it by [`GITHUB_OAUTH_STASH_KEY`] and
//! Forgejo by [`moatless_core::secret_keys::forgejo_oauth_stash_key`], one per
//! instance.

use chrono::{DateTime, Utc};

use crate::secret::error::SecretError;
use crate::secret::schemas::PutSecretRequest;
use crate::secret::service::SecretService;
use moatless_types::access_control::ScopeContext;
use moatless_types::secret::SecretKind;

/// Key of the internal secret stashing a GitHub App user's original credential
/// while a PAT override is active; removing the override restores it.
///
/// Encrypted and `internal`-kind, so it never reaches a sandbox.
pub const GITHUB_OAUTH_STASH_KEY: &str = "GITHUB_OAUTH_ACCESS_TOKEN";

/// The OAuth credential a PAT override stands in front of.
///
/// `refresh_token` and `expires_at` are as much the credential as the access
/// token: dropped on restore,
/// [`crate::auth::token_refresh::ensure_fresh_token`] can't renew it, and
/// access silently expires.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StashedOAuthCredential {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

impl StashedOAuthCredential {
    /// A credential carrying only its access token, for a caller that has
    /// nothing else to stash.
    pub fn from_access_token(access_token: &str) -> Self {
        Self {
            access_token: access_token.to_string(),
            refresh_token: None,
            expires_at: None,
        }
    }

    /// Reads a stored stash.
    ///
    /// A stash written before the credential was stored whole holds the bare
    /// access token, not JSON; those restore as access-token-only and refresh
    /// once the person reconnects.
    fn parse(stored: &str) -> Self {
        serde_json::from_str(stored).unwrap_or_else(|_| Self::from_access_token(stored))
    }
}

/// `ScopeContext` for a backend-initiated read/write of a user's own secret.
/// Not admin — the only scope it can reach is that user's.
fn user_scope(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.to_string(),
        is_admin: false,
    }
}

/// Stash `user_id`'s OAuth credential for `provider` behind an active PAT
/// override, under the secret named `key`.
pub async fn stash_oauth_credential(
    secrets: &SecretService,
    user_id: &str,
    key: &str,
    provider: &str,
    credential: &StashedOAuthCredential,
) -> Result<(), SecretError> {
    let value = serde_json::to_string(credential)
        .map_err(|e| SecretError::Internal(format!("serialize the {provider} credential: {e}")))?;
    secrets
        .put_secret(
            &user_scope(user_id),
            user_id,
            PutSecretRequest {
                scope: Some(moatless_api_schemas::enums::Scope::User),
                key: key.to_string(),
                kind: SecretKind::Internal,
                provider: Some(provider.to_string()),
                value: Some(value),
                external_ref: None,
            },
        )
        .await
        .map(|_| ())
}

/// Read the credential stashed under `key`, if any.
pub async fn read_oauth_stash(
    secrets: &SecretService,
    user_id: &str,
    key: &str,
) -> Result<Option<StashedOAuthCredential>, SecretError> {
    Ok(secrets
        .resolve_user_secret(user_id, key, Some(SecretKind::Internal))
        .await?
        .as_deref()
        .map(StashedOAuthCredential::parse))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential() -> StashedOAuthCredential {
        StashedOAuthCredential {
            access_token: "ghu_live".to_string(),
            refresh_token: Some("ghr_live".to_string()),
            expires_at: Some(DateTime::from_timestamp(1_800_000_000, 0).expect("a valid time")),
        }
    }

    #[test]
    fn a_whole_credential_round_trips() {
        let stored = serde_json::to_string(&credential()).expect("serialize");
        assert_eq!(StashedOAuthCredential::parse(&stored), credential());
    }

    #[test]
    fn a_stash_written_as_a_bare_token_still_reads() {
        // Every stash written before this struct existed is a raw `ghu_`
        // string. Failing them would strand the people holding one behind a
        // PAT they cannot remove.
        assert_eq!(
            StashedOAuthCredential::parse("ghu_legacy"),
            StashedOAuthCredential::from_access_token("ghu_legacy")
        );
    }
}
