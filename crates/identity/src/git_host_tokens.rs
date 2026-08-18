//! Issuing a git-host token for one named identity.
//!
//! This is the issuing counterpart of [`crate::git_host_access`], which
//! *administers* a credential — connect, disconnect, validate, status. This
//! module answers one question instead: what token does this identity present
//! to this host for this repository, right now.

use std::sync::Arc;

use tracing::warn;

use moatless_api_schemas::enums::RepositoryProvider;
use moatless_core::secret_keys::GITHUB_TOKEN;

use crate::auth::actor::Actor;
use crate::auth::error::AuthError;
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::models::ProviderToken;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::{
    AUTH_METHOD_APP_INSTALLATION, AUTH_METHOD_SECRET_REF, ProviderTokenAuthDao,
};
use crate::auth::token_refresh::ensure_fresh_token;
use crate::git_host_repo::GitHostRepo;
use crate::secret::models::SecretKind;
use crate::secret::service::SecretService;
use moatless_backend_core::scope::Scope;

/// Why an identity has no usable token for a repository.
///
/// [`Self::NoCredential`] is the only "not connected" answer. A dangling
/// `secret_ref`, an unknown App key or an unusable stored key is
/// [`Self::Broken`] — collapsing the two would send a caller down an anonymous
/// path instead of surfacing breakage.
#[derive(Debug, thiserror::Error)]
pub enum GitHostTokenError {
    #[error("{actor} has not connected {provider}")]
    NoCredential {
        actor: Actor,
        provider: RepositoryProvider,
    },

    #[error("{actor}'s {} credential cannot reach {repo}", repo.provider())]
    Forbidden {
        actor: Actor,
        repo: GitHostRepo,
        #[source]
        source: AuthError,
    },

    #[error("{actor}'s {provider} credential is unusable: {source}")]
    Broken {
        actor: Actor,
        provider: RepositoryProvider,
        #[source]
        source: AuthError,
    },

    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// A git host whose credential is a single stored value, readable without
/// naming a repository.
///
/// GitHub is absent by construction: an installation token is minted through
/// the App, resolved from the repo's owner, so there is no stored GitHub token
/// and [`GitHostTokens::token`] is the only way to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlainGitHost {
    Gitlab,
    Gitness,
}

impl PlainGitHost {
    fn provider(self) -> RepositoryProvider {
        match self {
            Self::Gitlab => RepositoryProvider::Gitlab,
            Self::Gitness => RepositoryProvider::Gitness,
        }
    }
}

/// The one way to obtain a git-host token.
///
/// Constructed once at the composition root, so the five infrastructure ports
/// below are supplied in a single place rather than threaded through every
/// caller.
pub struct GitHostTokens {
    provider_tokens: Arc<dyn ProviderTokenDao>,
    provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
    github_app_service: Option<Arc<GitHubAppService>>,
    github_apps: Arc<GitHubAppRegistry>,
    secrets: Arc<SecretService>,
}

