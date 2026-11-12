//! Issuing a git-host token for one named identity.
//!
//! [`crate::git_host_access`] administers connections; this module resolves
//! their credentials for a git-host account or instance.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::warn;

use crate::git_credential_overrides::GitCredentialOverrides;
use moatless_core::secret_keys::{GITHUB_TOKEN, forgejo_token_key};
use moatless_ports::identity::{AppWriteToken, GitHostTokenError, GitHostTokenIssuer, GitIdentity};
use moatless_types::git_host::{GitHostAccount, RepositoryProvider};
use moatless_types::store::StoreError;

use crate::auth::credentials::{
    GITHUB_OAUTH_STASH_KEY, StashedOAuthCredential, read_oauth_stash, stash_oauth_credential,
};
use crate::auth::error::AuthError;
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::repository_role::RepositoryRole;
use crate::auth::github::service::GitHubAppService;
use crate::auth::models::ProviderToken;
use crate::auth::provider_token_auth_dao::ProviderTokenAuthDao;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::{
    AUTH_METHOD_APP_INSTALLATION, AUTH_METHOD_APP_USER, AUTH_METHOD_FORGEJO_OAUTH,
    AUTH_METHOD_SECRET_REF, forgejo_provider_key,
};
use crate::auth::token_refresh::ensure_fresh_token;
use crate::forgejo::instances::ForgejoInstances;
use crate::forgejo::oauth::ForgejoOAuth;
use crate::secret::service::SecretService;
use moatless_types::access_control::Actor;
use moatless_types::access_control::Scope;
use moatless_types::attribution::Attribution;
use moatless_types::secret::SecretKind;

/// The one way to obtain a git-host token.
///
/// Constructed once at the composition root, so the infrastructure ports below
/// are supplied in a single place rather than threaded through every caller.
pub struct GitHostTokens {
    provider_tokens: Arc<dyn ProviderTokenDao>,
    provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
    github_app_service: Option<Arc<GitHubAppService>>,
    github_apps: Arc<GitHubAppRegistry>,
    secrets: Arc<SecretService>,
    /// The credentials a Workspace pins. `None` in a deployment that wires no
    /// workspace credential store, which resolves every identity to its own
    /// credential.
    credential_overrides: Option<Arc<dyn GitCredentialOverrides>>,
    /// `None` in a deployment that wires no Forgejo registry, which leaves a
    /// stored Forgejo credential to be presented until its host rejects it.
    forgejo: Option<ForgejoRefresh>,
    /// `AuthConfig::github_pats_disabled`: a PAT row presents the App
    /// credential stashed behind it, never the PAT.
    github_pats_disabled: bool,
}

/// The two handles renewing a Forgejo OAuth credential needs: the instance's
/// registered application, and the endpoint to exchange against.
struct ForgejoRefresh {
    instances: Arc<ForgejoInstances>,
    oauth: ForgejoOAuth,
}

impl GitHostTokens {
    pub(crate) fn new(
        provider_tokens: Arc<dyn ProviderTokenDao>,
        provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
        github_app_service: Option<Arc<GitHubAppService>>,
        github_apps: Arc<GitHubAppRegistry>,
        secrets: Arc<SecretService>,
    ) -> Self {
        Self {
            provider_tokens,
            provider_token_auth,
            github_app_service,
            github_apps,
            secrets,
            credential_overrides: None,
            forgejo: None,
            github_pats_disabled: false,
        }
    }

    /// Wire the store that answers what a Workspace pins.
    #[must_use]
    pub(crate) fn with_credential_overrides(
        mut self,
        credential_overrides: Arc<dyn GitCredentialOverrides>,
    ) -> Self {
        self.credential_overrides = Some(credential_overrides);
        self
    }

    /// Refuse to present a GitHub personal access token.
    #[must_use]
    pub(crate) fn with_github_pats_disabled(mut self, disabled: bool) -> Self {
        self.github_pats_disabled = disabled;
        self
    }

    /// Wire the registry a Forgejo OAuth credential is renewed through.
    #[must_use]
    pub(crate) fn with_forgejo(
        mut self,
        instances: Arc<ForgejoInstances>,
        timeouts: moatless_types::http::HttpTimeouts,
    ) -> Self {
        self.forgejo = Some(ForgejoRefresh {
            instances,
            oauth: ForgejoOAuth::new(timeouts),
        });
        self
    }

    /// The credential `actor`'s Workspace pins for `provider`, if it pins one.
    ///
    /// A Workspace that pins a credential whose value cannot be read is an
    /// error rather than an absent pin: falling through to the identity's own
    /// token would make the call as a different account than the Workspace
    /// names, which is the failure this seam exists to remove.
    async fn pinned_token(
        &self,
        actor: &Actor,
        provider: RepositoryProvider,
    ) -> Result<Option<String>, GitHostTokenError> {
        let (Some(store), Some(workspace_id)) =
            (self.credential_overrides.as_ref(), actor.workspace_id())
        else {
            return Ok(None);
        };
        let token = store
            .token(workspace_id, provider)
            .await
            .map_err(|source| GitHostTokenError::Broken {
                actor: actor.clone(),
                provider,
                source: Box::new(source),
            })?;
        Ok(token.filter(|token| !token.is_empty()))
    }

