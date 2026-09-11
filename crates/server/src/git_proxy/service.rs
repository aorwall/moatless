//! Deciding which upstream a proxied git request reaches, and with whose
//! credential.

use std::sync::Arc;

use moatless_core::sandbox_jwt::verify_user_auth_token;
use moatless_ports::identity::{CommitIdentity, GitHostTokenError, GitHostTokenIssuer};
use moatless_ports::workspace::{RepositoryReader, WorkspaceResolver};
use moatless_sandbox_control::secrets::resolve_task_actor;
use moatless_types::access_control::{ScopeContext, ScopedResource};
use moatless_types::repository::{GitHostRepo, Repository};
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
    repositories: Arc<dyn RepositoryReader>,
    users: Arc<dyn CommitIdentity>,
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
        repositories: Arc<dyn RepositoryReader>,
        users: Arc<dyn CommitIdentity>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        task_token_secret: Option<String>,
    ) -> Self {
        Self {
            sandboxes,
            workspaces,
            repositories,
            users,
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
    ///
    /// Authorization is the other way round, and it is two arms. The Workspace's
    /// placement list answers first. A repository the placements do not cover
    /// falls through to every repository the token's own identity may use, which
    /// is what `moat repos clone <repository_id>` reaches.
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

        let placed = self
            .workspaces
            .placements(&workspace_id)
            .await
            .map_err(|e| {
                warn!(error = %e, workspace_id, "Failed to read the workspace placements");
                GitProxyError::Upstream("Failed to read the workspace".to_string())
            })?
            .into_iter()
            .find(|placement| placement.repository_id == repository_id)
            .and_then(|placement| placement.repository);

        let repository = match placed {
            Some(repository) => repository,
            None => usable_repository(
                &self.repositories,
                &self.users,
                &claims.user_id,
                repository_id,
            )
            .await
            .ok_or(GitProxyError::NotFound)?,
        };

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
        let host = repo.host().to_string();
        Ok(Upstream {
            base_url: format!("https://{host}/{}/{}.git", repo.owner(), repo.name()),
            host,
            token,
        })
    }
}

/// The repository `user_id` may fetch and push by id, or `None` when there is
/// none.
///
/// `user_id` is the pod's own identity, the task's `created_by`, and the same
/// one `sandbox::grpc_server::resolve_write` gates a pod's task writes on. It is
/// deliberately not the run-as actor the credential comes from: a Workspace
/// pinned to a bot would otherwise lend every member the bot's whole reach. The
/// *credential* is still that actor's, so a caller who may read a
/// `Scope::Global` row reaches it on the bot's token.
///
/// The gate is `ScopeContext::can_use_repository`, the same one that admits a
/// repository to a Workspace. Admission also requires write on the Workspace, so
/// a member of a bot-pinned `Scope::Global` Workspace pushes where no placement
/// they could write would reach.
///
/// Returns `None` for a repository the reader hides, for one the caller may not
/// use, and for a read that failed, so a caller cannot tell the three apart.
async fn usable_repository(
    repositories: &Arc<dyn RepositoryReader>,
    users: &Arc<dyn CommitIdentity>,
    user_id: &str,
    repository_id: &str,
) -> Option<Repository> {
    let ctx = ScopeContext {
        user_id: user_id.to_string(),
        is_admin: matches!(users.is_platform_admin(user_id).await, Ok(true)),
    };
    let repository = repositories
        .describe(&ctx, repository_id)
        .await
        .map_err(|e| {
            warn!(error = %e, repository_id, "Failed to read a repository for a proxied git request");
        })
        .ok()??;
    let owner = ScopedResource::borrowed(
        repository.scope,
        repository
            .owner_user_id
            .as_deref()
            .or(repository.created_by.as_deref()),
    );
    ctx.can_use_repository(&owner).then_some(repository)
}