impl GitHostTokens {
    pub fn new(
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
        }
    }

    /// The token `actor` presents to `repo`'s host right now.
    ///
    /// Callers may rely on these facts:
    ///
    /// - No scope walk is performed. `actor` is the sole authority: another
    ///   identity's credential is never tried, at any scope.
    /// - The returned string is never `SECRET_REF_TOKEN_PLACEHOLDER`,
    ///   `APP_INSTALLATION_TOKEN_PLACEHOLDER`, or empty.
    /// - Freshness is handled here. OAuth rows refresh through
    ///   [`ensure_fresh_token`]; installation rows mint. Do not cache the
    ///   returned string.
    pub async fn token(
        &self,
        actor: &Actor,
        repo: &GitHostRepo,
    ) -> Result<String, GitHostTokenError> {
        match repo.provider() {
            RepositoryProvider::Github => self.github_token(actor, repo).await,
            RepositoryProvider::Gitlab => self.host_token(actor, PlainGitHost::Gitlab).await,
            RepositoryProvider::Gitness => self.host_token(actor, PlainGitHost::Gitness).await,
        }
    }

    /// The token `actor` holds for a host that needs no repository to answer.
    ///
    /// The same rules as [`Self::token`]: no scope walk, and an absent row is
    /// [`GitHostTokenError::NoCredential`].
    pub async fn host_token(
        &self,
        actor: &Actor,
        host: PlainGitHost,
    ) -> Result<String, GitHostTokenError> {
        let provider = host.provider();
        self.provider_tokens
            .find_provider_token(actor.user_id(), provider.as_str())
            .await?
            .map(|row| row.access_token)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| GitHostTokenError::NoCredential {
                actor: actor.clone(),
                provider,
            })
    }

    /// Whether `actor`'s GitHub credential is usable, with no repository in
    /// hand to name.
    ///
    /// For reporting connection state, not for calling GitHub — there is no
    /// token to return, because minting needs an owner to resolve the
    /// installation, which only a repository names. Such a row is reported
    /// usable on the strength of the App handle alone; every path that actually
    /// reaches GitHub names a repository and goes through [`Self::token`], where
    /// the mint either succeeds or says why not.
    pub async fn github_is_usable(&self, actor: &Actor) -> Result<(), GitHostTokenError> {
        let Some(row) = self
            .provider_tokens
            .find_github_row(actor.user_id())
            .await?
        else {
            return Err(GitHostTokenError::NoCredential {
                actor: actor.clone(),
                provider: RepositoryProvider::Github,
            });
        };

        let checked = match row.auth_method.as_deref() {
            Some(AUTH_METHOD_APP_INSTALLATION) => self.installation_app(&row).await.map(|_| ()),
            Some(AUTH_METHOD_SECRET_REF) => self.read_secret_ref_token(actor).await.map(|_| ()),
            Some("github_app") => {
                self.refreshed_oauth_token(&row, actor).await;
                Ok(())
            }
            _ => Ok(()),
        };

        checked.map_err(|source| GitHostTokenError::Broken {
            actor: actor.clone(),
            provider: RepositoryProvider::Github,
            source,
        })
    }

    /// GitHub dispatches on the row's `auth_method`.
    async fn github_token(
        &self,
        actor: &Actor,
        repo: &GitHostRepo,
    ) -> Result<String, GitHostTokenError> {
        let Some(row) = self
            .provider_tokens
            .find_github_row(actor.user_id())
            .await?
        else {
            return Err(GitHostTokenError::NoCredential {
                actor: actor.clone(),
                provider: RepositoryProvider::Github,
            });
        };

        let resolved = match row.auth_method.as_deref() {
            Some(AUTH_METHOD_APP_INSTALLATION) => self.mint_installation_token(&row, repo).await,
            Some(AUTH_METHOD_SECRET_REF) => self.read_secret_ref_token(actor).await,
            Some("github_app") => Ok(self.refreshed_oauth_token(&row, actor).await),
            _ => Ok(row.access_token.clone()),
        };

        resolved.map_err(|source| classify(actor, repo, source))
    }

    /// An App-installation row mints an installation-wide token: it can reach
    /// every repository the installation covers, not just the one being worked
    /// on. The installation is resolved *from* the owner, so it cannot mismatch
    /// the owner and no stored `installation_id` is needed.
    async fn mint_installation_token(
        &self,
        row: &ProviderToken,
        repo: &GitHostRepo,
    ) -> Result<String, AuthError> {
        let app = self.installation_app(row).await?;
        let installation = app.resolve_installation_id(repo.owner()).await?;
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

    /// A GitHub App user row refreshes near expiry. A refresh failure falls
    /// back to the stored access token, which may still be valid.
    async fn refreshed_oauth_token(&self, row: &ProviderToken, actor: &Actor) -> String {
        let Some(app) = self.github_app_service.as_deref() else {
            return row.access_token.clone();
        };
        match ensure_fresh_token(
            self.provider_tokens.as_ref(),
            self.provider_token_auth.as_ref(),
            app,
            actor.user_id(),
        )
        .await
        {
            Ok(token) => token,
            Err(e) => {
                warn!(
                    error = %e,
                    actor = %actor,
                    "GitHub App token refresh failed, falling back to the stored token"
                );
                row.access_token.clone()
            }
        }
    }

    /// Read the encrypted PAT a `secret_ref` row points at.
    ///
    /// Every failure mode is an error rather than an absent credential: a
    /// reference row with no resolvable secret means the identity *is*
    /// connected but the credential is unreachable.
    async fn read_secret_ref_token(&self, actor: &Actor) -> Result<String, AuthError> {
        self.secrets
            .resolve_scope_secret(
                Scope::User,
                Some(actor.user_id()),
                GITHUB_TOKEN,
                Some(SecretKind::ProviderToken),
            )
            .await
            .map_err(|e| AuthError::Config(format!("Failed to read the GitHub token secret: {e}")))?
            .ok_or_else(|| {
                AuthError::Config(
                    "The GitHub credential is a secret_ref but no matching GITHUB_TOKEN secret \
                     exists"
                        .to_string(),
                )
            })
    }
}

