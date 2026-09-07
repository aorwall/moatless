//! The GitHub OAuth stash.
//!
//! Issuing a token is [`crate::git_host_tokens`]. This module holds only the
//! stash that lets a PAT override be removed without losing the App OAuth
//! credential it covered.

use chrono::{DateTime, Utc};

use crate::secret::error::SecretError;
use crate::secret::schemas::PutSecretRequest;
use crate::secret::service::SecretService;
use moatless_types::access_control::ScopeContext;
use moatless_types::secret::SecretKind;

/// Key of the internal secret holding a GitHub App user's original credential
/// while a PAT override is active, so removing the override can restore it.
/// Encrypted and `internal`-kind, so it never reaches a sandbox.
pub const GITHUB_OAUTH_STASH_KEY: &str = "GITHUB_OAUTH_ACCESS_TOKEN";

/// The GitHub App user credential a PAT override stands in front of.
///
/// A user access token expires within hours, so `refresh_token` and
/// `expires_at` are as much of the credential as the token is: a restore that
/// drops them leaves a row
/// [`crate::auth::token_refresh::ensure_fresh_token`] can neither date nor
/// renew, and the person loses GitHub access at the original token's expiry
/// with nothing in the logs to say so.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StashedGithubCredential {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

impl StashedGithubCredential {
    /// A credential carrying only its access token, for a caller that has
    /// nothing else to stash.
    pub fn from_access_token(access_token: &str) -> Self {
        Self {
            access_token: access_token.to_string(),
            refresh_token: None,
            expires_at: None,
        }
    }

    /// Read a stored stash.
    ///
    /// A stash written before the credential was stored whole holds the bare
    /// access token, which is not JSON — those restore as access-token-only
    /// and refresh once the person reconnects.
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

/// Stash a GitHub App user credential behind an active PAT override.
pub async fn stash_github_oauth_token(
    secrets: &SecretService,
    user_id: &str,
    credential: &StashedGithubCredential,
) -> Result<(), SecretError> {
    let value = serde_json::to_string(credential)
        .map_err(|e| SecretError::Internal(format!("serialize the GitHub App credential: {e}")))?;
    secrets
        .put_secret(
            &user_scope(user_id),
            user_id,
            PutSecretRequest {
                scope: Some(moatless_api_schemas::enums::Scope::User),
                key: GITHUB_OAUTH_STASH_KEY.to_string(),
                kind: SecretKind::Internal,
                provider: Some("github".to_string()),
                value: Some(value),
                external_ref: None,
            },
        )
        .await
        .map(|_| ())
}

/// Read the stashed GitHub App user credential, if any.
pub async fn read_github_oauth_stash(
    secrets: &SecretService,
    user_id: &str,
) -> Result<Option<StashedGithubCredential>, SecretError> {
    Ok(secrets
        .resolve_user_secret(user_id, GITHUB_OAUTH_STASH_KEY, Some(SecretKind::Internal))
        .await?
        .as_deref()
        .map(StashedGithubCredential::parse))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential() -> StashedGithubCredential {
        StashedGithubCredential {
            access_token: "ghu_live".to_string(),
            refresh_token: Some("ghr_live".to_string()),
            expires_at: Some(DateTime::from_timestamp(1_800_000_000, 0).expect("a valid time")),
        }
    }

    #[test]
    fn a_whole_credential_round_trips() {
        let stored = serde_json::to_string(&credential()).expect("serialize");
        assert_eq!(StashedGithubCredential::parse(&stored), credential());
    }

    #[test]
    fn a_stash_written_as_a_bare_token_still_reads() {
        // Every stash written before this struct existed is a raw `ghu_`
        // string. Failing them would strand the people holding one behind a
        // PAT they cannot remove.
        assert_eq!(
            StashedGithubCredential::parse("ghu_legacy"),
            StashedGithubCredential::from_access_token("ghu_legacy")
        );
    }
}
