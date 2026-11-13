//! Deciding which GitHub host a proxied request reaches, and with whose token.

use std::sync::Arc;

use moatless_adapter_github_pr::PullRequestCredits;
use moatless_core::secret_keys::GITHUB_TOKEN;
use moatless_ports::identity::{AppWriteToken, GitHostTokenError, GitHostTokenIssuer};
use moatless_sandbox_control::secrets::{
    resolve_task_actor, resolve_task_github_token, resolve_workspace_credentials,
};
use moatless_types::access_control::Actor;
use moatless_types::http::HttpStreamTimeouts;
use moatless_workspace_ports::{WorkspaceCredentialReader, WorkspaceResolver};
use moatless_workspace_types::repository::{GitHostRepo, Repository, RepositoryProvider};
use tracing::{debug, warn};

use crate::github_policy::GitHubPolicy;
use crate::proxy::caller::{CallerError, SandboxCallers, SandboxScope};

/// GitHub's release-asset upload host, which `gh release upload` posts to.
pub const GITHUB_UPLOADS_BASE: &str = "https://uploads.github.com";

/// The GitHub host a proxied request is for: the first segment after
/// `/github/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubUpstream {
    /// REST and GraphQL.
    Api,
    Uploads,
}

impl GitHubUpstream {
    /// The upstream a path segment names, or `None` for one this proxy does not
    /// forward to, so a pod cannot steer the token to a host of its choosing.
    pub fn named(segment: &str) -> Option<Self> {
        match segment {
            "api" => Some(Self::Api),
            "uploads" => Some(Self::Uploads),
            _ => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitHubProxyError {
    /// Every rejection of the caller's own task token.
    #[error("{0}")]
    Unauthorized(String),
    /// The sandbox resolved, but its identity holds no GitHub token. Returned
    /// to `gh`, which prints it, so it names what to connect.
    #[error("{0}")]
    NoGitHubIdentity(String),
    /// A store or the upstream failed; the caller's request may succeed later.
    #[error("{0}")]
    Upstream(String),
}

/// Resolves a proxied GitHub API request to an upstream URL and a token.
pub struct GitHubProxy {
    callers: Arc<SandboxCallers>,
    workspaces: Arc<dyn WorkspaceResolver>,
    workspace_credentials: Arc<dyn WorkspaceCredentialReader>,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    /// [`GitHubUpstream::Api`]'s root, without a trailing `/`.
    api_base: String,
    /// [`GitHubUpstream::Uploads`]'s root, without a trailing `/`.
    uploads_base: String,
    /// Decides which writes a sandbox may send.
    pub(crate) policy: Arc<GitHubPolicy>,
    pub(crate) http: reqwest::Client,
}

/// Idle, not total: a release asset streams for as long as it is big.
///
/// Redirects are returned to `gh` rather than followed: a log or artifact
/// download redirects to signed storage, which the pod fetches itself, so this
/// proxy only ever sends the token to the two hosts [`GitHubUpstream`] names.
#[expect(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "sets the timeouts `Config::http` holds, and fails as `Client::new` would"
)]
fn http_client(timeouts: HttpStreamTimeouts) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(timeouts.connect)
        .read_timeout(timeouts.idle)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(moatless_core::USER_AGENT)
        .build()
        .expect("the TLS backend initialises")
}