/// Separate "the credential cannot reach this repository" from "the credential
/// is broken". The remedies differ, and "connect GitHub" is wrong advice for
/// someone already connected.
fn classify(actor: &Actor, repo: &GitHostRepo, source: AuthError) -> GitHostTokenError {
    let out_of_reach = match &source {
        AuthError::Forbidden => true,
        AuthError::Config(message) => message.contains("does not have access to repository"),
        _ => false,
    };
    if out_of_reach {
        return GitHostTokenError::Forbidden {
            actor: actor.clone(),
            repo: repo.clone(),
            source,
        };
    }
    GitHostTokenError::Broken {
        actor: actor.clone(),
        provider: repo.provider(),
        source,
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
pub fn seam_over(provider_tokens: Arc<dyn ProviderTokenDao>) -> Arc<GitHostTokens> {
    Arc::new(GitHostTokens::new(
        provider_tokens,
        Arc::new(crate::auth::provider_tokens::MockProviderTokenAuthDao::new()),
        None,
        Arc::new(GitHubAppRegistry::default()),
        Arc::new(crate::secret::test_support::service_with(vec![])),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::provider_tokens::{MockProviderTokenAuthDao, SECRET_REF_TOKEN_PLACEHOLDER};
    use crate::auth::test_support::{StubProviderTokens, github_row};
    use crate::secret::test_support::{secret, service_with};

    fn github_repo() -> GitHostRepo {
        GitHostRepo::parse(RepositoryProvider::Github, "https://github.com/acme/repo")
            .expect("parses")
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

    async fn resolve(
        row: Option<ProviderToken>,
        secrets: SecretService,
    ) -> Result<String, GitHostTokenError> {
        tokens(StubProviderTokens::with_user_row(row), secrets)
            .token(&Actor::of("u1"), &github_repo())
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
            .token(&Actor::of("u1"), &github_repo())
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

    #[tokio::test]
    async fn gitlab_reads_the_row_for_that_provider_only() {
        let mut row = github_row("manual", "glpat_mine");
        row.provider = "gitlab".to_string();
        let repo = GitHostRepo::parse(RepositoryProvider::Gitlab, "https://gitlab.com/acme/repo")
            .expect("parses");
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
        let repo = GitHostRepo::parse(RepositoryProvider::Gitlab, "https://gitlab.com/acme/repo")
            .expect("parses");
        let error = tokens(StubProviderTokens::default(), service_with(vec![]))
            .token(&Actor::of("u1"), &repo)
            .await
            .expect_err("no GitLab row is not a token");
        assert!(error.to_string().contains("gitlab"), "got {error}");
    }
}
