//! Deciding which upstream a proxied git request reaches, and with whose
//! credential.

use std::sync::Arc;

use moatless_core::sandbox_jwt::verify_user_auth_token;
use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer};
use moatless_ports::workspace::WorkspaceResolver;
use moatless_sandbox_control::secrets::resolve_task_actor;
use moatless_types::repository::{GitHostRepo, Repository, RepositoryProvider};
use tracing::warn;

use crate::sandbox::lifecycle::SandboxIngress;

/// Where one proxied request goes, and what it presents on arrival.
pub struct Upstream {
    /// `https://{host}/{owner}/{name}.git`, rebuilt from the parsed remote
    /// rather than taken from it. A Workspace controls the stored `git_url`, and
    /// honouring its scheme would let one downgrade the connection its own
    /// credential travels over.
    pub base_url: String,
    /// Presented as the HTTP Basic password, which is how git hosts take a
    /// token over smart-HTTP.
    pub token: String,
    /// For the request log. The token is never logged.
    pub host: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GitProxyError {
    /// Every rejection of the caller's own credential. The message is returned
    /// to git, which shows it to the person watching the clone.
    #[error("{0}")]
    Unauthorized(String),
    /// No repository this token may reach answers to that id. Deliberately not
    /// distinguished from "no such repository": the id is guessable, and a
    /// distinct 403 would confirm one that exists.
    #[error("Repository not found")]
    NotFound,
    /// The repository resolved but its credential did not.
    #[error("{0}")]
    Upstream(String),
}

/// Resolves a proxied git request to an upstream and a credential.
pub struct GitProxy {
    sandboxes: Arc<SandboxIngress>,
    workspaces: Arc<dyn WorkspaceResolver>,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    /// `None` in a deployment that mints no task tokens, which refuses every
    /// proxied request rather than admitting an unverifiable one.
    task_token_secret: Option<String>,
    pub(crate) http: reqwest::Client,
}

impl GitProxy {
    pub(crate) fn new(
        sandboxes: Arc<SandboxIngress>,
        workspaces: Arc<dyn WorkspaceResolver>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        task_token_secret: Option<String>,
    ) -> Self {
        Self {
            sandboxes,
            workspaces,
            git_host_tokens,
            task_token_secret,
            // `FORWARDED_REQUEST_HEADERS` copies git's own `User-Agent` over
            // this default, so it covers only a client that sends none. See
            // `moatless_core::USER_AGENT`.
            http: reqwest::Client::builder()
                .user_agent(moatless_core::USER_AGENT)
                .build()
                .unwrap_or_else(|error| {
                    warn!(%error, "Failed to build the git proxy's HTTP client");
                    reqwest::Client::new()
                }),
        }
    }

    /// Resolve `repository_id` for the sandbox `task_token` was minted for.
    ///
    /// The credential is the one the sandbox's *run-as* identity holds, which is
    /// not the identity in the token: a Workspace's `run_as_user_id` overrides
    /// it, and a credential the Workspace pins overrides that in turn. Reading
    /// the token's own `userId` would hand the pod the starter's credential and
    /// silently ignore both.
    pub async fn resolve(
        &self,
        repository_id: &str,
        task_token: &str,
    ) -> Result<Upstream, GitProxyError> {
        let secret = self.task_token_secret.as_deref().ok_or_else(|| {
            GitProxyError::Unauthorized(
                "This deployment verifies no task tokens, so it proxies no git traffic".to_string(),
            )
        })?;
        let claims = verify_user_auth_token(task_token, secret)
            .map_err(|e| GitProxyError::Unauthorized(format!("Invalid task token: {e}")))?;
        // `generate_user_auth_token` binds the token to a Sandbox, not to a
        // Task: a shared Sandbox runs several Member tasks off one pod and one
        // checkout, so the Sandbox is what a clone belongs to.
        let sandbox_id = claims.sandbox_id.as_deref().ok_or_else(|| {
            GitProxyError::Unauthorized("The task token names no sandbox".to_string())
        })?;

        let workspace_id = self
            .sandboxes
            .workspace_id(sandbox_id)
            .await
            .map_err(|e| {
                warn!(error = %e, sandbox_id, "Failed to read the sandbox for a proxied git request");
                GitProxyError::Upstream("Failed to read the sandbox".to_string())
            })?
            .ok_or(GitProxyError::NotFound)?;

        // The placement list is the authorization: a sandbox reaches the
        // repositories its own Workspace checks out and no others.
        let repository = self
            .workspaces
            .placements(&workspace_id)
            .await
            .map_err(|e| {
                warn!(error = %e, workspace_id, "Failed to read the workspace placements");
                GitProxyError::Upstream("Failed to read the workspace".to_string())
            })?
            .into_iter()
            .find(|placement| placement.repository_id == repository_id)
            .and_then(|placement| placement.repository)
            .ok_or(GitProxyError::NotFound)?;

        let repo = git_host_repo(&repository).ok_or(GitProxyError::NotFound)?;
        let run_as_user_id = self
            .sandboxes
            .run_as_user_id(sandbox_id)
            .await
            .unwrap_or_else(|e| {
                warn!(error = %e, sandbox_id, "Failed to read the sandbox run-as identity");
                None
            });
        let actor = resolve_task_actor(
            Some(&self.workspaces),
            Some(&workspace_id),
            run_as_user_id.as_deref(),
            &claims.user_id,
        )
        .await;

        let token = self
            .git_host_tokens
            .token(&actor, &repo)
            .await
            .map_err(|e| credential_error(&e, repo.host()))?;
        let host = repo.host().ok_or(GitProxyError::NotFound)?.to_string();
        Ok(Upstream {
            base_url: format!("https://{host}/{}/{}.git", repo.owner(), repo.name()),
            host,
            token,
        })
    }
}

/// The remote a proxied request forwards to, or `None` when the repository
/// names none this proxy can address.
///
/// Only Forgejo is proxied. Every other provider's clone URL reaches the pod
/// directly, so a request naming one arrived at the wrong route.
fn git_host_repo(repository: &Repository) -> Option<GitHostRepo> {
    if repository.provider? != RepositoryProvider::Forgejo {
        return None;
    }
    GitHostRepo::parse(
        RepositoryProvider::Forgejo,
        repository.remote_url.as_deref()?,
    )
}

fn credential_error(error: &GitHostTokenError, host: Option<&str>) -> GitProxyError {
    warn!(%error, host, "No credential for a proxied git request");
    match error {
        GitHostTokenError::NoCredential { .. } | GitHostTokenError::Forbidden { .. } => {
            GitProxyError::Unauthorized(error.to_string())
        }
        GitHostTokenError::Broken { .. } | GitHostTokenError::Store(_) => {
            GitProxyError::Upstream(error.to_string())
        }
    }
}
