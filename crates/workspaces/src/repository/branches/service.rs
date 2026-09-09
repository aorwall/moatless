use super::super::access::git_host_token_error;
use super::super::dao::RepositoryDao;
use super::super::error::RepositoryError;
use super::super::models::RepositoryPatch;
use super::super::schemas::{BranchEntry, BranchListResponse};
use super::providers::{
    fetch_forgejo_branches, fetch_github_branches, fetch_gitlab_branches, fetch_gitness_branches,
};
use crate::workspace::dao::WorkspaceDao;
use moatless_identity::credentials::{CredentialError, CredentialService};
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_types::access_control::{Actor, ScopeContext};
use moatless_types::repository::{GitHostRepo, RepositoryProvider};
use std::sync::Arc;
use tracing::{info, warn};

/// Lists repository branches with one credential gate and a shared HTTP client.
pub struct RepositoryBranchService {
    repositories: Arc<dyn RepositoryDao>,
    workspaces: Arc<dyn WorkspaceDao>,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    credentials: Arc<CredentialService>,
    http: Arc<reqwest::Client>,
}

impl RepositoryBranchService {
    /// Bind branch listing to the stores, credential issuer and HTTP client.
    pub(crate) fn new(
        repositories: Arc<dyn RepositoryDao>,
        workspaces: Arc<dyn WorkspaceDao>,
        git_host_tokens: Arc<dyn GitHostTokenIssuer>,
        credentials: Arc<CredentialService>,
        http: Arc<reqwest::Client>,
    ) -> Self {
        Self {
            repositories,
            workspaces,
            git_host_tokens,
            credentials,
            http,
        }
    }