    /// The single stored value `actor` holds for a plain host.
    async fn stored_host_token(
        &self,
        actor: &Actor,
        provider: RepositoryProvider,
    ) -> Result<String, GitHostTokenError> {
        self.provider_tokens
            .find_provider_token(actor.user_id(), provider.as_str())
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read the {provider} token for {actor}"), e)
            })?
            .map(|row| row.access_token)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| GitHostTokenError::NoCredential {
                actor: actor.clone(),
                provider,
                host: None,
            })
    }

    /// An App-installation row is reported usable on the strength of the App
    /// handle alone — see [`GitHostTokenIssuer::github_is_usable`], which this
    /// serves.
    async fn github_credential_is_usable(&self, actor: &Actor) -> Result<(), GitHostTokenError> {
        let Some(row) = self
            .provider_tokens
            .find_github_row(actor.user_id())
            .await
            .map_err(|e| StoreError::with_source(format!("read {actor}'s GitHub row"), e))?
        else {
            return Err(GitHostTokenError::NoCredential {
                actor: actor.clone(),
                provider: RepositoryProvider::Github,
                host: None,
            });
        };

        let checked = match row.auth_method.as_deref() {
            _ if self.blocks_pat(&row) => self.stashed_app_token(actor).await.map(|_| ()),
            Some(AUTH_METHOD_APP_INSTALLATION) => self.installation_app(&row).await.map(|_| ()),
            Some(AUTH_METHOD_SECRET_REF) => self
                .read_secret_ref_token(actor, GITHUB_TOKEN)
                .await
                .map(|_| ()),
            Some(AUTH_METHOD_APP_USER) => self.refreshed_oauth_token(&row, actor).await.map(|_| ()),
            _ => Ok(()),
        };

        checked.map_err(|source| GitHostTokenError::Broken {
            actor: actor.clone(),
            provider: RepositoryProvider::Github,
            source: Box::new(source),
        })
    }

    /// GitHub dispatches on the row's `auth_method`.
    async fn github_token(
        &self,
        actor: &Actor,
        owner: &str,
        repository: Option<&str>,
    ) -> Result<String, GitHostTokenError> {
        let Some(row) = self
            .provider_tokens
            .find_github_row(actor.user_id())
            .await
            .map_err(|e| StoreError::with_source(format!("read {actor}'s GitHub row"), e))?
        else {
            return Err(GitHostTokenError::NoCredential {
                actor: actor.clone(),
                provider: RepositoryProvider::Github,
                host: None,
            });
        };

        let resolved = match (row.auth_method.as_deref(), actor.person()) {
            _ if self.blocks_pat(&row) => self.stashed_app_token(actor).await,
            (Some(AUTH_METHOD_APP_INSTALLATION), Some(person)) => match repository {
                Some(repository) => {
                    return self
                        .mint_token_for_person(actor, &row, owner, repository, person)
                        .await;
                }
                // A person's role exists only per repository.
                None => Err(AuthError::Forbidden),
            },
            (Some(AUTH_METHOD_APP_INSTALLATION), None) => {
                self.mint_installation_token(&row, owner).await
            }
            (Some(AUTH_METHOD_SECRET_REF), _) => {
                self.read_secret_ref_token(actor, GITHUB_TOKEN).await
            }
            (Some(AUTH_METHOD_APP_USER), _) => self.refreshed_oauth_token(&row, actor).await,
            _ => Ok(row.access_token.clone()),
        };

        resolved.map_err(|source| {
            classify(
                actor,
                GitHostAccount::Github {
                    owner: owner.to_string(),
                    repository: repository.map(str::to_string),
                },
                source,
            )
        })
    }

    /// An installation token for `owner/repository` alone, capped by
    /// `person`'s role there.
    ///
    /// A refusal for `person`'s own missing connection or role names them, so
    /// it points at the access to grant; a failure minting names `actor`. A
    /// `person` whose row is an App installation gets the uncapped token.
    async fn mint_token_for_person(
        &self,
        actor: &Actor,
        row: &ProviderToken,
        owner: &str,
        repository: &str,
        person: &str,
    ) -> Result<String, GitHostTokenError> {
        let account = || GitHostAccount::Github {
            owner: owner.to_string(),
            repository: Some(repository.to_string()),
        };
        let as_actor = |source| classify(actor, account(), source);
        let person_actor = Actor::of(person);
        let person_row = self
            .provider_tokens
            .find_github_row(person)
            .await
            .map_err(|e| StoreError::with_source(format!("read {person}'s GitHub row"), e))?
            .ok_or_else(|| GitHostTokenError::NoCredential {
                actor: person_actor.clone(),
                provider: RepositoryProvider::Github,
                host: None,
            })?;
        if person_row.auth_method.as_deref() == Some(AUTH_METHOD_APP_INSTALLATION) {
            return self
                .mint_installation_token(row, owner)
                .await
                .map_err(as_actor);
        }
        let login = github_login(&person_row).ok_or_else(|| GitHostTokenError::Broken {
            actor: person_actor.clone(),
            provider: RepositoryProvider::Github,
            source: "the GitHub connection records no login to check repository access for".into(),
        })?;
        let app = self.installation_app(row).await.map_err(as_actor)?;
        role_capped_token(&app, owner, repository, &login, RepositoryRole::Read)
            .await
            .map_err(as_actor)?
            .ok_or_else(|| GitHostTokenError::Forbidden {
                actor: person_actor.clone(),
                account: Box::new(account()),
                source: format!("{login} has no role on {owner}/{repository} the App can act with")
                    .into(),
            })
    }

    /// The GitHub login `user_id` signed in as themselves; `None` for a bot
    /// connected through an App installation.
    async fn own_github_login(&self, user_id: &str) -> Result<Option<String>, StoreError> {
        let row = self
            .provider_tokens
            .find_github_row(user_id)
            .await
            .map_err(|e| StoreError::with_source(format!("read {user_id}'s GitHub row"), e))?;
        Ok(match row {
            Some(row) if row.auth_method.as_deref() == Some(AUTH_METHOD_APP_INSTALLATION) => None,
            // The App sign-in the PAT stands in front of names the person.
            Some(row) if self.blocks_pat(&row) => metadata_login(&row, "oauth_login"),
            Some(row) => github_login(&row),
            None => None,
        })
    }

    /// Serves [`GitHostTokenIssuer::github_app_write`], whose error rule this
    /// applies to [`Self::app_token_for_sign_in`].
    async fn app_write_token(
        &self,
        actor: &Actor,
        owner: &str,
        repository: &str,
    ) -> Result<Option<AppWriteToken>, GitHostTokenError> {
        match self.app_token_for_sign_in(actor, owner, repository).await {
            Err(error @ GitHostTokenError::Forbidden { .. }) => Err(error),
            Err(error) => {
                warn!(%error, owner, repository, "No App token for a GitHub write; writing as the actor");
                Ok(None)
            }
            minted => minted,
        }
    }

    /// The App token for an actor whose own row is a login-App user token.
    async fn app_token_for_sign_in(
        &self,
        actor: &Actor,
        owner: &str,
        repository: &str,
    ) -> Result<Option<AppWriteToken>, GitHostTokenError> {
        if actor.person().is_some() {
            return Ok(None);
        }
        let row = self
            .provider_tokens
            .find_github_row(actor.user_id())
            .await
            .map_err(|e| StoreError::with_source(format!("read {actor}'s GitHub row"), e))?;
        let (row_is_app_sign_in, login) = match row {
            Some(row) if row.auth_method.as_deref() == Some(AUTH_METHOD_APP_USER) => {
                (true, github_login(&row))
            }
            // The App sign-in the PAT stands in front of names the driver.
            Some(row) if self.blocks_pat(&row) => (true, metadata_login(&row, "oauth_login")),
            _ => (false, None),
        };
        let Some(login) = login else {
            return if row_is_app_sign_in {
                Err(broken(
                    actor,
                    "the GitHub connection records no login to post for; reconnect GitHub",
                ))
            } else {
                Ok(None)
            };
        };
        let Some(app) = self.github_apps.default_service() else {
            return Err(broken(
                actor,
                "no GitHub login App is configured to post as",
            ));
        };
        let account = GitHostAccount::Github {
            owner: owner.to_string(),
            repository: Some(repository.to_string()),
        };
        let token = role_capped_token(&app, owner, repository, &login, RepositoryRole::Write)
            .await
            .map_err(|source| classify(actor, account.clone(), source))?
            .ok_or_else(|| GitHostTokenError::Forbidden {
                actor: actor.clone(),
                account: Box::new(account),
                source: format!(
                    "{login} needs write access to {owner}/{repository} for the GitHub App to \
                     post for them; ask a repository admin to grant it"
                )
                .into(),
            })?;
        Ok(Some(AppWriteToken { token }))
    }

    /// An App-installation row mints an installation-wide token: it can reach
    /// every repository the installation covers, not just the one being worked
    /// on. The installation is resolved *from* the owner, so it cannot mismatch
    /// the owner and no stored `installation_id` is needed.
    async fn mint_installation_token(
        &self,
        row: &ProviderToken,
        owner: &str,
    ) -> Result<String, AuthError> {
        let app = self.installation_app(row).await?;
        let installation = app.resolve_installation_id(owner).await?;
        app.get_installation_token(installation).await
    }

    /// The App an installation row mints through.
    async fn installation_app(
        &self,
        row: &ProviderToken,
    ) -> Result<Arc<GitHubAppService>, AuthError> {
        let app_key = row.provider_user_id.as_deref().ok_or_else(|| {
            AuthError::Config(format!(
                "Bot user '{}' provider_tokens row is missing provider_user_id (App key)",
                row.user_id
            ))
        })?;
        self.github_apps.get(app_key).await?.ok_or_else(|| {
            AuthError::Config(format!(
                "Bot user '{}' references unknown GitHub App key '{app_key}'",
                row.user_id
            ))
        })
    }

    /// Whether `row` is a GitHub personal access token this deployment refuses.
    fn blocks_pat(&self, row: &ProviderToken) -> bool {
        self.github_pats_disabled
            && (row.auth_method.as_deref() == Some(AUTH_METHOD_SECRET_REF)
                || row.token_type.as_deref() == Some("pat"))
    }

    /// The App credential stashed behind `actor`'s refused PAT, renewed and
    /// re-stashed near expiry. The PAT row itself is left as it is, so the
    /// person can still remove the override in Settings.
    async fn stashed_app_token(&self, actor: &Actor) -> Result<String, AuthError> {
        let user_id = actor.user_id();
        let stashed = read_oauth_stash(&self.secrets, user_id, GITHUB_OAUTH_STASH_KEY)
            .await
            .map_err(|e| AuthError::Config(format!("Failed to read the GitHub App stash: {e}")))?
            .ok_or_else(|| {
                AuthError::Config(
                    "GitHub personal access tokens are disabled on this deployment; \
                     remove the token in Settings and connect GitHub through the App"
                        .to_string(),
                )
            })?;
        let (Some(refresh_token), Some(app), false) = (
            stashed.refresh_token.as_deref(),
            self.github_app_service.as_deref(),
            stash_is_fresh(&stashed),
        ) else {
            return Ok(stashed.access_token);
        };
        let renewed = match app.refresh_user_token(refresh_token).await {
            Ok(renewed) => renewed,
            // A concurrent caller may have spent the single-use refresh token
            // and stashed the new pair; `stashed` predates that.
            Err(e) => {
                return match read_oauth_stash(&self.secrets, user_id, GITHUB_OAUTH_STASH_KEY).await
                {
                    Ok(Some(current)) if stash_is_fresh(&current) => Ok(current.access_token),
                    _ => Err(e),
                };
            }
        };
        let now = chrono::Utc::now();
        let credential = StashedOAuthCredential {
            access_token: renewed.access_token.clone(),
            refresh_token: renewed.refresh_token,
            expires_at: renewed
                .expires_in
                .map(|secs| now + chrono::Duration::seconds(secs as i64)),
        };
        // The refresh token is single-use: an unsaved pair strands the stash.
        stash_oauth_credential(
            &self.secrets,
            user_id,
            GITHUB_OAUTH_STASH_KEY,
            "github",
            &credential,
        )
        .await
        .map_err(|e| {
            AuthError::Config(format!("Failed to save the renewed GitHub App token: {e}"))
        })?;
        Ok(renewed.access_token)
    }

    /// A GitHub App user row refreshes near expiry; a failed refresh falls back
    /// as [`fallback_after_failed_refresh`] decides.
    async fn refreshed_oauth_token(
        &self,
        row: &ProviderToken,
        actor: &Actor,
    ) -> Result<String, AuthError> {
        let Some(app) = self.github_app_service.as_deref() else {
            return Ok(row.access_token.clone());
        };
        match ensure_fresh_token(
            self.provider_tokens.as_ref(),
            self.provider_token_auth.as_ref(),
            app,
            actor.user_id(),
        )
        .await
        {
            Ok(token) => Ok(token),
            Err(e) => {
                // A concurrent caller may have spent the single-use refresh
                // token and stored a new pair; `row` predates that.
                let current = self
                    .provider_tokens
                    .find_github_row(actor.user_id())
                    .await
                    .ok()
                    .flatten();
                let row = current.as_ref().unwrap_or(row);
                fallback_after_failed_refresh(row, actor, e, chrono::Utc::now())
            }
        }
    }

    /// Read the encrypted PAT the `secret_ref` row for `key` points at.
    ///
    /// `key` names the secret: one identity holds one `secret_ref` row per
    /// host (`GITHUB_TOKEN`, or [`forgejo_token_key`] per Forgejo instance).
    /// Every failure is an error, not an absent credential: an unresolvable
    /// reference means the identity *is* connected but unreachable.
    async fn read_secret_ref_token(&self, actor: &Actor, key: &str) -> Result<String, AuthError> {
        self.secrets
            .resolve_scope_secret(
                Scope::User,
                Some(actor.user_id()),
                key,
                Some(SecretKind::ProviderToken),
            )
            .await
            .map_err(|e| AuthError::Config(format!("Failed to read the {key} secret: {e}")))?
            .ok_or_else(|| {
                AuthError::Config(format!(
                    "The credential is a secret_ref but no matching {key} secret exists"
                ))
            })
    }

    /// Resolve the credential stored for this Forgejo instance.
    async fn forgejo_token(&self, actor: &Actor, host: &str) -> Result<String, GitHostTokenError> {
        let row = self
            .provider_tokens
            .find_provider_token_full(actor.user_id(), &forgejo_provider_key(host))
            .await
            .map_err(|e| {
                StoreError::with_source(format!("read {actor}'s Forgejo row for {host}"), e)
            })?
            .ok_or_else(|| GitHostTokenError::NoCredential {
                actor: actor.clone(),
                provider: RepositoryProvider::Forgejo,
                host: Some(host.to_string()),
            })?;

        let resolved = match row.auth_method.as_deref() {
            Some(AUTH_METHOD_SECRET_REF) => {
                self.read_secret_ref_token(actor, &forgejo_token_key(host))
                    .await
            }
            Some(AUTH_METHOD_FORGEJO_OAUTH) => {
                Ok(self.refreshed_forgejo_token(&row, actor, host).await)
            }
            _ => Ok(row.access_token.clone()),
        };

        resolved.map_err(|source| {
            classify(
                actor,
                GitHostAccount::Forgejo {
                    host: host.to_string(),
                },
                source,
            )
        })
    }

    /// A Forgejo OAuth row refreshes near expiry. Every failure falls back to
    /// the stored access token, which may still be valid; the host's own
    /// rejection is what reports it when it is not.
    ///
    /// Unserialized, on the terms
    /// [`crate::auth::token_refresh::ensure_fresh_token`] states.
    async fn refreshed_forgejo_token(
        &self,
        row: &ProviderToken,
        actor: &Actor,
        host: &str,
    ) -> String {
        let stored = row.access_token.clone();
        let Some(refresh) = self.forgejo.as_ref() else {
            return stored;
        };
        // A row with no expiry is undated rather than perpetual, so it is
        // renewed once and dated by the result.
        if row
            .expires_at
            .is_some_and(|at| at > chrono::Utc::now() + chrono::Duration::minutes(5))
        {
            return stored;
        }
        let Some(refresh_token) = row.refresh_token.as_deref() else {
            return stored;
        };

        match self
            .renew_forgejo_credential(refresh, actor, host, refresh_token)
            .await
        {
            Ok(token) => token,
            Err(error) => {
                warn!(
                    %error,
                    actor = %actor,
                    host,
                    "Forgejo token refresh failed, falling back to the stored token"
                );
                stored
            }
        }
    }

    async fn renew_forgejo_credential(
        &self,
        refresh: &ForgejoRefresh,
        actor: &Actor,
        host: &str,
        refresh_token: &str,
    ) -> Result<String, AuthError> {
        let app = refresh
            .instances
            .app(host)
            .await?
            .ok_or_else(|| AuthError::NotFound(format!("No OAuth application for {host}")))?;
        let renewed = refresh
            .oauth
            .refresh(&app, refresh_token)
            .await
            .map_err(|e| AuthError::OAuth(e.to_string()))?;

        // Forgejo invalidates the presented refresh token, so the answer is
        // stored whole before it is used: a renewal that is returned but not
        // persisted leaves the row holding a refresh token nothing can spend.
        self.provider_tokens
            .upsert_token_with_auth_method(
                actor.user_id(),
                &forgejo_provider_key(host),
                AUTH_METHOD_FORGEJO_OAUTH,
                &renewed.access_token,
                renewed.refresh_token.as_deref(),
                Some("bearer"),
                Some(&serde_json::json!({ "host": host })),
                renewed.expires_at,
            )
            .await
            .map_err(AuthError::Database)?;
        Ok(renewed.access_token)
    }
}

