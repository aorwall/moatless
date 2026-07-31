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
use super::github::registry::GitHubAppRegistry;
use super::github::service::GitHubAppService;
use super::models::ProviderToken;
use super::provider_token_dao::ProviderTokenDao;
use super::provider_tokens::{
    ProviderTokenAuthDao, AUTH_METHOD_APP_INSTALLATION, AUTH_METHOD_SECRET_REF,
};
use super::token_refresh::ensure_fresh_token;
use crate::credentials::github::scope::GitHubScope;
use crate::sandbox::secrets::GITHUB_TOKEN;
use crate::secret::error::SecretError;
use crate::secret::models::SecretKind;
use crate::secret::schemas::PutSecretRequest;
use crate::secret::service::SecretService;
use moatless_backend_core::scope::{Scope, ScopeContext};

/// Resolve the GitHub token a given user should present to GitHub at this
/// moment, from the user's own scope or the org-wide global one.
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
/// Returns `Ok(None)` only when there is no `provider_tokens` row at any scope
/// reachable from `user_id` — including the org-wide global row.
pub async fn resolve_user_github_token(
    provider_tokens: &dyn ProviderTokenDao,
    provider_token_auth: Option<&dyn ProviderTokenAuthDao>,
    github_app_service: Option<&GitHubAppService>,
    github_apps: &GitHubAppRegistry,
    secrets: Option<&SecretService>,
    user_id: &str,
    repo_owner: Option<&str>,
) -> Result<Option<String>, AuthError> {
    Ok(resolve_github_token_for_scopes(
        provider_tokens,
        provider_token_auth,
        github_app_service,
        github_apps,
        secrets,
        user_id,
        repo_owner,
    )
    .await?
    .map(|resolved| resolved.token))
}

/// A GitHub token plus the scope it came from.
pub struct ResolvedGithubToken {
    pub token: String,
    pub scope: GitHubScope,
    /// GitHub login recorded on the winning row's metadata, when known.
    pub login: Option<String>,
}

/// Resolve the GitHub token for `user_id`, walking `user > global`.
///
/// The walk is what makes an admin-set org-wide token usable: a user with no
/// credential of their own falls through to the single global row.
#[allow(clippy::too_many_arguments)]
// reason: mirrors `resolve_user_github_token`'s dependency list; bundling
// would only move the argument count to the call sites.
pub async fn resolve_github_token_for_scopes(
    provider_tokens: &dyn ProviderTokenDao,
    provider_token_auth: Option<&dyn ProviderTokenAuthDao>,
    github_app_service: Option<&GitHubAppService>,
    github_apps: &GitHubAppRegistry,
    secrets: Option<&SecretService>,
    user_id: &str,
    repo_owner: Option<&str>,
) -> Result<Option<ResolvedGithubToken>, AuthError> {
    let Some(presence) = provider_tokens
        .find_github_scope_presence(user_id)
        .await
        .map_err(AuthError::Database)?
    else {
        return Ok(None);
    };

    let scope = GitHubScope::from_row(
        &presence.scope,
        presence.user_id.as_deref(),
        presence.team_id.as_deref(),
    );

    let Some(row) = provider_tokens
        .find_github_row(&scope)
        .await
        .map_err(AuthError::Database)?
    else {
        // The presence query just saw this row; a miss now means it was
        // deleted mid-walk. Treat as not connected rather than erroring.
        return Ok(None);
    };

    let token = resolve_github_row_token(
        provider_tokens,
        provider_token_auth,
        github_app_service,
        github_apps,
        secrets,
        &row,
        &scope,
        user_id,
        repo_owner,
        None,
    )
    .await?;

    Ok(Some(ResolvedGithubToken {
        token,
        scope,
        login: presence.login,
    }))
}