    /// List visible repository branches after authorizing any pinned workspace credential.
    pub async fn list_branches(
        &self,
        ctx: &ScopeContext,
        repository_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<BranchListResponse, RepositoryError> {
        let Self {
            repositories,
            workspaces,
            git_host_tokens,
            credentials,
            ..
        } = self;
        let repo = repositories
            .find_by_id(repository_id, ctx)
            .await?
            .ok_or_else(|| RepositoryError::NotFound(repository_id.to_string()))?;

        if repo.default_branch.is_none() {
            info!(repository_id = %repository_id, "No default branch set, fetching from provider");
        }

        let remote_url = repo.remote_url.as_deref().ok_or_else(|| {
            RepositoryError::BadRequest("Repository has no remote URL".to_string())
        })?;

        let provider = repo.provider.unwrap_or(RepositoryProvider::Github);

        // The viewer's own credential on every host, unless they named a workspace
        // that pins one. A repository someone else shared does not lend its owner's
        // access to whoever opens it; a workspace's pin is lent to its own
        // repositories on purpose, because that is the token its Tasks clone with.
        let actor = branch_listing_actor(workspaces, ctx, repository_id, workspace_id).await?;
        let access_token = git_host_tokens
            .token(&actor, &git_host_repo(provider, remote_url)?)
            .await
            .map_err(git_host_token_error)?;

        let result = match provider {
            RepositoryProvider::Github => {
                fetch_github_branches(&self.http, remote_url, &access_token).await?
            }
            RepositoryProvider::Gitlab => {
                fetch_gitlab_branches(&self.http, remote_url, &access_token).await?
            }
            RepositoryProvider::Forgejo => {
                fetch_forgejo_branches(&self.http, remote_url, &access_token).await?
            }
            RepositoryProvider::Gitness => {
                let metadata = credentials
                    .provider_metadata(&ctx.user_id, "gitness")
                    .await
                    .map_err(credential_error)?;
                let base_url = metadata
                .as_ref()
                .and_then(|m| m.get("base_url"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    RepositoryError::BadRequest(
                        "Gitness base URL not configured. Please reconnect your Gitness account in Settings.".to_string()
                    )
                })?
                .to_string();
                fetch_gitness_branches(&self.http, remote_url, &access_token, &base_url).await?
            }
        };

        let default_branch = repo
            .default_branch
            .clone()
            .or(result.default_branch.clone());

        // Persist provider-reported default_branch back to DB if missing
        if repo.default_branch.is_none() {
            if let Some(ref branch) = default_branch {
                let patch = RepositoryPatch {
                    default_branch: Some(Some(branch.clone())),
                    ..Default::default()
                };
                if let Err(e) = repositories.update(repository_id, &patch, ctx).await {
                    warn!(repository_id = %repository_id, error = %e, "Failed to persist default branch");
                }
            }
        }

        let branches = result.branches;

        // Sort: default branch first, then alphabetical
        let mut entries: Vec<BranchEntry> = branches
            .into_iter()
            .map(|name| {
                let is_default = default_branch.as_deref() == Some(&name);
                BranchEntry { name, is_default }
            })
            .collect();

        entries.sort_by(|a, b| {
            // Default branch always first
            match (a.is_default, b.is_default) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });

        Ok(BranchListResponse {
            branches: entries,
            default_branch,
        })
    }
}

async fn branch_listing_actor(
    workspaces: &Arc<dyn WorkspaceDao>,
    ctx: &ScopeContext,
    repository_id: &str,
    workspace_id: Option<&str>,
) -> Result<Actor, RepositoryError> {
    let caller = Actor::caller(ctx);
    let Some(workspace_id) = workspace_id.filter(|id| !id.is_empty()) else {
        return Ok(caller);
    };

    if workspaces.find_by_id(workspace_id, ctx).await?.is_none() {
        return Err(RepositoryError::Forbidden(format!(
            "Workspace {workspace_id} not found or not accessible"
        )));
    }

    let attached = workspaces
        .find_repos(workspace_id)
        .await?
        .iter()
        .any(|placement| placement.repository_id == repository_id);
    if !attached {
        return Err(RepositoryError::BadRequest(format!(
            "Repository {repository_id} is not part of workspace {workspace_id}"
        )));
    }

    Ok(caller.on_workspace(Some(workspace_id)))
}

fn git_host_repo(
    provider: RepositoryProvider,
    remote_url: &str,
) -> Result<GitHostRepo, RepositoryError> {
    GitHostRepo::parse(provider, remote_url).ok_or_else(|| {
        RepositoryError::BadRequest(format!(
            "Cannot parse {} URL: {remote_url}",
            provider.as_str()
        ))
    })
}

/// A credential read failure, mapped to the status the branch listing answers
/// with. A store error stays a 500 through [`RepositoryError::Db`].
fn credential_error(error: CredentialError) -> RepositoryError {
    match error {
        CredentialError::Database(source) => RepositoryError::Db(source),
        other => RepositoryError::ProviderError(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::models::Repository;
    use crate::repository::mutations::preserved_user_scope;
    use crate::repository::remote_url::normalize_remote_url;
    use moatless_types::access_control::Scope;
    use moatless_types::repository::RepositoryKind;

    fn repository(scope: Scope, owner_user_id: Option<&str>) -> Repository {
        Repository {
            id: "repo-1".to_string(),
            name: "repo".to_string(),
            kind: RepositoryKind::Remote,
            remote_url: Some("https://github.com/acme/repo.git".to_string()),
            default_branch: None,
            description: None,
            provider: Some(RepositoryProvider::Github),
            scope,
            owner_user_id: owner_user_id.map(str::to_string),
            created_by: Some("creator".to_string()),
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn normalize_remote_url_strips_and_lowercases() {
        assert_eq!(
            normalize_remote_url("https://github.com/Owner/Repo.git"),
            "https://github.com/owner/repo"
        );
        assert_eq!(
            normalize_remote_url("https://github.com/Owner/Repo/"),
            "https://github.com/owner/repo"
        );
        assert_eq!(
            normalize_remote_url("  https://GitHub.com/o/r.git/  "),
            "https://github.com/o/r"
        );
        // Two URLs differing only by trailing `.git`/slash/case normalize equal.
        assert_eq!(
            normalize_remote_url("https://github.com/a/b"),
            normalize_remote_url("https://github.com/A/B.git/")
        );
    }

    #[test]
    fn resubmitting_user_scope_preserves_owner() {
        let existing = repository(Scope::User, Some("original-owner"));
        let owner_user_id = preserved_user_scope(&existing).unwrap();
        assert_eq!(owner_user_id, "original-owner");
    }

    #[test]
    fn ownerless_repository_cannot_move_to_user_scope() {
        let existing = repository(Scope::Global, None);
        let err = preserved_user_scope(&existing)
            .expect_err("ownerless repository needs an explicit future owner-picker");
        assert!(matches!(err, RepositoryError::BadRequest(_)));
    }

    // --- branch_listing_actor ----------------------------------------------

    use crate::workspace::dao::MockWorkspaceDao;
    use moatless_types::workspace::{Workspace, WorkspaceKind, WorkspaceRepo, WorkspaceSource};

    fn ctx() -> ScopeContext {
        ScopeContext {
            user_id: "viewer".to_string(),
            is_admin: false,
        }
    }

    fn workspace() -> Workspace {
        Workspace {
            id: "ws-1".to_string(),
            name: "ws".to_string(),
            description: None,
            kind: WorkspaceKind::Single,
            docker_image: None,
            docker_access: None,
            sandbox_service_account: None,
            system_prompt: None,
            setup_commands: None,
            env_vars: None,
            servers: None,
            scripts: None,
            resources: None,
            network_profile: None,
            source: WorkspaceSource::Manual,
            source_repository_id: None,
            source_key: None,
            source_config_path: None,
            scope: Scope::User,
            owner_user_id: Some("viewer".to_string()),
            run_as_user_id: None,
            created_by: Some("viewer".to_string()),
            deleted: false,
            created_at: None,
            updated_at: None,
        }
    }

    fn placement(repository_id: &str) -> WorkspaceRepo {
        WorkspaceRepo {
            id: "wsr-1".to_string(),
            workspace_id: "ws-1".to_string(),
            repository_id: repository_id.to_string(),
            branch: None,
            mount_name: None,
            is_primary: true,
            position: 0,
            created_at: None,
        }
    }

    /// A workspace the caller can read lends its pin to its own repository —
    /// the actor names the workspace, which is what `GitHostTokens` resolves
    /// the pinned credential from.
    #[tokio::test]
    async fn a_readable_workspace_lends_its_pin_to_its_own_repository() {
        let mut dao = MockWorkspaceDao::new();
        dao.expect_find_by_id()
            .returning(|_, _| Ok(Some(workspace())));
        dao.expect_find_repos()
            .returning(|_| Ok(vec![placement("repo-1")]));
        let workspaces: Arc<dyn WorkspaceDao> = Arc::new(dao);

        let actor = branch_listing_actor(&workspaces, &ctx(), "repo-1", Some("ws-1"))
            .await
            .expect("the workspace is readable and holds the repository");

        assert_eq!(actor.workspace_id(), Some("ws-1"));
        assert_eq!(actor.user_id(), "viewer");
    }

    /// Naming no workspace lists as the caller's own identity, which is what
    /// every repository-scoped listing did before a pin could exist.
    #[tokio::test]
    async fn naming_no_workspace_lists_as_the_caller() {
        let workspaces: Arc<dyn WorkspaceDao> = Arc::new(MockWorkspaceDao::new());

        let actor = branch_listing_actor(&workspaces, &ctx(), "repo-1", None)
            .await
            .expect("no workspace is not a failure");

        assert_eq!(actor.workspace_id(), None);
    }

    /// `find_by_id` is scope-filtered, so a workspace the caller cannot read
    /// answers `None` — and its credential is not theirs to present.
    #[tokio::test]
    async fn an_unreadable_workspace_lends_nothing() {
        let mut dao = MockWorkspaceDao::new();
        dao.expect_find_by_id().returning(|_, _| Ok(None));
        let workspaces: Arc<dyn WorkspaceDao> = Arc::new(dao);

        let err = branch_listing_actor(&workspaces, &ctx(), "repo-1", Some("ws-1"))
            .await
            .expect_err("a workspace the caller cannot read is not one to act for");

        assert!(matches!(err, RepositoryError::Forbidden(_)));
    }

    /// The pin reaches the workspace's own repositories and no others. Without
    /// this, any repository row the caller can read would borrow the credential
    /// and see branches it was never granted.
    #[tokio::test]
    async fn a_repository_outside_the_workspace_cannot_borrow_its_pin() {
        let mut dao = MockWorkspaceDao::new();
        dao.expect_find_by_id()
            .returning(|_, _| Ok(Some(workspace())));
        dao.expect_find_repos()
            .returning(|_| Ok(vec![placement("repo-1")]));
        let workspaces: Arc<dyn WorkspaceDao> = Arc::new(dao);

        let err = branch_listing_actor(&workspaces, &ctx(), "someone-elses-repo", Some("ws-1"))
            .await
            .expect_err("a repository the workspace does not hold gets no pin");

        assert!(matches!(err, RepositoryError::BadRequest(_)));
    }
}
