//! Administering one person's credential for one Forgejo instance.
//!
//! The GitHub counterpart is [`crate::git_host_access`], and the two share a
//! shape deliberately: a personal access token is stored as a secret behind a
//! `secret_ref` row, and setting one stashes whatever OAuth credential it
//! displaced so removing it restores a live one. What differs is addressing —
//! every row, secret and stash here is keyed by the instance's host.

use std::sync::Arc;

use moatless_core::secret_keys::{forgejo_oauth_stash_key, forgejo_token_key};

use crate::auth::credentials::{StashedOAuthCredential, read_oauth_stash, stash_oauth_credential};
use crate::auth::error::AuthError;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::{
    AUTH_METHOD_FORGEJO_OAUTH, AUTH_METHOD_SECRET_REF, SECRET_REF_TOKEN_PLACEHOLDER,
    forgejo_provider_key,
};
use crate::forgejo::instances::{ForgejoInstances, normalize_host};
use crate::forgejo::oauth::{ForgejoOAuth, ForgejoOAuthTokens};
use crate::git_host_access::{GitHostAccessError, GitHostPrincipal};
use crate::secret::schemas::PutSecretRequest;
use crate::secret::service::SecretService;
use moatless_types::secret::SecretKind;

/// One person's connection to one Forgejo instance.
///
/// There is no login: a `read:repository` token cannot reach Forgejo's `user`
/// endpoint, so [`crate::forgejo::api::validate_token`] only confirms the
/// credential works; a screen names [`Self::host`] instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgejoProviderTokenStatus {
    pub host: String,
    pub connected: bool,
    /// How the stored credential resolves — `provider_tokens.auth_method`.
    pub auth_method: Option<String>,
    /// Whether a personal access token is standing in front of an OAuth
    /// credential that [`ForgejoAccess::restore_oauth_token`] can put back.
    pub has_pat_override: bool,
    /// Whether this instance has a registered OAuth2 application. False leaves
    /// a personal access token as the only way to connect, which is what the
    /// screen offers.
    pub oauth_available: bool,
}

pub struct ForgejoAccess {
    provider_tokens: Arc<dyn ProviderTokenDao>,
    secrets: Arc<SecretService>,
    instances: Arc<ForgejoInstances>,
    oauth: ForgejoOAuth,
    http: reqwest::Client,
}

impl ForgejoAccess {
    pub(crate) fn new(
        provider_tokens: Arc<dyn ProviderTokenDao>,
        secrets: Arc<SecretService>,
        instances: Arc<ForgejoInstances>,
        timeouts: moatless_types::http::HttpTimeouts,
    ) -> Self {
        Self {
            provider_tokens,
            secrets,
            instances,
            oauth: ForgejoOAuth::new(timeouts),
            http: super::http_client(timeouts),
        }
    }

    pub async fn status(
        &self,
        user_id: &str,
        host: &str,
    ) -> Result<ForgejoProviderTokenStatus, GitHostAccessError> {
        let host = normalize_host(host);
        let row = self
            .provider_tokens
            .find_provider_token_full(user_id, &forgejo_provider_key(&host))
            .await?;
        let oauth_available = self.oauth_available(&host).await;
        let Some(row) = row else {
            return Ok(ForgejoProviderTokenStatus {
                host,
                connected: false,
                auth_method: None,
                has_pat_override: false,
                oauth_available,
            });
        };
        Ok(ForgejoProviderTokenStatus {
            has_pat_override: row.auth_method.as_deref() == Some(AUTH_METHOD_SECRET_REF)
                && self.has_stashed_oauth_token(user_id, &host).await,
            connected: true,
            auth_method: row.auth_method,
            host,
            oauth_available,
        })
    }

