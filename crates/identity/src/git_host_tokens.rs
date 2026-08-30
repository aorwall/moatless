//! Issuing a git-host token for one named identity.
//!
//! This is the issuing counterpart of [`crate::git_host_access`], which
//! *administers* a credential — connect, disconnect, validate, status. This
//! module answers one question instead: what token does this identity present
//! to this host for this repository, right now.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::warn;

use moatless_backend_core::identity::{
    GitHostTokenError, GitHostTokenIssuer, PlainGitHost,
};
use moatless_backend_core::store::StoreError;
use moatless_backend_core::workspace::WorkspaceCredentialReader;
use moatless_types::workspace::WorkspaceCredentialProvider;
use moatless_core::secret_keys::GITHUB_TOKEN;
use moatless_types::repository::RepositoryProvider;

use moatless_types::access_control::Actor;
use crate::auth::error::AuthError;
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::models::ProviderToken;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::{
    AUTH_METHOD_APP_INSTALLATION, AUTH_METHOD_SECRET_REF, ProviderTokenAuthDao,
};
use crate::auth::token_refresh::ensure_fresh_token;
use moatless_types::repository::GitHostRepo;
use moatless_types::secret::SecretKind;
use crate::secret::service::SecretService;
use moatless_types::access_control::Scope;

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
    workspace_credentials: Option<Arc<dyn WorkspaceCredentialReader>>,
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
            workspace_credentials: None,
        }
    }

    /// Wire the store that answers what a Workspace pins.
    #[must_use]
    pub fn with_workspace_credentials(
        mut self,
        workspace_credentials: Arc<dyn WorkspaceCredentialReader>,
    ) -> Self {
        self.workspace_credentials = Some(workspace_credentials);
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
            (self.workspace_credentials.as_ref(), actor.workspace_id())
        else {
            return Ok(None);
        };
        let env = store.resolve_env(workspace_id).await.map_err(|e| {
            GitHostTokenError::Broken {
                actor: actor.clone(),
                provider,
                source: Box::new(AuthError::Config(format!(
                    "Failed to read the credentials pinned to workspace '{workspace_id}': {e}"
                ))),
            }
        })?;
        let key = WorkspaceCredentialProvider::from(provider).env_key();
        Ok(env.get(key).filter(|token| !token.is_empty()).cloned())
    }

    /// The single stored value `actor` holds for a plain host.
    async fn stored_host_token(
        &self,
        actor: &Actor,
        host: PlainGitHost,
    ) -> Result<String, GitHostTokenError> {
        let provider = host.provider();
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
            })
    }

    /// Whether `actor`'s GitHub credential is usable, with no repository in
    /// hand to name.
    ///
    /// For reporting connection state, not for calling GitHub — there is no
    /// token to return, because minting needs an owner to resolve the
    /// installation, which only a repository names. Such a row is reported
    /// usable on the strength of the App handle alone; every path that actually
    /// reaches GitHub names a repository and goes through
    /// [`GitHostTokenIssuer::token`], where the mint either succeeds or says
    /// why not.
    ///
    /// A Workspace pin is usable on its own: it is the token every call on that
    /// Workspace presents, so the identity needs no row of its own behind it.
    pub async fn github_is_usable(&self, actor: &Actor) -> Result<(), GitHostTokenError> {
        if self
            .pinned_token(actor, RepositoryProvider::Github)
            .await?
            .is_some()
        {
            return Ok(());
        }

        let Some(row) = self
            .provider_tokens
            .find_github_row(actor.user_id())
            .await
            .map_err(|e| StoreError::with_source(format!("read {actor}'s GitHub row"), e))?
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
            source: Box::new(source),
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
            .await
            .map_err(|e| StoreError::with_source(format!("read {actor}'s GitHub row"), e))?
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
            repo: Box::new(repo.clone()),
            source: Box::new(source),
        };
    }
    GitHostTokenError::Broken {
        actor: actor.clone(),
        provider: repo.provider(),
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
pub fn seam_over(provider_tokens: Arc<dyn ProviderTokenDao>) -> Arc<dyn GitHostTokenIssuer> {
    Arc::new(GitHostTokens::new(
        provider_tokens,
        Arc::new(crate::auth::provider_tokens::MockProviderTokenAuthDao::new()),
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
    async fn token(&self, actor: &Actor, repo: &GitHostRepo) -> Result<String, GitHostTokenError> {
        let provider = repo.provider();
        if let Some(pinned) = self.pinned_token(actor, provider).await? {
            return Ok(pinned);
        }
        match provider {
            RepositoryProvider::Github => self.github_token(actor, repo).await,
            RepositoryProvider::Gitlab => self.stored_host_token(actor, PlainGitHost::Gitlab).await,
            RepositoryProvider::Gitness => {
                self.stored_host_token(actor, PlainGitHost::Gitness).await
            }
        }
    }

    async fn host_token(
        &self,
        actor: &Actor,
        host: PlainGitHost,
    ) -> Result<String, GitHostTokenError> {
        if let Some(pinned) = self.pinned_token(actor, host.provider()).await? {
            return Ok(pinned);
        }
        self.stored_host_token(actor, host).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
            .token(&Actor::of("u1"), &github_repo())
            .await
            .expect("the actor's own row resolves");
        assert_eq!(token, "gho_run_as_user");
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

    // --- the credential a workspace pins ------------------------------------

    /// A credential store answering one workspace, or failing for every read.
    struct PinnedCredentials {
        workspace_id: &'static str,
        env: HashMap<String, String>,
        fails: bool,
    }

    impl PinnedCredentials {
        fn holding(workspace_id: &'static str, key: &str, value: &str) -> Arc<dyn WorkspaceCredentialReader> {
            Arc::new(Self {
                workspace_id,
                env: HashMap::from([(key.to_string(), value.to_string())]),
                fails: false,
            })
        }

        fn unreadable() -> Arc<dyn WorkspaceCredentialReader> {
            Arc::new(Self {
                workspace_id: "ws_1",
                env: HashMap::new(),
                fails: true,
            })
        }
    }

    #[async_trait::async_trait]
    impl WorkspaceCredentialReader for PinnedCredentials {
        async fn resolve_env(
            &self,
            workspace_id: &str,
        ) -> Result<HashMap<String, String>, moatless_backend_core::store::StoreError> {
            if self.fails {
                return Err(moatless_backend_core::store::StoreError::new("unreadable"));
            }
            Ok(if workspace_id == self.workspace_id {
                self.env.clone()
            } else {
                HashMap::new()
            })
        }
    }

    #[tokio::test]
    async fn a_workspace_pin_outranks_the_identitys_own_github_row() {
        // The whole point: the workspace names the account, so a task on it
        // reaches repositories the user's own token cannot.
        let seam = tokens(
            StubProviderTokens::with_user_row(Some(github_row("manual", "ghp_user"))),
            service_with(vec![]),
        )
        .with_workspace_credentials(PinnedCredentials::holding(
            "ws_1",
            "GITHUB_TOKEN",
            "ghp_workspace",
        ));

        let token = seam
            .token(&Actor::of("u1").on_workspace(Some("ws_1")), &github_repo())
            .await
            .expect("the pin resolves");
        assert_eq!(token, "ghp_workspace");
    }

    #[tokio::test]
    async fn a_workspace_pin_answers_for_an_identity_holding_no_row_at_all() {
        let seam = tokens(StubProviderTokens::default(), service_with(vec![]))
            .with_workspace_credentials(PinnedCredentials::holding(
                "ws_1",
                "GITHUB_TOKEN",
                "ghp_workspace",
            ));
        let actor = Actor::of("u1").on_workspace(Some("ws_1"));

        assert_eq!(
            seam.token(&actor, &github_repo()).await.expect("resolves"),
            "ghp_workspace"
        );
        seam.github_is_usable(&actor)
            .await
            .expect("a pin is a usable credential on its own");
    }

    #[tokio::test]
    async fn an_actor_naming_no_workspace_resolves_its_own_row() {
        // Work no workspace is behind — a settings check, an interactive
        // request — must not pick up another workspace's account.
        let seam = tokens(
            StubProviderTokens::with_user_row(Some(github_row("manual", "ghp_user"))),
            service_with(vec![]),
        )
        .with_workspace_credentials(PinnedCredentials::holding(
            "ws_1",
            "GITHUB_TOKEN",
            "ghp_workspace",
        ));

        let token = seam
            .token(&Actor::of("u1"), &github_repo())
            .await
            .expect("the user's own row resolves");
        assert_eq!(token, "ghp_user");
    }

    #[tokio::test]
    async fn a_workspace_pinning_nothing_for_this_host_falls_through_to_the_row() {
        let seam = tokens(
            StubProviderTokens::with_user_row(Some(github_row("manual", "ghp_user"))),
            service_with(vec![]),
        )
        .with_workspace_credentials(PinnedCredentials::holding(
            "ws_1",
            "OPENAI_API_KEY",
            "sk-workspace",
        ));

        let token = seam
            .token(&Actor::of("u1").on_workspace(Some("ws_1")), &github_repo())
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
        .with_workspace_credentials(PinnedCredentials::unreadable());

        let error = seam
            .token(&Actor::of("u1").on_workspace(Some("ws_1")), &github_repo())
            .await
            .expect_err("an unreadable pin is not an absent pin");
        assert!(matches!(error, GitHostTokenError::Broken { .. }), "got {error}");
    }

    #[tokio::test]
    async fn a_workspace_pin_governs_gitlab_the_same_way() {
        let repo = GitHostRepo::parse(RepositoryProvider::Gitlab, "https://gitlab.com/acme/repo")
            .expect("parses");
        let seam = tokens(StubProviderTokens::default(), service_with(vec![]))
            .with_workspace_credentials(PinnedCredentials::holding(
                "ws_1",
                "GITLAB_TOKEN",
                "glpat_workspace",
            ));

        assert_eq!(
            seam.token(&Actor::of("u1").on_workspace(Some("ws_1")), &repo)
                .await
                .expect("resolves"),
            "glpat_workspace"
        );
        assert_eq!(
            seam.host_token(&Actor::of("u1").on_workspace(Some("ws_1")), PlainGitHost::Gitlab)
                .await
                .expect("the repository-free read resolves the same pin"),
            "glpat_workspace"
        );
    }
}
