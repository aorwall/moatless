//! Git-host credential administration.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_core::git_provider::GitProviderError;
use moatless_core::secret_keys::GITHUB_TOKEN;
use moatless_types::access_control::ScopeContext;

use crate::auth::credentials::{
    GITHUB_OAUTH_STASH_KEY, StashedOAuthCredential, read_oauth_stash, stash_oauth_credential,
};
use crate::auth::models::ProviderToken;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::{
    AUTH_METHOD_APP_INSTALLATION, AUTH_METHOD_APP_USER, AUTH_METHOD_SECRET_REF,
    SECRET_REF_TOKEN_PLACEHOLDER,
};
use crate::secret::error::SecretError;
use crate::secret::schemas::PutSecretRequest;
use crate::secret::service::SecretService;
use moatless_types::secret::SecretKind;

#[derive(Clone, Debug)]
pub struct GitHostPrincipal {
    pub user_id: String,
    pub scope_context: ScopeContext,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubProviderTokenStatus {
    pub connected: bool,
    pub login: Option<String>,
    pub token_type: Option<String>,
    /// How the stored credential mints a token — `provider_tokens.auth_method`.
    /// Says which of the connect paths the person is on, so a screen can name
    /// the credential instead of guessing from `token_type`.
    pub auth_method: Option<String>,
    /// Whether a PAT is standing in front of an App-issued credential that
    /// [`GitHostAccess::restore_github_app_token`] can put back. A PAT with
    /// nothing behind it is the only credential there is, not an override.
    pub has_pat_override: bool,
    /// Whether this deployment can run the App authorization flow at all. False
    /// leaves a PAT as the only way to connect, which is what the screen offers.
    pub app_connect_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitnessProviderTokenStatus {
    pub connected: bool,
    pub login: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum GitHostAccessError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Provider error: {0}")]
    Provider(#[source] GitProviderError),
    #[error("Secret storage error: {0}")]
    Secret(#[from] SecretError),
    /// A fault in the deployment rather than in the request — a missing
    /// encryption key, a registration that cannot be opened. Rendered as a 500
    /// with the cause logged, so a caller is not told to fix its own input.
    #[error("Internal error: {0}")]
    Internal(#[source] crate::auth::error::AuthError),
}

#[async_trait]
pub trait GitHostTokenValidator: Send + Sync {
    async fn validate_github(&self, token: &str) -> Result<String, GitProviderError>;
    async fn validate_gitness(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<String, GitProviderError>;
}

pub struct ProviderGitHostTokenValidator {
    client: reqwest::Client,
}

impl Default for ProviderGitHostTokenValidator {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl GitHostTokenValidator for ProviderGitHostTokenValidator {
    async fn validate_github(&self, token: &str) -> Result<String, GitProviderError> {
        moatless_core::github_client::validate_github_token(token).await
    }

    async fn validate_gitness(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<String, GitProviderError> {
        let base = base_url.trim_end_matches('/');
        let response = self
            .client
            .get(format!("{base}/api/v1/user"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|source| GitProviderError::Unreachable {
                operation: "Gitness API",
                source,
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(GitProviderError::Rejected {
                operation: "Gitness API",
                status,
                body,
            });
        }

        let data: serde_json::Value =
            response
                .json()
                .await
                .map_err(|source| GitProviderError::UndecodableResponse {
                    operation: "Gitness",
                    source,
                })?;
        data.get("login")
            .or_else(|| data.get("uid"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                GitProviderError::MalformedResponse(
                    "Gitness response missing login/uid field".to_string(),
                )
            })
    }
}

pub struct GitHostAccess {
    provider_tokens: Arc<dyn ProviderTokenDao>,
    secrets: Arc<SecretService>,
    validator: Arc<dyn GitHostTokenValidator>,
    /// Start-up state, so it is passed in rather than read per request. See
    /// [`GitHubProviderTokenStatus::app_connect_available`].
    app_connect_available: bool,
}

impl GitHostAccess {
    pub(crate) fn new(
        provider_tokens: Arc<dyn ProviderTokenDao>,
        secrets: Arc<SecretService>,
        validator: Arc<dyn GitHostTokenValidator>,
        app_connect_available: bool,
    ) -> Self {
        Self {
            provider_tokens,
            secrets,
            validator,
            app_connect_available,
        }
    }

    pub async fn github_status(
        &self,
        principal: &GitHostPrincipal,
    ) -> Result<GitHubProviderTokenStatus, GitHostAccessError> {
        let token = self
            .provider_tokens
            .find_provider_token_full(&principal.user_id, "github")
            .await?;
        let Some(row) = token else {
            return Ok(GitHubProviderTokenStatus {
                connected: false,
                login: None,
                token_type: None,
                auth_method: None,
                has_pat_override: false,
                app_connect_available: self.app_connect_available,
            });
        };
        Ok(GitHubProviderTokenStatus {
            connected: true,
            login: metadata_string(row.metadata.as_ref(), "login"),
            has_pat_override: self
                .has_stashed_app_token(&principal.user_id, row.token_type.as_deref())
                .await,
            auth_method: row.auth_method,
            token_type: row.token_type,
            app_connect_available: self.app_connect_available,
        })
    }

    /// Whether an App-issued credential is stashed behind this user's PAT —
    /// that is, whether [`Self::restore_github_app_token`] would hand back a
    /// working credential rather than fail.
    ///
    /// Keyed on the stash itself, not on the row's metadata: the App
    /// authorization flow stashes a displaced PAT's predecessor without ever
    /// writing to the row, so metadata answers for only one of the two writers.
    async fn has_stashed_app_token(&self, user_id: &str, token_type: Option<&str>) -> bool {
        if token_type != Some("pat") {
            return false;
        }
        match read_oauth_stash(&self.secrets, user_id, GITHUB_OAUTH_STASH_KEY).await {
            Ok(stash) => stash.is_some(),
            Err(error) => {
                tracing::warn!(%error, user_id, "Failed to read the GitHub App token stash");
                false
            }
        }
    }

    pub async fn save_github_provider_token(
        &self,
        principal: &GitHostPrincipal,
        token: String,
    ) -> Result<GitHubProviderTokenStatus, GitHostAccessError> {
        let login = self
            .validator
            .validate_github(&token)
            .await
            .map_err(GitHostAccessError::Provider)?;
        let mut metadata = serde_json::json!({ "login": login });

        if let Some(existing) = self
            .provider_tokens
            .find_provider_token_full(&principal.user_id, "github")
            .await?
        {
            if existing.token_type.as_deref() == Some("pat") {
                // Replacing one PAT with another leaves whatever the first one
                // displaced still stashed and still restorable.
                for key in ["oauth_login", "oauth_auth_method"] {
                    if let Some(value) = metadata_string(existing.metadata.as_ref(), key) {
                        metadata[key] = serde_json::Value::String(value);
                    }
                }
            } else if let Some(auth_method) = restorable_auth_method(&existing) {
                // Keyed on the token being real rather than on one auth_method:
                // rows predating the column were backfilled to `manual` by
                // migration 0027 and still hold a live OAuth token, and losing
                // one means the person cannot get back to App access by
                // removing the PAT.
                stash_oauth_credential(
                    &self.secrets,
                    &principal.user_id,
                    GITHUB_OAUTH_STASH_KEY,
                    "github",
                    &StashedOAuthCredential {
                        access_token: existing.access_token.clone(),
                        refresh_token: existing.refresh_token.clone(),
                        expires_at: existing.expires_at,
                    },
                )
                .await?;
                if let Some(value) = metadata_string(existing.metadata.as_ref(), "login") {
                    metadata["oauth_login"] = serde_json::Value::String(value);
                }
                // The row's own method, so a restore puts back what was there.
                metadata["oauth_auth_method"] = serde_json::Value::String(auth_method);
            }
        }

        self.secrets
            .put_secret(
                &principal.scope_context,
                &principal.user_id,
                PutSecretRequest {
                    scope: Some(moatless_api_schemas::enums::Scope::User),
                    key: GITHUB_TOKEN.to_string(),
                    kind: SecretKind::ProviderToken,
                    provider: Some("github".to_string()),
                    value: Some(token),
                    external_ref: None,
                },
            )
            .await?;
        self.provider_tokens
            .upsert_token_with_auth_method(
                &principal.user_id,
                "github",
                AUTH_METHOD_SECRET_REF,
                SECRET_REF_TOKEN_PLACEHOLDER,
                None,
                Some("pat"),
                Some(&metadata),
                None,
            )
            .await?;

        Ok(GitHubProviderTokenStatus {
            connected: true,
            login: Some(login),
            token_type: Some("pat".to_string()),
            auth_method: Some(AUTH_METHOD_SECRET_REF.to_string()),
            has_pat_override: self
                .has_stashed_app_token(&principal.user_id, Some("pat"))
                .await,
            app_connect_available: self.app_connect_available,
        })
    }

    pub async fn disconnect_github(
        &self,
        principal: &GitHostPrincipal,
    ) -> Result<GitHubProviderTokenStatus, GitHostAccessError> {
        self.provider_tokens
            .delete_token(&principal.user_id, "github")
            .await?;
        self.cleanup_github_secrets(&principal.user_id, "GitHub disconnect")
            .await;
        Ok(GitHubProviderTokenStatus {
            connected: false,
            login: None,
            token_type: None,
            auth_method: None,
            has_pat_override: false,
            app_connect_available: self.app_connect_available,
        })
    }

    pub async fn restore_github_app_token(
        &self,
        principal: &GitHostPrincipal,
    ) -> Result<GitHubProviderTokenStatus, GitHostAccessError> {
        let token = self
            .provider_tokens
            .find_provider_token_full(&principal.user_id, "github")
            .await?
            .ok_or_else(|| GitHostAccessError::NotFound("No GitHub token found".to_string()))?;
        if token.token_type.as_deref() != Some("pat") {
            return Err(GitHostAccessError::BadRequest(
                "No PAT override to remove".to_string(),
            ));
        }
        let stashed = read_oauth_stash(&self.secrets, &principal.user_id, GITHUB_OAUTH_STASH_KEY)
            .await?
            .ok_or_else(|| {
                GitHostAccessError::BadRequest(
                    "Cannot restore GitHub App token — original token not preserved".to_string(),
                )
            })?;
        // No fallback to the caller's Moatless login: a screen reading this
        // field renders it as the GitHub account the credential acts as, and
        // absent beats wrong.
        let login = metadata_string(token.metadata.as_ref(), "oauth_login");
        let restored_auth_method = metadata_string(token.metadata.as_ref(), "oauth_auth_method")
            .unwrap_or_else(|| AUTH_METHOD_APP_USER.to_string());
        let metadata = login.as_ref().map_or_else(
            || serde_json::json!({}),
            |login| serde_json::json!({ "login": login }),
        );

        // The refresh token and the expiry travel with the access token: a row
        // restored without them cannot be dated or renewed, so it dies at the
        // stashed token's expiry and `ensure_fresh_token` reads the missing
        // expiry as "still valid" rather than renewing it.
        self.provider_tokens
            .upsert_token_with_auth_method(
                &principal.user_id,
                "github",
                &restored_auth_method,
                &stashed.access_token,
                stashed.refresh_token.as_deref(),
                Some("bearer"),
                Some(&metadata),
                stashed.expires_at,
            )
            .await?;
        self.cleanup_github_secrets(&principal.user_id, "PAT removal")
            .await;

        Ok(GitHubProviderTokenStatus {
            connected: true,
            login,
            token_type: Some("bearer".to_string()),
            auth_method: Some(restored_auth_method),
            has_pat_override: false,
            app_connect_available: self.app_connect_available,
        })
    }

    pub async fn gitness_status(
        &self,
        user_id: &str,
    ) -> Result<GitnessProviderTokenStatus, GitHostAccessError> {
        let token = self
            .provider_tokens
            .find_provider_token_full(user_id, "gitness")
            .await?;
        Ok(match token {
            Some(row) => GitnessProviderTokenStatus {
                connected: true,
                login: metadata_string(row.metadata.as_ref(), "login"),
                base_url: metadata_string(row.metadata.as_ref(), "base_url"),
            },
            None => GitnessProviderTokenStatus {
                connected: false,
                login: None,
                base_url: None,
            },
        })
    }

    pub async fn save_gitness_provider_token(
        &self,
        user_id: &str,
        base_url: String,
        token: String,
    ) -> Result<GitnessProviderTokenStatus, GitHostAccessError> {
        let login = self
            .validator
            .validate_gitness(&base_url, &token)
            .await
            .map_err(GitHostAccessError::Provider)?;
        let metadata = serde_json::json!({ "login": login, "base_url": base_url });
        self.provider_tokens
            .upsert_token(
                user_id,
                "gitness",
                &token,
                None,
                Some("pat"),
                Some(&metadata),
            )
            .await?;
        Ok(GitnessProviderTokenStatus {
            connected: true,
            login: Some(login),
            base_url: Some(base_url),
        })
    }

    pub async fn disconnect_gitness(
        &self,
        user_id: &str,
    ) -> Result<GitnessProviderTokenStatus, GitHostAccessError> {
        self.provider_tokens
            .delete_token(user_id, "gitness")
            .await?;
        Ok(GitnessProviderTokenStatus {
            connected: false,
            login: None,
            base_url: None,
        })
    }

    async fn cleanup_github_secrets(&self, user_id: &str, action: &str) {
        for (key, kind) in [
            (GITHUB_TOKEN, SecretKind::ProviderToken),
            (GITHUB_OAUTH_STASH_KEY, SecretKind::Internal),
        ] {
            if let Err(error) = self
                .secrets
                .delete_user_secret(user_id, key, Some(kind))
                .await
            {
                tracing::warn!(%error, user_id, key, action, "Failed to clean up GitHub secret");
            }
        }
    }
}

/// The `auth_method` to restore `row` under once a PAT override is removed, or
/// `None` when the row holds nothing worth stashing.
///
/// A `secret_ref` row stores [`SECRET_REF_TOKEN_PLACEHOLDER`] rather than a
/// token, and an installation token is minted per install and short-lived —
/// stashing either offers a restore that hands back a string GitHub rejects.
/// Everything else is a real user token, including the `manual` rows migration
/// 0027 backfilled.
fn restorable_auth_method(row: &ProviderToken) -> Option<String> {
    if row.access_token.is_empty() || row.access_token == SECRET_REF_TOKEN_PLACEHOLDER {
        return None;
    }
    match row.auth_method.as_deref() {
        Some(AUTH_METHOD_SECRET_REF | AUTH_METHOD_APP_INSTALLATION) => None,
        // The column is nullable, and only the App login path writes a real
        // token without going through one of the methods above. A floor, not a
        // claim about history: migration 0027 backfilled the rows that predate
        // the column to `manual`.
        None => Some(AUTH_METHOD_APP_USER.to_string()),
        Some(auth_method) => Some(auth_method.to_string()),
    }
}

fn metadata_string(metadata: Option<&serde_json::Value>, key: &str) -> Option<String> {
    metadata
        .and_then(|value| value.get(key))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::auth::models::ProviderToken;
    use crate::auth::test_support::{StubProviderTokens, UpsertedToken};
    use crate::secret::test_support::service_with;

    struct RejectingValidator;

    #[async_trait]
    impl GitHostTokenValidator for RejectingValidator {
        async fn validate_github(&self, _token: &str) -> Result<String, GitProviderError> {
            Err(GitProviderError::MalformedResponse(
                "invalid token".to_string(),
            ))
        }

        async fn validate_gitness(
            &self,
            _base_url: &str,
            _token: &str,
        ) -> Result<String, GitProviderError> {
            Err(GitProviderError::MalformedResponse(
                "invalid token".to_string(),
            ))
        }
    }

    /// Accepts any token as `ada`'s, so a test can reach the write path.
    struct AcceptingValidator;

    #[async_trait]
    impl GitHostTokenValidator for AcceptingValidator {
        async fn validate_github(&self, _token: &str) -> Result<String, GitProviderError> {
            Ok("ada".to_string())
        }

        async fn validate_gitness(
            &self,
            _base_url: &str,
            _token: &str,
        ) -> Result<String, GitProviderError> {
            Ok("ada".to_string())
        }
    }

    fn access(tokens: Arc<StubProviderTokens>) -> GitHostAccess {
        access_with_stash(tokens, None)
    }

    /// The secret store is handed back so a test can read what a save wrote to
    /// it — the stash is the whole subject of these cases.
    fn accepting_access(row: ProviderToken) -> (GitHostAccess, Arc<SecretService>) {
        let secrets = Arc::new(service_with(Vec::new()));
        let access = GitHostAccess::new(
            Arc::new(StubProviderTokens::with_user_row(Some(row))),
            Arc::clone(&secrets),
            Arc::new(AcceptingValidator),
            true,
        );
        (access, secrets)
    }

    /// Save a PAT over `row` and report what was stashed behind it.
    ///
    /// The displaced row is the one a PAT is being set *over*, so it carries
    /// the OAuth `token_type` rather than the `pat` a previous override left.
    async fn stash_after_saving_a_pat_over(
        mut row: ProviderToken,
    ) -> Option<StashedOAuthCredential> {
        row.token_type = Some("bearer".to_string());
        let (access, secrets) = accepting_access(row);
        access
            .save_github_provider_token(&principal(), "ghp_new".to_string())
            .await
            .expect("save the PAT");
        read_oauth_stash(&secrets, "u1", GITHUB_OAUTH_STASH_KEY)
            .await
            .expect("read the stash")
    }

    /// The access token alone, for a case that only cares which token was kept.
    async fn stashed_token_after_saving_a_pat_over(row: ProviderToken) -> Option<String> {
        stash_after_saving_a_pat_over(row)
            .await
            .map(|credential| credential.access_token)
    }

    /// `stash` is the App user token held behind a PAT, which is what makes a
    /// restore possible and so what `has_pat_override` reports.
    fn access_with_stash(tokens: Arc<StubProviderTokens>, stash: Option<&str>) -> GitHostAccess {
        let records = stash
            .map(|token| {
                vec![crate::secret::test_support::secret(
                    "user",
                    Some("u1"),
                    SecretKind::Internal.as_str(),
                    GITHUB_OAUTH_STASH_KEY,
                    token,
                )]
            })
            .unwrap_or_default();
        GitHostAccess::new(
            tokens,
            Arc::new(service_with(records)),
            Arc::new(RejectingValidator),
            true,
        )
    }

    fn principal() -> GitHostPrincipal {
        GitHostPrincipal {
            user_id: "u1".to_string(),
            scope_context: ScopeContext {
                user_id: "u1".to_string(),
                is_admin: false,
            },
        }
    }

    #[tokio::test]
    async fn github_validation_failure_writes_nothing() {
        let tokens = Arc::new(StubProviderTokens::default());
        let result = access(Arc::clone(&tokens))
            .save_github_provider_token(&principal(), "bad".to_string())
            .await;

        assert!(matches!(result, Err(GitHostAccessError::Provider(_))));
        assert_eq!(tokens.writes.load(Ordering::Relaxed), 0);
    }

    /// The principal never says how anyone signed in, so every status test
    /// below runs as somebody who authenticated through OIDC and connected
    /// GitHub separately — the case the old login-provider gate got wrong.
    async fn status(row: Option<ProviderToken>) -> GitHubProviderTokenStatus {
        status_with_stash(row, None).await
    }

    async fn status_with_stash(
        row: Option<ProviderToken>,
        stash: Option<&str>,
    ) -> GitHubProviderTokenStatus {
        access_with_stash(Arc::new(StubProviderTokens::with_user_row(row)), stash)
            .github_status(&principal())
            .await
            .expect("status")
    }

    fn row_with(auth_method: &str, token_type: &str, metadata: serde_json::Value) -> ProviderToken {
        let mut row = crate::auth::test_support::github_row(auth_method, "tok");
        row.token_type = Some(token_type.to_string());
        row.metadata = Some(metadata);
        row
    }

    #[tokio::test]
    async fn an_app_credential_is_not_reported_as_a_pat_override() {
        let status = status(Some(row_with(
            AUTH_METHOD_APP_USER,
            "bearer",
            serde_json::json!({ "login": "ada" }),
        )))
        .await;

        assert!(status.connected);
        assert!(!status.has_pat_override);
        assert_eq!(status.auth_method.as_deref(), Some(AUTH_METHOD_APP_USER));
        assert_eq!(status.login.as_deref(), Some("ada"));
    }

    #[tokio::test]
    async fn an_app_token_is_never_an_override_however_much_is_stashed_behind_it() {
        // A stash outlives nothing here — the live row is the App credential,
        // so there is no PAT to take away.
        let status = status_with_stash(
            Some(row_with(
                AUTH_METHOD_APP_USER,
                "bearer",
                serde_json::json!({ "login": "ada" }),
            )),
            Some("ghu_stashed"),
        )
        .await;

        assert!(!status.has_pat_override);
    }

    #[tokio::test]
    async fn a_pat_standing_in_front_of_an_app_token_is_an_override() {
        let status = status_with_stash(
            Some(row_with(
                AUTH_METHOD_SECRET_REF,
                "pat",
                serde_json::json!({ "login": "ada" }),
            )),
            Some("ghu_stashed"),
        )
        .await;

        assert!(status.has_pat_override);
    }

    #[tokio::test]
    async fn a_pat_with_nothing_behind_it_is_the_only_credential_not_an_override() {
        // Offering "remove override" here would delete the person's only way
        // to reach GitHub and restore nothing in its place.
        let status = status(Some(row_with(
            AUTH_METHOD_SECRET_REF,
            "pat",
            serde_json::json!({ "login": "ada" }),
        )))
        .await;

        assert!(status.connected);
        assert!(!status.has_pat_override);
    }

    #[tokio::test]
    async fn connect_availability_is_reported_to_someone_holding_no_credential() {
        // The page needs it precisely when there is no row: that is when it
        // must choose between offering the App flow and asking for a PAT.
        let status = status(None).await;

        assert!(!status.connected);
        assert!(status.app_connect_available);
        assert!(!status.has_pat_override);
    }

    #[tokio::test]
    async fn a_pat_set_over_a_backfilled_row_keeps_the_token_it_displaced() {
        // Migration 0027 backfilled every pre-existing row to `manual`, and
        // those rows hold a live OAuth token. Skipping the stash for them means
        // removing the PAT later cannot get the person back to App access.
        let row = crate::auth::test_support::github_row("manual", "ghu_displaced");
        assert_eq!(
            stashed_token_after_saving_a_pat_over(row).await.as_deref(),
            Some("ghu_displaced")
        );
    }

    #[tokio::test]
    async fn a_pat_set_over_an_app_credential_keeps_the_token_it_displaced() {
        let row = crate::auth::test_support::github_row(AUTH_METHOD_APP_USER, "ghu_displaced");
        assert_eq!(
            stashed_token_after_saving_a_pat_over(row).await.as_deref(),
            Some("ghu_displaced")
        );
    }

    #[tokio::test]
    async fn a_row_holding_no_real_token_is_not_stashed() {
        // Restoring either would hand back a string GitHub rejects: the
        // `secret_ref` row stores a placeholder, and an installation token is
        // minted per install and short-lived.
        for row in [
            crate::auth::test_support::github_row(
                AUTH_METHOD_SECRET_REF,
                SECRET_REF_TOKEN_PLACEHOLDER,
            ),
            crate::auth::test_support::github_row(AUTH_METHOD_APP_INSTALLATION, "ghs_install"),
            crate::auth::test_support::github_row(AUTH_METHOD_APP_USER, ""),
        ] {
            let auth_method = row.auth_method.clone();
            assert_eq!(
                stash_after_saving_a_pat_over(row).await,
                None,
                "{auth_method:?} must not be offered for restore"
            );
        }
    }

    #[tokio::test]
    async fn a_pat_set_over_an_app_credential_keeps_what_renews_it() {
        // The access token is the short-lived half. Stashing it alone means a
        // restore hands back a credential that expires within hours and that
        // nothing can renew, so the person silently loses GitHub access.
        let mut row = crate::auth::test_support::github_row(AUTH_METHOD_APP_USER, "ghu_displaced");
        row.refresh_token = Some("ghr_displaced".to_string());
        row.expires_at = Some(expiry());

        let stashed = stash_after_saving_a_pat_over(row)
            .await
            .expect("a stashed credential");

        assert_eq!(stashed.refresh_token.as_deref(), Some("ghr_displaced"));
        assert_eq!(stashed.expires_at, Some(expiry()));
    }

    #[tokio::test]
    async fn removing_a_pat_override_restores_what_renews_the_credential() {
        // The row this writes is the one `ensure_fresh_token` reads. Without
        // the expiry it reads as undated and without the refresh token there is
        // nothing to renew it with, so the restored credential dies at the
        // stashed token's expiry and never refreshes.
        let restored = restore(&StashedOAuthCredential {
            access_token: "ghu_stashed".to_string(),
            refresh_token: Some("ghr_stashed".to_string()),
            expires_at: Some(expiry()),
        })
        .await;

        assert_eq!(restored.access_token, "ghu_stashed");
        assert_eq!(restored.refresh_token.as_deref(), Some("ghr_stashed"));
        assert_eq!(restored.expires_at, Some(expiry()));
        assert_eq!(restored.auth_method, AUTH_METHOD_APP_USER);
        assert_eq!(restored.token_type.as_deref(), Some("bearer"));
    }

    #[tokio::test]
    async fn a_legacy_stash_restores_the_only_thing_it_holds() {
        // Stashes written before the credential was stored whole are a bare
        // token. Restoring one is still better than refusing: the person gets
        // back whatever life it has left and can reconnect.
        let restored = restore(&StashedOAuthCredential::from_access_token("ghu_legacy")).await;

        assert_eq!(restored.access_token, "ghu_legacy");
        assert_eq!(restored.refresh_token, None);
        assert_eq!(restored.expires_at, None);
    }

    /// Remove a PAT override standing in front of `stashed`, and report the row
    /// the restore wrote.
    async fn restore(stashed: &StashedOAuthCredential) -> UpsertedToken {
        let row = row_with(
            AUTH_METHOD_SECRET_REF,
            "pat",
            serde_json::json!({ "login": "ada", "oauth_login": "ada" }),
        );
        let tokens = Arc::new(StubProviderTokens::with_user_row(Some(row)));
        let stored = serde_json::to_string(stashed).expect("serialize the stash");
        let access = access_with_stash(Arc::clone(&tokens), Some(&stored));

        access
            .restore_github_app_token(&principal())
            .await
            .expect("restore the App token");

        let recorded = tokens
            .last_auth_method_upsert
            .lock()
            .expect("the recorded write")
            .clone();
        recorded.expect("the restore wrote a row")
    }

    fn expiry() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_800_000_000, 0).expect("a valid time")
    }

    #[tokio::test]
    async fn gitness_validation_failure_writes_nothing() {
        let tokens = Arc::new(StubProviderTokens::default());
        let result = access(Arc::clone(&tokens))
            .save_gitness_provider_token(
                "u1",
                "https://gitness.example".to_string(),
                "bad".to_string(),
            )
            .await;

        assert!(matches!(result, Err(GitHostAccessError::Provider(_))));
        assert_eq!(tokens.writes.load(Ordering::Relaxed), 0);
    }
}