    /// Store a personal access token for `host`, stashing whatever OAuth
    /// credential it displaces.
    pub async fn save_personal_access_token(
        &self,
        principal: &GitHostPrincipal,
        host: &str,
        token: String,
    ) -> Result<ForgejoProviderTokenStatus, GitHostAccessError> {
        let host = normalize_host(host);
        crate::forgejo::api::validate_token(&self.http, &host, &token)
            .await
            .map_err(GitHostAccessError::Provider)?;

        if let Some(existing) = self
            .provider_tokens
            .find_provider_token_full(&principal.user_id, &forgejo_provider_key(&host))
            .await?
        {
            // Replacing one token with another leaves whatever the first
            // displaced still stashed and still restorable.
            if existing.auth_method.as_deref() == Some(AUTH_METHOD_FORGEJO_OAUTH)
                && !existing.access_token.is_empty()
            {
                stash_oauth_credential(
                    &self.secrets,
                    &principal.user_id,
                    &forgejo_oauth_stash_key(&host),
                    "forgejo",
                    &StashedOAuthCredential {
                        access_token: existing.access_token.clone(),
                        refresh_token: existing.refresh_token.clone(),
                        expires_at: existing.expires_at,
                    },
                )
                .await?;
            }
        }

        self.secrets
            .put_secret(
                &principal.scope_context,
                &principal.user_id,
                PutSecretRequest {
                    scope: Some(moatless_api_schemas::enums::Scope::User),
                    key: forgejo_token_key(&host),
                    kind: SecretKind::ProviderToken,
                    provider: Some("forgejo".to_string()),
                    value: Some(token),
                    external_ref: None,
                },
            )
            .await?;
        self.provider_tokens
            .upsert_token_with_auth_method(
                &principal.user_id,
                &forgejo_provider_key(&host),
                AUTH_METHOD_SECRET_REF,
                SECRET_REF_TOKEN_PLACEHOLDER,
                None,
                Some("pat"),
                Some(&serde_json::json!({ "host": host })),
                None,
            )
            .await?;

        Ok(ForgejoProviderTokenStatus {
            has_pat_override: self
                .has_stashed_oauth_token(&principal.user_id, &host)
                .await,
            oauth_available: self.oauth_available(&host).await,
            host,
            connected: true,
            auth_method: Some(AUTH_METHOD_SECRET_REF.to_string()),
        })
    }

    /// Store the credential an authorization code bought.
    ///
    /// `redirect_uri` must be the one the authorization request carried.
    pub async fn complete_connect(
        &self,
        principal: &GitHostPrincipal,
        host: &str,
        code: &str,
        redirect_uri: &str,
    ) -> Result<ForgejoProviderTokenStatus, GitHostAccessError> {
        let host = normalize_host(host);
        let app = self.registered_app(&host).await?;
        let tokens = self
            .oauth
            .exchange_code(&app, code, redirect_uri)
            .await
            .map_err(GitHostAccessError::Provider)?;
        self.write_oauth_row(&principal.user_id, &host, &tokens)
            .await?;
        self.status(&principal.user_id, &host).await
    }

    /// Where to send `principal` to authorize this deployment against `host`.
    pub async fn authorize_url(
        &self,
        host: &str,
        redirect_uri: &str,
        state: &str,
    ) -> Result<String, GitHostAccessError> {
        let app = self.registered_app(&normalize_host(host)).await?;
        Ok(ForgejoOAuth::authorize_url(&app, redirect_uri, state))
    }

    /// Drop `user_id`'s credential for `host` altogether, stash included.
    pub async fn disconnect(
        &self,
        user_id: &str,
        host: &str,
    ) -> Result<ForgejoProviderTokenStatus, GitHostAccessError> {
        let host = normalize_host(host);
        self.provider_tokens
            .delete_token(user_id, &forgejo_provider_key(&host))
            .await?;
        self.cleanup_secrets(user_id, &host, "Forgejo disconnect")
            .await;
        Ok(ForgejoProviderTokenStatus {
            oauth_available: self.oauth_available(&host).await,
            host,
            connected: false,
            auth_method: None,
            has_pat_override: false,
        })
    }

