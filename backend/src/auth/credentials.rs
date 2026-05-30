//! Canonical GitHub-token resolver shared by the sandbox credential builder
//! and runtime API clients (github_pr adapter, etc.).
//!
//! Centralising the dispatch — bot user (installation token) vs. GitHub App
//! user (refreshed OAuth) vs. raw PAT — ensures every code path that hands a
//! token to GitHub goes through the same branch table. Previously the dispatch
//! lived only in `sandbox/secrets.rs`, so any non-sandbox path reading
//! `access_token` directly received the empty placeholder for bot users.
//!
//! The resolver returns `Option<String>` so callers decide error semantics
//! (e.g. the sandbox path silently skips a missing token, the adapter path
//! falls back to the global PAT or returns an error).

use tracing::warn;

use super::error::AuthError;
use super::github_app_registry::GitHubAppRegistry;
use super::github_app_service::GitHubAppService;
use super::models::ProviderToken;
use super::provider_token_dao::ProviderTokenRepository;
use super::provider_tokens::{ProviderTokenAuthRepository, AUTH_METHOD_APP_INSTALLATION};
use super::token_refresh::ensure_fresh_token;

/// Resolve the GitHub token a given user should present to GitHub at this
/// moment.
///
/// Branches by `provider_tokens.auth_method`:
/// - `github_app_installation` (bot user): mint a fresh installation token via
///   the App registry, scoped to `repo_owner`. Missing registry entry or
///   missing owner is a hard `Err(AuthError::Config(...))` — silently falling
///   back to "no token" hides bot misconfiguration.
/// - `github_app` (human user with App OAuth): delegate to `ensure_fresh_token`,
///   which refreshes if the access token is near expiry.
/// - anything else (OAuth, PAT): return `access_token` verbatim.
///
/// Returns `Ok(None)` only when the user has no `provider_tokens` row at all.
pub async fn resolve_user_github_token(
    provider_tokens: &dyn ProviderTokenRepository,
    provider_token_auth: Option<&dyn ProviderTokenAuthRepository>,
    github_app_service: Option<&GitHubAppService>,
    github_apps: &GitHubAppRegistry,
    user_id: &str,
    repo_owner: Option<&str>,
) -> Result<Option<String>, AuthError> {
    let Some(row) = provider_tokens
        .find_provider_token_full(user_id, "github")
        .await
        .map_err(AuthError::Database)?
    else {
        return Ok(None);
    };

    match row.auth_method.as_deref() {
        Some(AUTH_METHOD_APP_INSTALLATION) => {
            let token = mint_installation_token(&row, github_apps, repo_owner).await?;
            Ok(Some(token))
        }
        Some("github_app") => {
            if let (Some(app), Some(auth)) = (github_app_service, provider_token_auth) {
                match ensure_fresh_token(provider_tokens, auth, app, user_id).await {
                    Ok(token) => Ok(Some(token)),
                    Err(e) => {
                        warn!(
                            error = %e,
                            user_id,
                            "GitHub App token refresh failed, falling back to raw token"
                        );
                        Ok(Some(row.access_token))
                    }
                }
            } else {
                Ok(Some(row.access_token))
            }
        }
        _ => Ok(Some(row.access_token)),
    }
}

/// Mint a fresh installation token via the configured App for `repo_owner`.
///
/// The `provider_tokens` row carries the App key in `provider_user_id`. Any
/// missing dependency (registry handle for that key, owner, app_key on the
/// row) is reported as `AuthError::Config` so the caller fails fast and the
/// bot misconfiguration is surfaced rather than masked.
async fn mint_installation_token(
    row: &ProviderToken,
    github_apps: &GitHubAppRegistry,
    repo_owner: Option<&str>,
) -> Result<String, AuthError> {
    let owner = repo_owner.ok_or_else(|| {
        AuthError::Config(format!(
            "Cannot mint installation token for bot user '{}': no repo owner in context",
            row.user_id
        ))
    })?;
    let app_key = row.provider_user_id.as_deref().ok_or_else(|| {
        AuthError::Config(format!(
            "Bot user '{}' provider_tokens row is missing provider_user_id (App key)",
            row.user_id
        ))
    })?;
    let app = github_apps.get(app_key).ok_or_else(|| {
        AuthError::Config(format!(
            "Bot user '{}' references unknown GitHub App key '{app_key}'",
            row.user_id
        ))
    })?;
    app.installation_token_for_owner(owner).await
}