/// Dispatch one `provider_tokens` row to the token it stands for.
///
/// Branches by `auth_method`; see [`resolve_user_github_token`] for the table.
#[allow(clippy::too_many_arguments)]
// reason: the dependency list is the union of every branch's needs; bundling it
// into a struct would only move the argument count to the construction site.
async fn resolve_github_row_token(
    provider_tokens: &dyn ProviderTokenDao,
    provider_token_auth: Option<&dyn ProviderTokenAuthDao>,
    github_app_service: Option<&GitHubAppService>,
    github_apps: &GitHubAppRegistry,
    secrets: Option<&SecretService>,
    row: &ProviderToken,
    scope: &GitHubScope,
    user_id: &str,
    repo_owner: Option<&str>,
    repo_name: Option<&str>,
) -> Result<String, AuthError> {
    match row.auth_method.as_deref() {
        Some(AUTH_METHOD_APP_INSTALLATION) => {
            mint_installation_token(row, github_apps, repo_owner, repo_name).await
        }
        Some(AUTH_METHOD_SECRET_REF) => read_secret_ref_token(secrets, scope).await,
        Some("github_app") => {
            if let (Some(app), Some(auth)) = (github_app_service, provider_token_auth) {
                match ensure_fresh_token(provider_tokens, auth, app, user_id).await {
                    Ok(token) => Ok(token),
                    Err(e) => {
                        warn!(
                            error = %e,
                            user_id,
                            "GitHub App token refresh failed, falling back to raw token"
                        );
                        Ok(row.access_token.clone())
                    }
                }
            } else {
                Ok(row.access_token.clone())
            }
        }
        _ => Ok(row.access_token.clone()),
    }
}

/// Resolve the one GitHub credential row selected by a persisted repository
/// scope. This deliberately performs no scope walk.
#[allow(clippy::too_many_arguments)]
// reason: mirrors the canonical row resolver's infrastructure dependencies.
pub(crate) async fn resolve_repository_github_scope_token(
    provider_tokens: &dyn ProviderTokenDao,
    provider_token_auth: Option<&dyn ProviderTokenAuthDao>,
    github_app_service: Option<&GitHubAppService>,
    github_apps: &GitHubAppRegistry,
    secrets: Option<&SecretService>,
    scope: &GitHubScope,
    repo_owner: &str,
    repo_name: &str,
) -> Result<Option<String>, AuthError> {
    let Some(row) = provider_tokens
        .find_github_row(scope)
        .await
        .map_err(AuthError::Database)?
    else {
        return Ok(None);
    };

    match scope {
        GitHubScope::User(_)
            if row.auth_method.as_deref() == Some(AUTH_METHOD_APP_INSTALLATION) =>
        {
            return Err(AuthError::Config(
                "User-scoped repositories require the repository owner's PAT or OAuth token"
                    .to_string(),
            ));
        }
        GitHubScope::Global
            if row.auth_method.as_deref() != Some(AUTH_METHOD_APP_INSTALLATION) =>
        {
            return Err(AuthError::Config(format!(
                "GitHub credential at scope '{scope}' is not a GitHub App installation binding"
            )));
        }
        GitHubScope::Global => {
            let metadata = row.metadata.as_ref().ok_or_else(|| {
                AuthError::Config(format!(
                    "GitHub App installation binding at scope '{scope}' has no metadata"
                ))
            })?;
            if metadata
                .get("installation_id")
                .and_then(serde_json::Value::as_u64)
                .is_none()
                || metadata
                    .get("account_login")
                    .and_then(serde_json::Value::as_str)
                    .is_none()
            {
                return Err(AuthError::Config(format!(
                    "GitHub App installation binding at scope '{scope}' is malformed"
                )));
            }
        }
        _ => {}
    }

    let user_id = match scope {
        GitHubScope::User(user_id) => user_id.as_str(),
        GitHubScope::Global => "",
    };
    resolve_github_row_token(
        provider_tokens,
        provider_token_auth,
        github_app_service,
        github_apps,
        secrets,
        &row,
        scope,
        user_id,
        Some(repo_owner),
        Some(repo_name),
    )
    .await
    .map(Some)
}

/// Resolve the GitHub credential belonging to a task's run-as user, with no
/// scope walk — the global row is never consulted.
///
/// `Ok(None)` means the user holds no row of their own, leaving the caller to
/// decide whether to fall back. A bot row carries no `installation_id`, so it
/// mints an owner-wide installation token and the sandbox reaches every repo
/// the App is installed on rather than only the task's own repository.
#[allow(clippy::too_many_arguments)]
// reason: mirrors the canonical row resolver's infrastructure dependencies.
pub(crate) async fn resolve_run_as_user_github_token(
    provider_tokens: &dyn ProviderTokenDao,
    provider_token_auth: Option<&dyn ProviderTokenAuthDao>,
    github_app_service: Option<&GitHubAppService>,
    github_apps: &GitHubAppRegistry,
    secrets: Option<&SecretService>,
    user_id: &str,
    repo_owner: &str,
    repo_name: &str,
) -> Result<Option<String>, AuthError> {
    let scope = GitHubScope::user(user_id);
    let Some(row) = provider_tokens
        .find_github_row(&scope)
        .await
        .map_err(AuthError::Database)?
    else {
        return Ok(None);
    };

    resolve_github_row_token(
        provider_tokens,
        provider_token_auth,
        github_app_service,
        github_apps,
        secrets,
        &row,
        &scope,
        user_id,
        Some(repo_owner),
        Some(repo_name),
    )
    .await
    .map(Some)
}

