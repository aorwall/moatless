//! The GitHub OAuth stash.
//!
//! Issuing a token is [`crate::git_host_tokens`]. This module holds only the
//! stash that lets a PAT override be removed without losing the App OAuth
//! token it covered.

use crate::secret::error::SecretError;
use moatless_types::secret::SecretKind;
use crate::secret::schemas::PutSecretRequest;
use crate::secret::service::SecretService;
use moatless_types::access_control::ScopeContext;

/// Key of the internal secret holding a GitHub App user's original access
/// token while a PAT override is active, so removing the override can restore
/// it. Encrypted and `internal`-kind, so it never reaches a sandbox.
pub const GITHUB_OAUTH_STASH_KEY: &str = "GITHUB_OAUTH_ACCESS_TOKEN";

/// `ScopeContext` for a backend-initiated read/write of a user's own secret.
/// Not admin — the only scope it can reach is that user's.
fn user_scope(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.to_string(),
        is_admin: false,
    }
}

/// Stash a GitHub App user access token behind an active PAT override.
pub async fn stash_github_oauth_token(
    secrets: &SecretService,
    user_id: &str,
    oauth_access_token: &str,
) -> Result<(), SecretError> {
    secrets
        .put_secret(
            &user_scope(user_id),
            user_id,
            PutSecretRequest {
                scope: Some(moatless_api_schemas::enums::Scope::User),
                key: GITHUB_OAUTH_STASH_KEY.to_string(),
                kind: SecretKind::Internal,
                provider: Some("github".to_string()),
                value: Some(oauth_access_token.to_string()),
                external_ref: None,
            },
        )
        .await
        .map(|_| ())
}

/// Read the stashed GitHub App user access token, if any.
pub async fn read_github_oauth_stash(
    secrets: &SecretService,
    user_id: &str,
) -> Result<Option<String>, SecretError> {
    secrets
        .resolve_user_secret(user_id, GITHUB_OAUTH_STASH_KEY, Some(SecretKind::Internal))
        .await
}
