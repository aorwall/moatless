//! On-demand token refresh for GitHub App user tokens.
//!
//! GitHub App user access tokens expire (typically 8 hours). This module
//! provides a function to transparently refresh them before returning
//! credentials to callers.

use chrono::Utc;
use tracing::{info, warn};

use crate::auth::error::AuthError;
use crate::auth::github::service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::provider_tokens::ProviderTokenAuthRepository;

/// Ensure the GitHub token for a user is fresh.
///
/// If the token was obtained via GitHub App (`auth_method = "github_app"`) and
/// is expired or about to expire, refreshes it using the stored refresh token
/// and persists the new credentials.
///
/// For OAuth or PAT tokens, returns the token as-is.
///
/// # Concurrency
///
/// This function does NOT serialize concurrent callers — two parallel
/// invocations for the same user would both attempt to refresh, and the loser
/// would get `bad_refresh_token` because GitHub invalidates the prior refresh
/// token on every successful exchange. Callers that may race (e.g. parallel
/// task provisions for the same user) should go through
/// `crate::credentials::CredentialService::ensure_fresh`, which wraps this
/// with a per-`(user, key)` mutex.
pub async fn ensure_fresh_token(
    token_dao: &dyn ProviderTokenRepository,
    token_auth: &dyn ProviderTokenAuthRepository,
    github_app_service: &GitHubAppService,
    user_id: &str,
) -> Result<String, AuthError> {
    ensure_fresh_token_with_dao(token_dao, token_auth, github_app_service, user_id).await
}

/// Internal entry point used by `CredentialService::ensure_fresh` once it has
/// acquired the per-`(user, key)` mutex. Callers outside the service should
/// prefer the public `ensure_fresh_token` to keep the call-site uniform.
pub async fn ensure_fresh_token_with_dao(
    token_dao: &dyn ProviderTokenRepository,
    token_auth: &dyn ProviderTokenAuthRepository,
    github_app_service: &GitHubAppService,
    user_id: &str,
) -> Result<String, AuthError> {
    let token = token_dao
        .find_provider_token_full(user_id, "github")
        .await
        .map_err(AuthError::Database)?
        .ok_or_else(|| AuthError::NotFound("No GitHub token found for user".to_string()))?;

    // Only refresh GitHub App tokens
    if token.auth_method.as_deref() != Some("github_app") {
        return Ok(token.access_token);
    }

    // Check if token is still valid (5-minute safety margin)
    if let Some(expires_at) = token.expires_at {
        let margin = Utc::now() + chrono::Duration::minutes(5);
        if expires_at > margin {
            return Ok(token.access_token);
        }
    } else {
        // No expiry set — treat as still valid
        return Ok(token.access_token);
    }

    // Token is expired or about to expire — refresh it
    let refresh_token = token.refresh_token.as_deref().ok_or_else(|| {
        AuthError::OAuth("GitHub App token expired but no refresh token available".to_string())
    })?;

    info!(user_id = %user_id, "Refreshing expired GitHub App user token");

    let new_tokens = github_app_service.refresh_user_token(refresh_token).await?;

    let now = Utc::now();
    let new_pt = crate::auth::provider_tokens::ProviderToken {
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
