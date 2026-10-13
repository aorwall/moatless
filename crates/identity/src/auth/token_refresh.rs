//! On-demand token refresh for GitHub App user tokens.
//!
//! GitHub App user access tokens expire (typically 8 hours). This module
//! provides a function to transparently refresh them before returning
//! credentials to callers.

use chrono::Utc;
use tracing::{info, warn};

use crate::auth::error::AuthError;
use crate::auth::github::service::GitHubAppService;
use crate::auth::provider_token_auth_dao::ProviderTokenAuthDao;
use crate::auth::provider_token_dao::ProviderTokenDao;

/// Ensures the GitHub token for a user is fresh: refreshes an expired,
/// soon-to-expire, or undated `github_app` token and persists it. Other
/// token kinds, and a `github_app` row with no refresh token, return as-is.
///
/// Not concurrency-safe: parallel calls for the same user race and the
/// loser gets `bad_refresh_token`; the Codex path serializes separately.
pub async fn ensure_fresh_token(
    token_dao: &dyn ProviderTokenDao,
    token_auth: &dyn ProviderTokenAuthDao,
    github_app_service: &GitHubAppService,
    user_id: &str,
) -> Result<String, AuthError> {
    ensure_fresh_token_with_dao(token_dao, token_auth, github_app_service, user_id).await
}

/// Internal implementation that [`ensure_fresh_token`] forwards to, kept
/// separate so the DAO trio can be passed in directly. Callers should prefer
/// the public `ensure_fresh_token` to keep the call-site uniform.
pub async fn ensure_fresh_token_with_dao(
    token_dao: &dyn ProviderTokenDao,
    token_auth: &dyn ProviderTokenAuthDao,
    github_app_service: &GitHubAppService,
    user_id: &str,
) -> Result<String, AuthError> {
    let token = token_dao
        .find_provider_token_full(user_id, "github")
        .await
        .map_err(AuthError::Database)?
        .ok_or_else(|| AuthError::NotFound("No GitHub token found for user".to_string()))?;

    if token.auth_method.as_deref() != Some("github_app") {
        return Ok(token.access_token);
    }

    // Check if token is still valid (5-minute safety margin). A missing expiry
    // is unknown, never "never": every App user token expires, so a row that
    // cannot be dated is refreshed once and dated by the result.
    if let Some(expires_at) = token.expires_at {
        let margin = Utc::now() + chrono::Duration::minutes(5);
        if expires_at > margin {
            return Ok(token.access_token);
        }
    }

    // Token is expired, about to expire, or undatable — refresh it
    let Some(refresh_token) = token.refresh_token.as_deref() else {
        // Nothing to renew with, so the stored token is the only answer there
        // is. It is already dead when the expiry was real, and the caller's
        // 401 is what says so.
        warn!(
            user_id = %user_id,
            "GitHub App token needs a refresh but the row carries no refresh token"
        );
        return Ok(token.access_token);
    };

    info!(user_id = %user_id, "Refreshing expired GitHub App user token");

    let new_tokens = github_app_service.refresh_user_token(refresh_token).await?;

    let now = Utc::now();
    let new_pt = crate::auth::provider_token_auth_dao::ProviderToken {
        user_id: user_id.to_string(),
        provider: "github".to_string(),
        auth_method: "github_app".to_string(),
        access_token: new_tokens.access_token.clone(),
        refresh_token: new_tokens.refresh_token,
        token_type: Some(new_tokens.token_type),
        scope: new_tokens.scope,
        provider_user_id: None, // preserve existing via upsert
        provider_login: None,
        connected_at: now,
        expires_at: new_tokens
            .expires_in
            .map(|secs| now + chrono::Duration::seconds(secs as i64)),
    };

    if let Err(e) = token_auth.upsert_provider_token(&new_pt).await {
        warn!(
            error = %e,
            user_id = %user_id,
            "Failed to persist refreshed GitHub App token"
        );
        // Still return the new token even if persistence fails
    }

    Ok(new_tokens.access_token)
}