/// The remote a proxied request forwards to, or `None` when the repository
/// names none this proxy can address.
///
/// The row's own provider decides, so every provider `GitHostTokenIssuer` mints
/// a token for is proxied. A row naming no provider, or a remote URL its
/// provider's parser rejects, has no host to forward to.
fn git_host_repo(repository: &Repository) -> Option<GitHostRepo> {
    GitHostRepo::parse(repository.provider?, repository.remote_url.as_deref()?)
}

fn credential_error(error: &GitHostTokenError, host: &str) -> GitProxyError {
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

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_ports::identity::MockCommitIdentity;
    use moatless_ports::workspace::MockRepositoryReader;
    use moatless_types::access_control::Scope;
    use moatless_types::repository::{RepositoryKind, RepositoryProvider};
    use moatless_types::store::StoreError;

    const POD_USER: &str = "user-1";

    fn repository(scope: Scope, owner_user_id: Option<&str>) -> Repository {
        Repository {
            id: "repo-1".to_string(),
            name: "config-default".to_string(),
            kind: RepositoryKind::Git,
            remote_url: Some("https://git.example.com/owner/config-default.git".to_string()),
            default_branch: None,
            description: None,
            provider: Some(RepositoryProvider::Forgejo),
            scope,
            owner_user_id: owner_user_id.map(str::to_string),
            created_by: None,
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    /// A reader answering `shown` for every id, and an identity that is nobody's
    /// admin.
    fn ports(
        shown: Result<Option<Repository>, String>,
    ) -> (Arc<dyn RepositoryReader>, Arc<dyn CommitIdentity>) {
        let mut repositories = MockRepositoryReader::new();
        repositories
            .expect_describe()
            .returning(move |_, _| shown.clone().map_err(StoreError::new));
        let mut users = MockCommitIdentity::new();
        users.expect_is_platform_admin().returning(|_| Ok(false));
        (Arc::new(repositories), Arc::new(users))
    }

    #[tokio::test]
    async fn a_global_repository_outside_the_workspace_is_usable() {
        let (repositories, users) = ports(Ok(Some(repository(Scope::Global, None))));

        let found = usable_repository(&repositories, &users, POD_USER, "repo-1").await;

        assert_eq!(found.map(|repo| repo.id), Some("repo-1".to_string()));
    }

    #[tokio::test]
    async fn a_repository_the_reader_hides_is_not_usable() {
        let (repositories, users) = ports(Ok(None));

        assert!(
            usable_repository(&repositories, &users, POD_USER, "repo-1")
                .await
                .is_none()
        );
    }

    /// The scoped read already filters this one. The gate runs anyway, so a
    /// reader that widens later does not widen the proxy with it.
    #[tokio::test]
    async fn a_repository_owned_by_somebody_else_is_not_usable() {
        let (repositories, users) = ports(Ok(Some(repository(Scope::User, Some("user-2")))));

        assert!(
            usable_repository(&repositories, &users, POD_USER, "repo-1")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_failed_read_is_not_usable() {
        let (repositories, users) = ports(Err("database is down".to_string()));

        assert!(
            usable_repository(&repositories, &users, POD_USER, "repo-1")
                .await
                .is_none()
        );
    }

    #[test]
    fn every_provider_the_row_names_resolves_to_its_own_host() {
        let forgejo = repository(Scope::User, None);
        assert_eq!(git_host_repo(&forgejo).unwrap().host(), "git.example.com");

        let mut github = repository(Scope::User, None);
        github.provider = Some(RepositoryProvider::Github);
        github.remote_url = Some("https://github.com/owner/repo.git".to_string());
        let repo = git_host_repo(&github).expect("a GitHub row resolves");
        assert_eq!(repo.host(), "github.com");
        assert_eq!(repo.owner(), "owner");
    }

    #[test]
    fn a_row_with_no_provider_or_an_unparsable_remote_resolves_to_nothing() {
        let mut untagged = repository(Scope::User, None);
        untagged.provider = None;
        assert!(git_host_repo(&untagged).is_none());

        let mut ssh = repository(Scope::User, None);
        ssh.remote_url = Some("ssh://git@git.example.com:2222/owner/repo.git".to_string());
        assert!(git_host_repo(&ssh).is_none());
    }
}