/// The GitHub login a row was connected as: the column the App user flow
/// writes, else the `login` a PAT's metadata records.
fn github_login(row: &ProviderToken) -> Option<String> {
    row.provider_login
        .clone()
        .filter(|login| !login.is_empty())
        .or_else(|| metadata_login(row, "login"))
}

/// A non-empty login `row`'s metadata records under `key`.
fn metadata_login(row: &ProviderToken, key: &str) -> Option<String> {
    row.metadata
        .as_ref()?
        .get(key)?
        .as_str()
        .filter(|login| !login.is_empty())
        .map(str::to_string)
}

/// An installation token of `app` for `owner/repository` alone, capped by
/// `login`'s role there; `None` when that role is below `minimum` or grants
/// the App nothing.
async fn role_capped_token(
    app: &GitHubAppService,
    owner: &str,
    repository: &str,
    login: &str,
    minimum: RepositoryRole,
) -> Result<Option<String>, AuthError> {
    let installation = app.resolve_installation_id(owner).await?;
    let Some(role) = app
        .repository_role(installation, owner, repository, login)
        .await?
        .filter(|role| *role == RepositoryRole::Write || minimum == RepositoryRole::Read)
    else {
        return Ok(None);
    };
    let permissions = role.cap(&app.installation_permissions(installation).await?);
    if permissions.is_empty() {
        return Ok(None);
    }
    app.get_repository_installation_token(installation, owner, repository, &permissions)
        .await
        .map(Some)
}

