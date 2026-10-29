//! Deciding which GitHub host a proxied request reaches, and with whose token.

use std::sync::Arc;

use moatless_core::sandbox_jwt::{
    GenerationError, TokenGenerations, check_generation, verify_user_auth_token,
};
use moatless_core::secret_keys::GITHUB_TOKEN;
use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer};
use moatless_sandbox_control::secrets::{
    resolve_task_actor, resolve_task_github_token, resolve_workspace_credentials,
};
use moatless_types::http::HttpStreamTimeouts;
use moatless_workspace_ports::{WorkspaceCredentialReader, WorkspaceResolver};
use moatless_workspace_types::repository::GitHostRepo;
use tracing::{debug, warn};

use crate::sandbox::lifecycle::SandboxIngress;

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

/// The sandbox a verified task token was minted for, and the identities its
/// GitHub token resolves against.
#[derive(Debug, Clone)]
pub struct SandboxScope {
    pub sandbox: String,
    /// The Sandbox's driver, else its owner, which a run-as identity
    /// overrides.
    pub user: String,
    pub workspace: String,
    /// The run-as identity the Sandbox row pins, ahead of the Workspace's.
    pub run_as: Option<String>,
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
    sandboxes: Arc<SandboxIngress>,
    token_generations: Arc<dyn TokenGenerations>,
    workspaces: Arc<dyn WorkspaceResolver>,
    workspace_credentials: Arc<dyn WorkspaceCredentialReader>,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    /// `None` in a deployment that mints no task tokens, which refuses every
    /// proxied request rather than admitting an unverifiable one.
    task_token_secret: Option<String>,
    /// [`GitHubUpstream::Api`]'s root, without a trailing `/`.
    api_base: String,
    /// [`GitHubUpstream::Uploads`]'s root, without a trailing `/`.
    uploads_base: String,
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
        reason = "one port per lookup the token resolution makes"
    )]
    pub(crate) fn new(
        sandboxes: Arc<SandboxIngress>,
        token_generations: Arc<dyn TokenGenerations>,
        workspaces: Arc<dyn WorkspaceResolver>,
        workspace_credentials: Arc<dyn WorkspaceCredentialReader>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        task_token_secret: Option<String>,
        api_base: &str,
        uploads_base: &str,
        timeouts: HttpStreamTimeouts,
    ) -> Self {
        Self {
            sandboxes,
            token_generations,
            workspaces,
            workspace_credentials,
            git_host_tokens,
            task_token_secret,
            api_base: api_base.trim_end_matches('/').to_string(),
            uploads_base: uploads_base.trim_end_matches('/').to_string(),
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
        let task_token = task_token.ok_or_else(|| {
            GitHubProxyError::Unauthorized("No task token was presented".to_string())
        })?;
        let secret = self.task_token_secret.as_deref().ok_or_else(|| {
            GitHubProxyError::Unauthorized(
                "This deployment verifies no task tokens, so it proxies no GitHub traffic"
                    .to_string(),
            )
        })?;
        let claims = verify_user_auth_token(task_token, secret)
            .map_err(|e| GitHubProxyError::Unauthorized(format!("Invalid task token: {e}")))?;
        let sandbox_id = claims.sandbox_id.clone().ok_or_else(|| {
            GitHubProxyError::Unauthorized("The task token names no sandbox".to_string())
        })?;
        check_generation(
            self.token_generations.as_ref(),
            &sandbox_id,
            claims.generation,
        )
        .await
        .map_err(|e| match e {
            GenerationError::Stale { .. } | GenerationError::NoSandbox => {
                GitHubProxyError::Unauthorized(e.to_string())
            }
            GenerationError::Lookup(_) => GitHubProxyError::Upstream(e.to_string()),
        })?;

        let workspace_id = self
            .sandboxes
            .workspace_id(&sandbox_id)
            .await
            .map_err(|e| {
                warn!(error = %e, sandbox_id, "Failed to read the sandbox for a proxied GitHub request");
                GitHubProxyError::Upstream("Failed to read the sandbox".to_string())
            })?
            .ok_or_else(|| {
                GitHubProxyError::Unauthorized(
                    "The task token names a sandbox that no longer exists".to_string(),
                )
            })?;
        // Fails closed rather than degrading to the token's user: that is the
        // driver only while the read agrees.
        let identity = self
            .sandboxes
            .identity(&sandbox_id)
            .await
            .map_err(|e| {
                warn!(error = %e, sandbox_id, "Failed to read the sandbox identity");
                GitHubProxyError::Upstream("Failed to read the sandbox".to_string())
            })?
            .unwrap_or_default();
        Ok(SandboxScope {
            sandbox: sandbox_id,
            user: identity.acting_user_id.unwrap_or(claims.user_id),
            workspace: workspace_id,
            run_as: identity.pinned_run_as,
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
        let pinned = resolve_workspace_credentials(
            Some(&self.workspace_credentials),
            Some(&scope.workspace),
        )
        .await
        .map_err(|e| {
            warn!(error = %e, workspace_id = %scope.workspace, "Failed to read the workspace credentials");
            GitHubProxyError::Upstream("Failed to read the workspace credentials".to_string())
        })?;
        if let Some(token) = pinned.get(GITHUB_TOKEN) {
            return Ok(token.clone());
        }

        let actor = resolve_task_actor(
            Some(&self.workspaces),
            Some(&scope.workspace),
            scope.run_as.as_deref(),
            &scope.user,
        )
        .await;
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

        let repository = self
            .workspaces
            .placements(&scope.workspace)
            .await
            .map_err(|e| {
                warn!(error = %e, workspace_id = %scope.workspace, "Failed to read the workspace placements");
                GitHubProxyError::Upstream("Failed to read the workspace".to_string())
            })?
            .into_iter()
            .next()
            .and_then(|placement| placement.repository);

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