impl GitHubProxy {
    #[expect(
        clippy::too_many_arguments,
        reason = "one port per lookup a proxied request makes"
    )]
    pub(crate) fn new(
        callers: Arc<SandboxCallers>,
        workspaces: Arc<dyn WorkspaceResolver>,
        workspace_credentials: Arc<dyn WorkspaceCredentialReader>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        api_base: &str,
        uploads_base: &str,
        policy: Arc<GitHubPolicy>,
        timeouts: HttpStreamTimeouts,
    ) -> Self {
        Self {
            callers,
            workspaces,
            workspace_credentials,
            git_host_tokens,
            api_base: api_base.trim_end_matches('/').to_string(),
            uploads_base: uploads_base.trim_end_matches('/').to_string(),
            policy,
            http: http_client(timeouts),
        }
    }

    /// The upstream URL for `path_and_query`, which starts with `/`.
    pub fn upstream_url(&self, upstream: GitHubUpstream, path_and_query: &str) -> String {
        let base = match upstream {
            GitHubUpstream::Api => &self.api_base,
            GitHubUpstream::Uploads => &self.uploads_base,
        };
        format!("{base}{path_and_query}")
    }

    /// Verify `task_token` and read the sandbox it was minted for.
    pub async fn authenticate(
        &self,
        task_token: Option<&str>,
    ) -> Result<SandboxScope, GitHubProxyError> {
        self.callers
            .authenticate(task_token)
            .await
            .map_err(|e| match e {
                CallerError::Unauthorized(_) | CallerError::SandboxGone => {
                    GitHubProxyError::Unauthorized(e.to_string())
                }
                CallerError::Upstream(message) => GitHubProxyError::Upstream(message),
            })
    }

    /// The Workspace's pinned token, else the run-as identity's for
    /// `requested`, else its token for the primary repository. Resolved per
    /// request, so an expired token is replaced rather than presented.
    ///
    /// `requested` is any repository the request names, not only one Moatless
    /// knows: the issuer's cap by the Task owner's role is the boundary.
    pub async fn token(
        &self,
        scope: &SandboxScope,
        requested: Option<&GitHostRepo>,
    ) -> Result<String, GitHubProxyError> {
        if let Some(token) = self.pinned(scope).await? {
            return Ok(token);
        }
        let actor = self.actor(scope).await;
        // A capped identity's primary-repository token cannot see a sibling
        // repository. Falling back keeps a public repository the identity has
        // no role on, or an owner without the App, readable as before.
        if let Some(requested) = requested {
            match self
                .git_host_tokens
                .token(&actor, &requested.credential_account())
                .await
            {
                Ok(token) => return Ok(token),
                Err(error) => {
                    debug!(%error, repository = %requested, "No token for the requested repository, presenting the primary one");
                }
            }
        }

        let repository = self.primary_repository(scope).await?;
        match resolve_task_github_token(Some(&self.git_host_tokens), &actor, repository.as_ref())
            .await
        {
            Some(Ok(token)) => Ok(token),
            Some(Err(e)) => Err(credential_error(&e)),
            None => Err(GitHubProxyError::NoGitHubIdentity(
                "This task has no GitHub identity: its workspace's primary repository is not on \
                 GitHub or it places none, and the workspace pins no GITHUB_TOKEN"
                    .to_string(),
            )),
        }
    }

    /// The App's token for a write on `repository`, else the primary
    /// repository, per [`GitHostTokenIssuer::github_app_write`]; `None` means
    /// send it with [`Self::token`].
    pub async fn app_write(
        &self,
        scope: &SandboxScope,
        repository: Option<GitHostRepo>,
    ) -> Result<Option<AppWriteToken>, GitHubProxyError> {
        if self.pinned(scope).await?.is_some() {
            return Ok(None);
        }
        let repository = match repository {
            Some(repository) => Some(repository),
            None => self
                .primary_repository(scope)
                .await?
                .and_then(|primary| primary.remote?.host_repo),
        };
        let Some(repository) = repository.filter(|r| r.provider() == RepositoryProvider::Github)
        else {
            return Ok(None);
        };
        let actor = self.actor(scope).await;
        self.git_host_tokens
            .github_app_write(&actor, repository.owner(), repository.name())
            .await
            .map_err(|error| {
                warn!(%error, repository = %repository, "No App token for a proxied GitHub write");
                match error {
                    GitHostTokenError::Forbidden { source, .. } => {
                        GitHubProxyError::NoGitHubIdentity(source.to_string())
                    }
                    error => credential_error(&error),
                }
            })
    }

    /// The GitHub login an authored write names in its text: whom `scope`'s
    /// [`Attribution`](moatless_types::attribution::Attribution) credits,
    /// unless GitHub already shows them as its author, which a write `by_app`
    /// never does.
    pub async fn credited_login(&self, scope: &SandboxScope, by_app: bool) -> Option<String> {
        let credited = self
            .git_host_tokens
            .credited_github_login(&scope.attribution)
            .await
            .unwrap_or_else(|error| {
                warn!(%error, sandbox_id = %scope.sandbox, "Failed to read who a GitHub write credits");
                None
            });
        let (person, login) = credited?;
        if by_app {
            return Some(login);
        }
        (self.actor(scope).await.user_id() != person).then_some(login)
    }

    /// Whom an opened pull request's description names above its text, per
    /// [`moatless_adapter_github_pr::pull_request_credits`].
    pub async fn pull_request_credits(&self, scope: &SandboxScope) -> PullRequestCredits {
        moatless_adapter_github_pr::pull_request_credits(
            self.git_host_tokens.as_ref(),
            &scope.attribution,
        )
        .await
    }

    /// The repository GraphQL node `id` belongs to, asked as `scope`'s own
    /// identity; `None` when GitHub does not say.
    pub async fn repository_of_node(&self, scope: &SandboxScope, id: &str) -> Option<GitHostRepo> {
        const QUERY: &str = "query($id: ID!) { node(id: $id) { \
            ... on Repository { nameWithOwner } \
            ... on RepositoryNode { repository { nameWithOwner } } \
            ... on PullRequestReviewThread { pullRequest { repository { nameWithOwner } } } } }";
        let token = self.token(scope, None).await.ok()?;
        let response = self
            .http
            .post(self.upstream_url(GitHubUpstream::Api, "/graphql"))
            .bearer_auth(token)
            .json(&serde_json::json!({ "query": QUERY, "variables": { "id": id } }))
            .send()
            .await
            .ok()?;
        let json: serde_json::Value = response.json().await.ok()?;
        let node = json.pointer("/data/node")?;
        let slug = [
            "/nameWithOwner",
            "/repository/nameWithOwner",
            "/pullRequest/repository/nameWithOwner",
        ]
        .iter()
        .find_map(|pointer| node.pointer(pointer)?.as_str())?;
        let (owner, name) = slug.split_once('/')?;
        Some(GitHostRepo::github(owner, name))
    }

    async fn pinned(&self, scope: &SandboxScope) -> Result<Option<String>, GitHubProxyError> {
        let pinned = resolve_workspace_credentials(
            Some(&self.workspace_credentials),
            Some(&scope.workspace),
        )
        .await
        .map_err(|e| {
            warn!(error = %e, workspace_id = %scope.workspace, "Failed to read the workspace credentials");
            GitHubProxyError::Upstream("Failed to read the workspace credentials".to_string())
        })?;
        Ok(pinned.get(GITHUB_TOKEN).cloned())
    }

    async fn actor(&self, scope: &SandboxScope) -> Actor {
        resolve_task_actor(
            Some(&self.workspaces),
            Some(&scope.workspace),
            scope.run_as.as_deref(),
            &scope.user,
        )
        .await
    }

    async fn primary_repository(
        &self,
        scope: &SandboxScope,
    ) -> Result<Option<Repository>, GitHubProxyError> {
        Ok(self
            .workspaces
            .placements(&scope.workspace)
            .await
            .map_err(|e| {
                warn!(error = %e, workspace_id = %scope.workspace, "Failed to read the workspace placements");
                GitHubProxyError::Upstream("Failed to read the workspace".to_string())
            })?
            .into_iter()
            .next()
            .and_then(|placement| placement.repository))
    }
}

fn credential_error(error: &GitHostTokenError) -> GitHubProxyError {
    warn!(%error, "No GitHub token for a proxied GitHub request");
    match error {
        GitHostTokenError::NoCredential { .. } | GitHostTokenError::Forbidden { .. } => {
            GitHubProxyError::NoGitHubIdentity(format!("This task has no GitHub identity: {error}"))
        }
        GitHostTokenError::Broken { .. } | GitHostTokenError::Store(_) => {
            GitHubProxyError::Upstream(error.to_string())
        }
    }
}