/// Whether `stash` stays in date for at least the refresh margin.
fn stash_is_fresh(stash: &StashedOAuthCredential) -> bool {
    stash
        .expires_at
        .is_some_and(|at| at > chrono::Utc::now() + chrono::Duration::minutes(5))
}

/// `actor`'s GitHub credential reported unusable for `reason`.
fn broken(actor: &Actor, reason: &str) -> GitHostTokenError {
    GitHostTokenError::Broken {
        actor: actor.clone(),
        provider: RepositoryProvider::Github,
        source: reason.into(),
    }
}

/// The stored token after its refresh failed, while it is still in date.
///
/// Once it has expired GitHub answers it with 401, so handing it on only moves
/// the failure to every consumer as an unexplained `Bad credentials`; the
/// refresh error is returned instead, which reports the identity as broken.
fn fallback_after_failed_refresh(
    row: &ProviderToken,
    actor: &Actor,
    error: AuthError,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, AuthError> {
    if let Some(expired_at) = row.expires_at.filter(|at| *at <= now) {
        warn!(
            %error,
            actor = %actor,
            %expired_at,
            "GitHub App token refresh failed and the stored token has expired"
        );
        let remedy = if matches!(error, AuthError::RefreshRejected { .. }) {
            "the GitHub App authorization must be reconnected: "
        } else {
            ""
        };
        return Err(AuthError::OAuth(format!(
            "{remedy}the stored token expired at {expired_at} and refreshing it failed: {error}"
        )));
    }
    warn!(
        %error,
        actor = %actor,
        "GitHub App token refresh failed, falling back to the stored token"
    );
    Ok(row.access_token.clone())
}

/// Separate "the credential cannot reach this repository" from "the credential
/// is broken". The remedies differ, and "connect GitHub" is wrong advice for
/// someone already connected.
fn classify(actor: &Actor, account: GitHostAccount, source: AuthError) -> GitHostTokenError {
    let out_of_reach = match &source {
        AuthError::Forbidden | AuthError::NotInstalled { .. } => true,
        AuthError::Config(message) => message.contains("does not have access to repository"),
        _ => false,
    };
    if out_of_reach {
        return GitHostTokenError::Forbidden {
            actor: actor.clone(),
            account: Box::new(account),
            source: Box::new(source),
        };
    }
    GitHostTokenError::Broken {
        actor: actor.clone(),
        provider: account.provider(),
        source: Box::new(source),
    }
}

/// A seam over a caller-supplied token store, with the four App/secret ports
/// stubbed out.
///
/// Every consumer of this seam is composed with all five ports, so a test that
/// only cares about stored rows would otherwise repeat the other four.
// `test` stays first so the census keeps scoring this fake as test-context; the
// feature exposes it to other crates' tests (see Cargo.toml `test-support`).
#[cfg(any(test, feature = "test-support"))]
pub fn seam_over(
    provider_tokens: Arc<crate::auth::test_support::StubProviderTokens>,
) -> Arc<dyn GitHostTokenIssuer> {
    Arc::new(GitHostTokens::new(
        provider_tokens,
        Arc::new(crate::auth::provider_token_auth_dao::MockProviderTokenAuthDao::new()),
        None,
        Arc::new(GitHubAppRegistry::default()),
        Arc::new(crate::secret::test_support::service_with(vec![])),
    ))
}

/// Freshness is handled here: OAuth rows refresh through [`ensure_fresh_token`]
/// and installation rows mint, so a returned token is usable now and must not
/// be cached.
#[async_trait]
impl GitHostTokenIssuer for GitHostTokens {
    async fn token(
        &self,
        actor: &Actor,
        account: &GitHostAccount,
    ) -> Result<String, GitHostTokenError> {
        let provider = account.provider();
        // No GitHub pin: a Workspace cannot name the account GitHub work acts
        // as, which would bypass the App authoring it.
        if provider != RepositoryProvider::Github {
            if let Some(pinned) = self.pinned_token(actor, provider).await? {
                return Ok(pinned);
            }
        }
        match account {
            GitHostAccount::Github { owner, repository } => {
                self.github_token(actor, owner, repository.as_deref()).await
            }
            GitHostAccount::Gitlab => {
                self.stored_host_token(actor, RepositoryProvider::Gitlab)
                    .await
            }
            GitHostAccount::Forgejo { host } => self.forgejo_token(actor, host).await,
        }
    }

    async fn github_is_usable(&self, actor: &Actor) -> Result<(), GitHostTokenError> {
        self.github_credential_is_usable(actor).await
    }

    async fn github_app_write(
        &self,
        actor: &Actor,
        owner: &str,
        repository: &str,
    ) -> Result<Option<AppWriteToken>, GitHostTokenError> {
        self.app_write_token(actor, owner, repository).await
    }

    async fn github_app_commit_author(
        &self,
        actor: &Actor,
        owner: &str,
        repository: &str,
    ) -> Result<Option<GitIdentity>, GitHostTokenError> {
        if self
            .app_write_token(actor, owner, repository)
            .await?
            .is_none()
        {
            return Ok(None);
        }
        // `app_write_token` answers only with the login App configured.
        let Some(app) = self.github_apps.default_service() else {
            return Ok(None);
        };
        let bot = app.get_bot_identity().await.map_err(|e| {
            broken(
                actor,
                &format!("the GitHub App's bot identity is unreadable: {e}"),
            )
        })?;
        Ok(Some(GitIdentity {
            is_bot: true,
            name: Some(bot.login.clone()),
            login: bot.login,
            email: Some(bot.email),
        }))
    }

    async fn github_noreply_email(&self, user_id: &str) -> Result<Option<String>, StoreError> {
        let row = self
            .provider_tokens
            .find_github_row(user_id)
            .await
            .map_err(|e| StoreError::with_source(format!("read {user_id}'s GitHub row"), e))?;
        Ok(row.as_ref().and_then(noreply_email))
    }

    async fn credited_github_login(
        &self,
        attribution: &Attribution,
    ) -> Result<Option<(String, String)>, StoreError> {
        for person in attribution.credited() {
            if let Some(login) = self.own_github_login(person).await? {
                return Ok(Some((person.to_string(), login)));
            }
        }
        Ok(None)
    }
}

/// The noreply address GitHub links to `row`'s account: its numeric id and
/// login, as GitHub's own `{id}+{login}@users.noreply.github.com`.
fn noreply_email(row: &ProviderToken) -> Option<String> {
    let id = row
        .provider_user_id
        .as_deref()
        .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))?;
    let login = github_login(row)
        .filter(|login| login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))?;
    Some(format!("{id}+{login}@users.noreply.github.com"))
}