    /// Remove the personal access token overriding `host`'s OAuth credential,
    /// putting the stashed credential back.
    pub async fn restore_oauth_token(
        &self,
        user_id: &str,
        host: &str,
    ) -> Result<ForgejoProviderTokenStatus, GitHostAccessError> {
        let host = normalize_host(host);
        let stashed = read_oauth_stash(&self.secrets, user_id, &forgejo_oauth_stash_key(&host))
            .await?
            .ok_or_else(|| {
                GitHostAccessError::BadRequest(format!(
                    "Cannot restore the {host} OAuth credential — no credential was preserved"
                ))
            })?;
        self.write_oauth_row(
            user_id,
            &host,
            &ForgejoOAuthTokens {
                access_token: stashed.access_token,
                refresh_token: stashed.refresh_token,
                expires_at: stashed.expires_at,
            },
        )
        .await?;
        self.cleanup_secrets(user_id, &host, "Forgejo PAT removal")
            .await;
        self.status(user_id, &host).await
    }

    async fn write_oauth_row(
        &self,
        user_id: &str,
        host: &str,
        tokens: &ForgejoOAuthTokens,
    ) -> Result<(), GitHostAccessError> {
        // The refresh token and the expiry travel with the access token: a row
        // restored without them cannot be dated or renewed, so it dies at the
        // stored token's expiry with nothing able to notice.
        self.provider_tokens
            .upsert_token_with_auth_method(
                user_id,
                &forgejo_provider_key(host),
                AUTH_METHOD_FORGEJO_OAUTH,
                &tokens.access_token,
                tokens.refresh_token.as_deref(),
                Some("bearer"),
                Some(&serde_json::json!({ "host": host })),
                tokens.expires_at,
            )
            .await?;
        Ok(())
    }

    async fn registered_app(
        &self,
        host: &str,
    ) -> Result<crate::forgejo::instances::ForgejoOAuthApp, GitHostAccessError> {
        self.instances
            .app(host)
            .await
            .map_err(auth_to_access_error)?
            .ok_or_else(|| {
                GitHostAccessError::NotFound(format!(
                    "No Forgejo OAuth application is registered for {host}"
                ))
            })
    }

    /// Whether `host` has a registered application to connect through. A read
    /// that fails is reported as unavailable and logged: this only decides
    /// which buttons a screen offers.
    async fn oauth_available(&self, host: &str) -> bool {
        match self.instances.app_available(host).await {
            Ok(available) => available,
            Err(error) => {
                tracing::warn!(%error, host, "Failed to read the Forgejo OAuth application");
                false
            }
        }
    }

    async fn has_stashed_oauth_token(&self, user_id: &str, host: &str) -> bool {
        match read_oauth_stash(&self.secrets, user_id, &forgejo_oauth_stash_key(host)).await {
            Ok(stash) => stash.is_some(),
            Err(error) => {
                tracing::warn!(%error, user_id, host, "Failed to read the Forgejo OAuth stash");
                false
            }
        }
    }

    async fn cleanup_secrets(&self, user_id: &str, host: &str, action: &str) {
        for (key, kind) in [
            (forgejo_token_key(host), SecretKind::ProviderToken),
            (forgejo_oauth_stash_key(host), SecretKind::Internal),
        ] {
            if let Err(error) = self
                .secrets
                .delete_user_secret(user_id, &key, Some(kind))
                .await
            {
                tracing::warn!(%error, user_id, key, action, "Failed to clean up a Forgejo secret");
            }
        }
    }
}

/// A registration that could not be read is an internal fault, not a missing
/// one — the caller must not offer "register an application" for it.
fn auth_to_access_error(error: AuthError) -> GitHostAccessError {
    match error {
        AuthError::BadRequest(message) => GitHostAccessError::BadRequest(message),
        AuthError::NotFound(message) => GitHostAccessError::NotFound(message),
        AuthError::Database(source) => GitHostAccessError::Db(source),
        other => GitHostAccessError::Internal(other),
    }
}