/// Map a token scope onto the secret scope holding its encrypted value.
pub(crate) fn secret_scope_for(scope: &GitHubScope) -> (Scope, Option<&str>) {
    match scope {
        GitHubScope::User(id) => (Scope::User, Some(id.as_str())),
        GitHubScope::Global => (Scope::Global, None),
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
                scope: Some(Scope::User),
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
    Ok(resolve_github_token_for_scopes(
        state.provider_tokens.as_ref(),
        Some(state.provider_token_auth.as_ref()),
        state.github_app_service.as_deref(),
        state.github_apps.as_ref(),
        Some(state.secrets.as_ref()),
        user_id,
        repo_owner,
    )
    .await?
    .map(|resolved| resolved.token))
}

/// Read the encrypted PAT a `secret_ref` row points at.
///
/// Every failure mode is an error rather than `None`: a reference row with no
/// resolvable secret means the user *is* connected but the credential is
/// unreachable, and reporting that as "not connected" would send callers down
/// an anonymous path instead of surfacing the breakage.
async fn read_secret_ref_token(
    secrets: Option<&SecretService>,
    scope: &GitHubScope,
) -> Result<String, AuthError> {
    let secrets = secrets.ok_or_else(|| {
        AuthError::Config(format!(
            "GitHub token at scope '{scope}' is a secret_ref but no secret service is wired"
        ))
    })?;
    let (scope_type, scope_id) = secret_scope_for(scope);
    secrets
        .resolve_scope_secret(
            scope_type,
            scope_id,
            GITHUB_TOKEN,
            Some(SecretKind::ProviderToken),
        )
        .await
        .map_err(|e| {
            AuthError::Config(format!(
                "Failed to read GitHub token secret at scope '{scope}': {e}"
            ))
        })?
        .ok_or_else(|| {
            AuthError::Config(format!(
                "GitHub token at scope '{scope}' is a secret_ref but no matching GITHUB_TOKEN secret exists"
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
    repo_name: Option<&str>,
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
    if let Some(installation_id) = row
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("installation_id"))
        .and_then(serde_json::Value::as_u64)
    {
        let bound_login = row
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("account_login"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AuthError::Config(format!(
                    "GitHub App binding for installation {installation_id} is missing account_login"
                ))
            })?;
        if !bound_login.eq_ignore_ascii_case(owner) {
            return Err(AuthError::Config(format!(
                "Repository owner '{owner}' does not match bound GitHub installation account '{bound_login}'"
            )));
        }
        let repository = repo_name.ok_or_else(|| {
            AuthError::Config(format!(
                "Cannot mint repository-restricted installation token for '{owner}': no repository name in context"
            ))
        })?;
        let app = github_apps.get(app_key).await?.ok_or_else(|| {
            AuthError::Config(format!(
                "Unknown GitHub App key '{app_key}' in installation binding"
            ))
        })?;
        return app
            .get_repository_installation_token(installation_id, repository)
            .await;
    }

    let app = github_apps.get(app_key).await?.ok_or_else(|| {
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
        resolve_user_github_token(
            &StubProviderTokens::with_user_row(row),
            None,
            None,
            &apps,
            secrets,
            "u1",
            None,
        )
        .await
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
            Some(github_row(
                AUTH_METHOD_SECRET_REF,
                SECRET_REF_TOKEN_PLACEHOLDER,
            )),
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
            Some(github_row(
                AUTH_METHOD_SECRET_REF,
                SECRET_REF_TOKEN_PLACEHOLDER,
            )),
            Some(&secrets),
        )
        .await
        .expect_err("dangling reference must be an error");
        assert!(matches!(err, AuthError::Config(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn secret_ref_without_secret_service_errors() {
        let err = resolve(
            Some(github_row(
                AUTH_METHOD_SECRET_REF,
                SECRET_REF_TOKEN_PLACEHOLDER,
            )),
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
            Some(github_row(
                AUTH_METHOD_SECRET_REF,
                SECRET_REF_TOKEN_PLACEHOLDER,
            )),
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
    async fn no_row_at_any_scope_means_not_connected() {
        assert_eq!(resolve(None, None).await.unwrap(), None);
    }

    // --- scope walk: user > global ---

    async fn resolve_scoped(tokens: StubProviderTokens) -> Option<ResolvedGithubToken> {
        let apps = GitHubAppRegistry::default();
        resolve_github_token_for_scopes(&tokens, None, None, &apps, None, "u1", None)
            .await
            .expect("scope walk must not error")
    }

    #[tokio::test]
    async fn global_row_is_used_when_the_user_has_none() {
        // The point of the org-wide token: a user who never connected GitHub
        // still gets one.
        let resolved = resolve_scoped(
            StubProviderTokens::default().with_global_row(github_row("manual", "ghp_org")),
        )
        .await
        .expect("global row must satisfy a user with no token");
        assert_eq!(resolved.token, "ghp_org");
        assert_eq!(resolved.scope, GitHubScope::Global);
    }

    #[tokio::test]
    async fn user_row_wins_over_global() {
        let resolved = resolve_scoped(
            StubProviderTokens::with_user_row(Some(github_row("manual", "ghp_mine")))
                .with_global_row(github_row("manual", "ghp_org")),
        )
        .await
        .expect("user row must resolve");
        assert_eq!(resolved.token, "ghp_mine");
        assert_eq!(resolved.scope, GitHubScope::user("u1"));
    }

    #[tokio::test]
    async fn global_secret_ref_reads_the_global_secret_not_the_user_one() {
        // A global reference row must resolve against the global secret scope.
        // Reading the user scope here would return the wrong org's token — or
        // nothing, which reads as "not connected".
        let secrets = service_with(vec![
            secret(
                "global",
                None,
                "provider_token",
                "GITHUB_TOKEN",
                "ghp_org_secret",
            ),
            secret(
                "user",
                Some("u1"),
                "provider_token",
                "GITHUB_TOKEN",
                "ghp_user_secret",
            ),
        ]);
        let apps = GitHubAppRegistry::default();
        let tokens = StubProviderTokens::default().with_global_row(github_row(
            AUTH_METHOD_SECRET_REF,
            SECRET_REF_TOKEN_PLACEHOLDER,
        ));
        let resolved = resolve_github_token_for_scopes(
            &tokens,
            None,
            None,
            &apps,
            Some(&secrets),
            "u1",
            None,
        )
        .await
        .expect("global secret_ref must resolve")
        .expect("global row present");
        assert_eq!(resolved.token, "ghp_org_secret");
    }

    #[tokio::test]
    async fn repository_user_scope_never_falls_through_to_global() {
        let tokens =
            StubProviderTokens::default().with_global_row(github_row("manual", "ghp_global"));
        let apps = GitHubAppRegistry::default();
        let resolved = resolve_repository_github_scope_token(
            &tokens,
            None,
            None,
            &apps,
            None,
            &GitHubScope::user("owner"),
            "acme",
            "repo",
        )
        .await
        .expect("missing exact row is not an error");
        assert_eq!(resolved, None);
    }

    #[tokio::test]
    async fn repository_installation_binding_requires_verified_metadata() {
        let mut row = github_row(
            AUTH_METHOD_APP_INSTALLATION,
            crate::auth::provider_tokens::APP_INSTALLATION_TOKEN_PLACEHOLDER,
        );
        row.provider_user_id = Some("default".to_string());
        let tokens = StubProviderTokens::default().with_global_row(row);
        let apps = GitHubAppRegistry::default();
        let error = resolve_repository_github_scope_token(
            &tokens,
            None,
            None,
            &apps,
            None,
            &GitHubScope::Global,
            "acme",
            "repo",
        )
        .await
        .expect_err("missing installation metadata must fail");
        assert!(error.to_string().contains("metadata"));
    }

    #[tokio::test]
    async fn repository_owner_must_match_bound_installation_account() {
        let mut row = github_row(
            AUTH_METHOD_APP_INSTALLATION,
            crate::auth::provider_tokens::APP_INSTALLATION_TOKEN_PLACEHOLDER,
        );
        row.provider_user_id = Some("default".to_string());
        row.metadata = Some(serde_json::json!({
            "installation_id": 42,
            "account_login": "different-owner",
            "account_type": "Organization",
        }));
        let tokens = StubProviderTokens::default().with_global_row(row);
        let apps = GitHubAppRegistry::default();
        let error = resolve_repository_github_scope_token(
            &tokens,
            None,
            None,
            &apps,
            None,
            &GitHubScope::Global,
            "acme",
            "repo",
        )
        .await
        .expect_err("account mismatch must fail before minting");
        assert!(error.to_string().contains("does not match"));
    }
}