#[cfg(test)]
mod person_tests;

#[cfg(test)]
mod tests {
    use super::*;

    use crate::auth::provider_token_auth_dao::MockProviderTokenAuthDao;
    use crate::auth::provider_tokens::SECRET_REF_TOKEN_PLACEHOLDER;
    use crate::auth::test_support::{StubProviderTokens, github_row};
    use crate::secret::test_support::{secret, service_with};

    fn github_account() -> GitHostAccount {
        GitHostAccount::Github {
            owner: "acme".into(),
            repository: Some("repo".into()),
        }
    }

    fn tokens(provider_tokens: StubProviderTokens, secrets: SecretService) -> GitHostTokens {
        GitHostTokens::new(
            Arc::new(provider_tokens),
            Arc::new(MockProviderTokenAuthDao::new()),
            None,
            Arc::new(GitHubAppRegistry::default()),
            Arc::new(secrets),
        )
    }

    fn app_user_row_expiring_at(expires_at: chrono::DateTime<chrono::Utc>) -> ProviderToken {
        ProviderToken {
            expires_at: Some(expires_at),
            ..github_row(AUTH_METHOD_APP_USER, "ghu_stored")
        }
    }

    #[test]
    fn a_failed_refresh_falls_back_to_a_stored_token_still_in_date() {
        let now = chrono::Utc::now();
        let row = app_user_row_expiring_at(now + chrono::Duration::minutes(2));

        let token = fallback_after_failed_refresh(
            &row,
            &Actor::of("u1"),
            AuthError::OAuth("request timed out".into()),
            now,
        )
        .expect("an in-date token is still worth presenting");

        assert_eq!(token, "ghu_stored");
    }

