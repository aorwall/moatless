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
use super::provider_tokens::{
    ProviderTokenAuthRepository, AUTH_METHOD_APP_INSTALLATION, AUTH_METHOD_SECRET_REF,
};
use super::token_refresh::ensure_fresh_token;
use crate::sandbox::secrets::GITHUB_TOKEN;
use crate::secret::error::SecretError;
use crate::secret::models::{SecretKind, SecretScopeType};
use crate::secret::schemas::PutSecretRequest;
use crate::secret::service::SecretService;
use moatless_backend_core::scope::ScopeContext;

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
/// - `secret_ref` (validated PAT): read the value from the encrypted `secrets`
///   table. A row whose secret is missing is a hard error — returning `None`
///   would silently look like "GitHub not connected".
/// - anything else (OAuth, raw PAT): return `access_token` verbatim.
///
/// Returns `Ok(None)` only when the user has no `provider_tokens` row at all.
pub async fn resolve_user_github_token(
    provider_tokens: &dyn ProviderTokenRepository,
    provider_token_auth: Option<&dyn ProviderTokenAuthRepository>,
    github_app_service: Option<&GitHubAppService>,
    github_apps: &GitHubAppRegistry,
    secrets: Option<&SecretService>,
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
        Some(AUTH_METHOD_SECRET_REF) => {
            let token = read_secret_ref_token(secrets, user_id).await?;
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

/// Key of the internal secret holding a GitHub-OAuth user's original access
/// token while a PAT override is active, so removing the override can restore
/// it. Encrypted and `internal`-kind, so it never reaches a sandbox.
pub const GITHUB_OAUTH_STASH_KEY: &str = "GITHUB_OAUTH_ACCESS_TOKEN";

/// `ScopeContext` for a backend-initiated read/write of a user's own secret.
/// Not admin — the only scope it can reach is that user's.
fn user_scope(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.to_string(),
        team_ids: Vec::new(),
        is_admin: false,
    }
}

/// Stash a GitHub-OAuth access token behind an active PAT override.
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
                scope_type: SecretScopeType::User,
                scope_id: Some(user_id.to_string()),
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

/// Read the stashed GitHub-OAuth access token, if any.
pub async fn read_github_oauth_stash(
    secrets: &SecretService,
    user_id: &str,
) -> Result<Option<String>, SecretError> {
    secrets
        .resolve_user_secret(user_id, GITHUB_OAUTH_STASH_KEY, Some(SecretKind::Internal))
        .await
}

/// Resolve the GitHub token for `user_id` from the wiring on [`AppState`].
///
/// Thin wrapper over [`resolve_user_github_token`] so HTTP handlers don't each
/// repeat the dependency list — and so no handler is tempted to read
/// `access_token` directly and end up sending a sentinel to GitHub.
pub async fn resolve_state_github_token(
    state: &crate::state::AppState,
    user_id: &str,
    repo_owner: Option<&str>,
) -> Result<Option<String>, AuthError> {
    resolve_user_github_token(
        state.provider_tokens.as_ref(),
        Some(state.provider_token_auth.as_ref()),
        state.github_app_service.as_deref(),
        state.github_apps.as_ref(),
        Some(state.secrets.as_ref()),
        user_id,
        repo_owner,
    )
    .await
}

/// Read the encrypted PAT a `secret_ref` row points at.
///
/// Every failure mode is an error rather than `None`: a reference row with no
/// resolvable secret means the user *is* connected but the credential is
/// unreachable, and reporting that as "not connected" would send callers down
/// an anonymous path instead of surfacing the breakage.
async fn read_secret_ref_token(
    secrets: Option<&SecretService>,
    user_id: &str,
) -> Result<String, AuthError> {
    let secrets = secrets.ok_or_else(|| {
        AuthError::Config(format!(
            "User '{user_id}' has a secret_ref GitHub token but no secret service is wired"
        ))
    })?;
    secrets
        .resolve_user_secret(user_id, GITHUB_TOKEN, Some(SecretKind::ProviderToken))
        .await
        .map_err(|e| {
            AuthError::Config(format!(
                "Failed to read GitHub token secret for user '{user_id}': {e}"
            ))
        })?
        .ok_or_else(|| {
            AuthError::Config(format!(
                "User '{user_id}' has a secret_ref GitHub token but no matching GITHUB_TOKEN secret"
            ))
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::provider_tokens::SECRET_REF_TOKEN_PLACEHOLDER;
    use crate::auth::test_support::{github_row, StubProviderTokens};
    use crate::secret::test_support::{secret, service_with};

    async fn resolve(
        row: Option<ProviderToken>,
        secrets: Option<&SecretService>,
    ) -> Result<Option<String>, AuthError> {
        let apps = GitHubAppRegistry::default();
        resolve_user_github_token(&StubProviderTokens(row), None, None, &apps, secrets, "u1", None).await
    }

    #[tokio::test]
    async fn secret_ref_row_resolves_the_encrypted_pat() {
        let secrets = service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            "GITHUB_TOKEN",
            "ghp_real",
        )]);
        let token = resolve(
            Some(github_row(AUTH_METHOD_SECRET_REF, SECRET_REF_TOKEN_PLACEHOLDER)),
            Some(&secrets),
        )
        .await
        .unwrap();
        assert_eq!(
            token,
            Some("ghp_real".to_string()),
            "the sentinel in access_token must never be returned"
        );
    }

    #[tokio::test]
    async fn dangling_secret_ref_errors_instead_of_reporting_disconnected() {
        // Reference row present, secret gone. Returning Ok(None) here would
        // read as "GitHub not connected" and silently drop auth.
        let secrets = service_with(vec![]);
        let err = resolve(
            Some(github_row(AUTH_METHOD_SECRET_REF, SECRET_REF_TOKEN_PLACEHOLDER)),
            Some(&secrets),
        )
        .await
        .expect_err("dangling reference must be an error");
        assert!(matches!(err, AuthError::Config(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn secret_ref_without_secret_service_errors() {
        let err = resolve(
            Some(github_row(AUTH_METHOD_SECRET_REF, SECRET_REF_TOKEN_PLACEHOLDER)),
            None,
        )
        .await
        .expect_err("unwired secret service must be an error");
        assert!(matches!(err, AuthError::Config(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn env_kind_secret_does_not_satisfy_a_secret_ref_row() {
        // The escape-hatch `env` secret is deliberately not trusted here — a
        // reference row promises a *validated* PAT.
        let secrets = service_with(vec![secret(
            "user",
            Some("u1"),
            "env",
            "GITHUB_TOKEN",
            "ghp_unvalidated",
        )]);
        let err = resolve(
            Some(github_row(AUTH_METHOD_SECRET_REF, SECRET_REF_TOKEN_PLACEHOLDER)),
            Some(&secrets),
        )
        .await
        .expect_err("env-kind secret must not satisfy a secret_ref row");
        assert!(matches!(err, AuthError::Config(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn plain_rows_still_return_the_column_verbatim() {
        // Legacy PAT / OAuth rows are unchanged by the reference-row model.
        let token = resolve(Some(github_row("manual", "ghp_legacy")), None)
            .await
            .unwrap();
        assert_eq!(token, Some("ghp_legacy".to_string()));
    }

    #[tokio::test]
    async fn no_row_means_not_connected() {
        assert_eq!(resolve(None, None).await.unwrap(), None);
    }
}