    #[test]
    fn a_failed_refresh_of_an_expired_token_reports_the_refresh_error() {
        let now = chrono::Utc::now();
        let row = app_user_row_expiring_at(now - chrono::Duration::hours(1));

        let message = fallback_after_failed_refresh(
            &row,
            &Actor::of("u1"),
            AuthError::RefreshRejected {
                error: "bad_refresh_token".into(),
                description: String::new(),
            },
            now,
        )
        .expect_err("an expired token would only answer 401")
        .to_string();

        assert!(message.contains("reconnected"), "{message}");
        assert!(message.contains("bad_refresh_token"), "{message}");
    }

    #[test]
    fn a_transient_refresh_failure_of_an_expired_token_does_not_ask_for_a_reconnect() {
        let now = chrono::Utc::now();
        let row = app_user_row_expiring_at(now - chrono::Duration::hours(1));

        let message = fallback_after_failed_refresh(
            &row,
            &Actor::of("u1"),
            AuthError::OAuth("GitHub App token refresh returned 502 Bad Gateway".into()),
            now,
        )
        .expect_err("an expired token would only answer 401")
        .to_string();

        assert!(!message.contains("reconnected"), "{message}");
        assert!(message.contains("502"), "{message}");
    }

    async fn resolve(
        row: Option<ProviderToken>,
        secrets: SecretService,
    ) -> Result<String, GitHostTokenError> {
        tokens(StubProviderTokens::with_user_row(row), secrets)
            .token(&Actor::of("u1"), &github_account())
            .await
    }

    /// A PAT row standing in front of `stash`, the App credential it displaced.
    fn pat_over_stash(
        stash: Option<&StashedOAuthCredential>,
    ) -> (StubProviderTokens, SecretService) {
        let mut records = vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            "GITHUB_TOKEN",
            "ghp_real",
        )];
        if let Some(stash) = stash {
            records.push(secret(
                "user",
                Some("u1"),
                "internal",
                GITHUB_OAUTH_STASH_KEY,
                &serde_json::to_string(stash).unwrap(),
            ));
        }
        let row = ProviderToken {
            token_type: Some("pat".to_string()),
            ..github_row(AUTH_METHOD_SECRET_REF, SECRET_REF_TOKEN_PLACEHOLDER)
        };
        (
            StubProviderTokens::with_user_row(Some(row)),
            service_with(records),
        )
    }

    #[tokio::test]
    async fn a_refused_pat_presents_the_app_token_stashed_behind_it() {
        let stash = StashedOAuthCredential {
            access_token: "ghu_stashed".to_string(),
            refresh_token: Some("ghr_stashed".to_string()),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        };
        let (rows, secrets) = pat_over_stash(Some(&stash));
        let seam = tokens(rows, secrets).with_github_pats_disabled(true);

        let token = seam
            .token(&Actor::of("u1"), &github_account())
            .await
            .expect("the stash stands in for the PAT");
        assert_eq!(token, "ghu_stashed");
        seam.github_is_usable(&Actor::of("u1"))
            .await
            .expect("the stash is a usable credential");
    }

    #[tokio::test]
    async fn a_refused_pat_with_nothing_stashed_says_pats_are_disabled() {
        let (rows, secrets) = pat_over_stash(None);
        let seam = tokens(rows, secrets).with_github_pats_disabled(true);

        let error = seam
            .token(&Actor::of("u1"), &github_account())
            .await
            .expect_err("a refused PAT is never presented");
        assert!(
            error
                .to_string()
                .contains("personal access tokens are disabled"),
            "got {error}"
        );
        assert!(seam.github_is_usable(&Actor::of("u1")).await.is_err());
    }

    #[tokio::test]
    async fn a_pat_is_presented_while_pats_are_allowed() {
        let (rows, secrets) = pat_over_stash(None);
        let token = tokens(rows, secrets)
            .token(&Actor::of("u1"), &github_account())
            .await
            .expect("resolves");
        assert_eq!(token, "ghp_real");
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
            secrets,
        )
        .await
        .expect("the stored PAT resolves");
        assert_eq!(
            token, "ghp_real",
            "the sentinel in access_token must never be returned"
        );
    }

    #[tokio::test]
    async fn dangling_secret_ref_is_broken_not_disconnected() {
        // Reference row present, secret gone. Reporting this as NoCredential
        // would read as "GitHub not connected" and silently drop auth.
        let error = resolve(
            Some(github_row(
                AUTH_METHOD_SECRET_REF,
                SECRET_REF_TOKEN_PLACEHOLDER,
            )),
            service_with(vec![]),
        )
        .await
        .expect_err("a dangling reference must be an error");
        assert!(
            matches!(error, GitHostTokenError::Broken { .. }),
            "got {error:?}"
        );
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
        let error = resolve(
            Some(github_row(
                AUTH_METHOD_SECRET_REF,
                SECRET_REF_TOKEN_PLACEHOLDER,
            )),
            secrets,
        )
        .await
        .expect_err("an env-kind secret must not satisfy a secret_ref row");
        assert!(
            matches!(error, GitHostTokenError::Broken { .. }),
            "got {error:?}"
        );
    }

    #[tokio::test]
    async fn a_plain_row_returns_the_column_verbatim() {
        let token = resolve(
            Some(github_row("manual", "ghp_legacy")),
            service_with(vec![]),
        )
        .await
        .expect("a plain row resolves");
        assert_eq!(token, "ghp_legacy");
    }

    #[tokio::test]
    async fn no_row_means_no_credential() {
        let error = resolve(None, service_with(vec![]))
            .await
            .expect_err("no row is not a token");
        assert!(
            matches!(error, GitHostTokenError::NoCredential { .. }),
            "got {error:?}"
        );
    }

    #[tokio::test]
    async fn another_actors_credential_is_never_reached() {
        // This is the deleted scope walk, asserted as absent: a global row and
        // a row belonging to someone else both stay invisible.
        let stub = StubProviderTokens::with_user_row_for(
            "someone-else",
            github_row("manual", "ghp_theirs"),
        )
        .with_global_row(github_row("manual", "ghp_org"));
        let error = tokens(stub, service_with(vec![]))
            .token(&Actor::of("u1"), &github_account())
            .await
            .expect_err("u1 holds no row of their own");
        assert!(
            matches!(error, GitHostTokenError::NoCredential { .. }),
            "got {error:?}"
        );
    }

    #[tokio::test]
    async fn an_installation_row_resolves_through_the_app_registry() {
        // The App key is unregistered, so resolution stops at the registry.
        // Reaching that point proves the branch went through the App lookup
        // before any mint — installation-wide or otherwise.
        let mut row = github_row(
            AUTH_METHOD_APP_INSTALLATION,
            crate::auth::provider_tokens::APP_INSTALLATION_TOKEN_PLACEHOLDER,
        );
        row.provider_user_id = Some("unregistered".to_string());
        let error = resolve(Some(row), service_with(vec![]))
            .await
            .expect_err("the test registry has no App");
        assert!(
            error.to_string().contains("unknown GitHub App key"),
            "got {error}"
        );
    }

    async fn usable(row: Option<ProviderToken>) -> Result<(), GitHostTokenError> {
        tokens(StubProviderTokens::with_user_row(row), service_with(vec![]))
            .github_is_usable(&Actor::of("u1"))
            .await
    }

    #[tokio::test]
    async fn an_app_backed_identity_is_never_reported_as_never_connected() {
        // What the T3 source-control panel reads. An installation row has no
        // token or login, so a naive read sees "nothing" and suggests
        // "connect an account" — while the identity already pushes as the
        // App. Must distinguish "no credential" from "a credential whose
        // App couldn't be resolved": only the former invites a connect.
        let mut row = github_row(
            AUTH_METHOD_APP_INSTALLATION,
            crate::auth::provider_tokens::APP_INSTALLATION_TOKEN_PLACEHOLDER,
        );
        row.provider_user_id = Some("unregistered".to_string());
        let error = usable(Some(row))
            .await
            .expect_err("the test registry has no App");
        assert!(
            matches!(error, GitHostTokenError::Broken { .. }),
            "an App-backed row is a broken credential, not an absent one: got {error:?}"
        );
    }

    #[tokio::test]
    async fn a_stored_row_is_a_usable_github_credential() {
        usable(Some(github_row("pat", "ghp_stored")))
            .await
            .expect("a stored token needs no App to be usable");
    }

    #[tokio::test]
    async fn no_row_is_the_one_state_that_invites_connecting_an_account() {
        let error = usable(None).await.expect_err("nothing is stored");
        assert!(
            matches!(error, GitHostTokenError::NoCredential { .. }),
            "got {error:?}"
        );
    }

    #[tokio::test]
    async fn a_provider_token_secret_alone_is_not_a_credential() {
        // The identity holds a validated PAT in the secret store but no
        // `provider_tokens` row. There is no credential: a secret is reachable
        // only through the reference row that names it.
        let secrets = service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            "GITHUB_TOKEN",
            "ghp_validated",
        )]);
        let error = resolve(None, secrets)
            .await
            .expect_err("a secret with no row is not a credential");
        assert!(
            matches!(error, GitHostTokenError::NoCredential { .. }),
            "got {error:?}"
        );
    }

    #[tokio::test]
    async fn an_env_secret_alone_is_not_a_credential() {
        let secrets = service_with(vec![secret(
            "user",
            Some("u1"),
            "env",
            "GITHUB_TOKEN",
            "ghp_unvalidated",
        )]);
        let error = resolve(None, secrets)
            .await
            .expect_err("an env secret with no row is not a credential");
        assert!(
            matches!(error, GitHostTokenError::NoCredential { .. }),
            "got {error:?}"
        );
    }

    #[tokio::test]
    async fn a_stored_row_outranks_a_secret_under_the_same_key() {
        let secrets = service_with(vec![secret(
            "user",
            Some("u1"),
            "provider_token",
            "GITHUB_TOKEN",
            "ghp_from_secret",
        )]);
        let token = resolve(Some(github_row("oauth", "gho_from_row")), secrets)
            .await
            .expect("the row resolves");
        assert_eq!(token, "gho_from_row");
    }

    #[tokio::test]
    async fn the_actors_own_row_wins_over_a_global_one() {
        // A run-as identity owns its credential: its own row is used even
        // though a global row exists. Without this the sandbox silently ran as
        // the global App.
        let stub = StubProviderTokens::with_user_row(Some(github_row("oauth", "gho_run_as_user")))
            .with_global_row(github_row("oauth", "gho_global_binding"));
        let token = tokens(stub, service_with(vec![]))
            .token(&Actor::of("u1"), &github_account())
            .await
            .expect("the actor's own row resolves");
        assert_eq!(token, "gho_run_as_user");
    }

    #[tokio::test]
    async fn gitlab_reads_the_row_for_that_provider_only() {
        let mut row = github_row("manual", "glpat_mine");
        row.provider = "gitlab".to_string();
        let repo = GitHostAccount::Gitlab;
        let token = tokens(
            StubProviderTokens::with_user_row(Some(row)),
            service_with(vec![]),
        )
        .token(&Actor::of("u1"), &repo)
        .await
        .expect("a GitLab row resolves through the same seam");
        assert_eq!(token, "glpat_mine");
    }

    #[tokio::test]
    async fn a_missing_gitlab_row_names_gitlab_and_not_github() {
        let repo = GitHostAccount::Gitlab;
        let error = tokens(StubProviderTokens::default(), service_with(vec![]))
            .token(&Actor::of("u1"), &repo)
            .await
            .expect_err("no GitLab row is not a token");
        assert!(error.to_string().contains("gitlab"), "got {error}");
    }

    #[tokio::test]
    async fn forgejo_selects_the_named_instance_without_a_repository() {
        let mut row = github_row("manual", "forgejo_token");
        row.provider = "forgejo:git.acme.test".into();
        let seam = tokens(
            StubProviderTokens::with_user_row(Some(row)),
            service_with(vec![]),
        );
        let account = GitHostAccount::Forgejo {
            host: "git.acme.test".into(),
        };
        assert_eq!(
            seam.token(&Actor::of("u1"), &account).await.unwrap(),
            "forgejo_token"
        );
        let other = GitHostAccount::Forgejo {
            host: "git.other.test".into(),
        };
        assert!(matches!(seam.token(&Actor::of("u1"), &other).await,
            Err(GitHostTokenError::NoCredential { host: Some(host), .. }) if host == "git.other.test"));
    }

    // --- the credential a workspace pins ------------------------------------

    /// A credential store answering one workspace, or failing for every read.
    struct PinnedCredentials {
        workspace_id: &'static str,
        provider: RepositoryProvider,
        value: String,
        fails: bool,
    }

    impl PinnedCredentials {
        fn holding(
            workspace_id: &'static str,
            provider: RepositoryProvider,
            value: &str,
        ) -> Arc<dyn GitCredentialOverrides> {
            Arc::new(Self {
                workspace_id,
                provider,
                value: value.to_string(),
                fails: false,
            })
        }

        fn unreadable() -> Arc<dyn GitCredentialOverrides> {
            Arc::new(Self {
                workspace_id: "ws_1",
                provider: RepositoryProvider::Gitlab,
                value: String::new(),
                fails: true,
            })
        }
    }

    #[async_trait::async_trait]
    impl GitCredentialOverrides for PinnedCredentials {
        async fn token(
            &self,
            workspace_id: &str,
            provider: RepositoryProvider,
        ) -> Result<Option<String>, moatless_types::store::StoreError> {
            if self.fails {
                return Err(moatless_types::store::StoreError::new("unreadable"));
            }
            Ok(
                (workspace_id == self.workspace_id && provider == self.provider)
                    .then(|| self.value.clone()),
            )
        }
    }

    #[tokio::test]
    async fn a_workspace_never_pins_github() {
        let seam = tokens(
            StubProviderTokens::with_user_row(Some(github_row("manual", "ghp_user"))),
            service_with(vec![]),
        )
        .with_credential_overrides(PinnedCredentials::holding(
            "ws_1",
            RepositoryProvider::Github,
            "ghp_workspace",
        ));

        let token = seam
            .token(
                &Actor::of("u1").on_workspace(Some("ws_1")),
                &github_account(),
            )
            .await
            .expect("the identity's own row resolves");
        assert_eq!(token, "ghp_user");
    }

    #[tokio::test]
    async fn a_workspace_pin_answers_for_an_identity_holding_no_row_at_all() {
        let seam =
            tokens(StubProviderTokens::default(), service_with(vec![])).with_credential_overrides(
                PinnedCredentials::holding("ws_1", RepositoryProvider::Gitlab, "glpat_workspace"),
            );

        assert_eq!(
            seam.token(
                &Actor::of("u1").on_workspace(Some("ws_1")),
                &GitHostAccount::Gitlab
            )
            .await
            .expect("resolves"),
            "glpat_workspace"
        );
    }

    #[tokio::test]
    async fn a_workspace_pinning_nothing_for_this_host_falls_through_to_the_row() {
        let seam = tokens(
            StubProviderTokens::with_user_row(Some(github_row("manual", "ghp_user"))),
            service_with(vec![]),
        )
        .with_credential_overrides(PinnedCredentials::holding(
            "ws_1",
            RepositoryProvider::Gitlab,
            "sk-workspace",
        ));

        let token = seam
            .token(
                &Actor::of("u1").on_workspace(Some("ws_1")),
                &github_account(),
            )
            .await
            .expect("a pin for another provider is not a GitHub pin");
        assert_eq!(token, "ghp_user");
    }

    #[tokio::test]
    async fn a_pin_that_cannot_be_read_is_broken_rather_than_the_users_own_token() {
        // Falling through here would make the call as a different account than
        // the workspace names — the failure this seam exists to remove.
        let seam = tokens(
            StubProviderTokens::with_user_row(Some(github_row("manual", "ghp_user"))),
            service_with(vec![]),
        )
        .with_credential_overrides(PinnedCredentials::unreadable());

        let error = seam
            .token(
                &Actor::of("u1").on_workspace(Some("ws_1")),
                &GitHostAccount::Gitlab,
            )
            .await
            .expect_err("an unreadable pin is not an absent pin");
        assert!(
            matches!(error, GitHostTokenError::Broken { .. }),
            "got {error}"
        );
    }

    #[tokio::test]
    async fn a_workspace_pin_governs_gitlab_the_same_way() {
        let repo = GitHostAccount::Gitlab;
        let seam =
            tokens(StubProviderTokens::default(), service_with(vec![])).with_credential_overrides(
                PinnedCredentials::holding("ws_1", RepositoryProvider::Gitlab, "glpat_workspace"),
            );

        assert_eq!(
            seam.token(&Actor::of("u1").on_workspace(Some("ws_1")), &repo)
                .await
                .expect("resolves"),
            "glpat_workspace"
        );
    }
}
