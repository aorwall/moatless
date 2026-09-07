//! Ordered, administrator-requested repository configuration synchronization.

use moatless_api_schemas::loops::LoopSyncSummary;
use moatless_api_schemas::workspace::WorkspaceSyncSummary;
use moatless_loops::loops::sync::LoopSyncService;
use moatless_types::access_control::ScopeContext;
use moatless_workspaces::repository::{
    dao::RepositoryDao, error::RepositoryError, models::Repository,
};
use moatless_workspaces::workspace::git_sync::{GitSyncService, SyncForce, log_workspace_sync_for};

/// Repository configuration and both synchronization summaries.
pub struct SyncResult {
    pub repository: Repository,
    pub loops: LoopSyncSummary,
    pub workspaces: WorkspaceSyncSummary,
}

/// Synchronize Loop approval/configuration before best-effort workspace refresh.
pub async fn sync_config(
    repositories: &dyn RepositoryDao,
    loop_sync: &LoopSyncService,
    git_sync: &GitSyncService,
    ctx: &ScopeContext,
    repository_id: &str,
) -> Result<SyncResult, RepositoryError> {
    if !ctx.is_admin {
        return Err(RepositoryError::Forbidden(
            "Only administrators can synchronize repository configuration".to_string(),
        ));
    }
    let repo = repositories
        .find_by_id(repository_id, ctx)
        .await?
        .ok_or_else(|| RepositoryError::NotFound(repository_id.to_string()))?;

    // Explicit, user-initiated sync: git is authoritative again, so overridden
    // workspaces are reclaimed. The automatic sync on task start is not.
    let (updated, loops, workspaces) =
        sync_repo_config(loop_sync, git_sync, &repo, ctx, &SyncForce::All).await?;

    Ok(SyncResult {
        repository: updated,
        loops,
        workspaces,
    })
}

async fn sync_repo_config(
    loop_sync: &LoopSyncService,
    git_sync: &GitSyncService,
    repo: &Repository,
    ctx: &ScopeContext,
    force: &SyncForce,
) -> Result<(Repository, LoopSyncSummary, WorkspaceSyncSummary), RepositoryError> {
    if repo.remote_url.is_none() {
        return Err(RepositoryError::BadRequest(
            "Repository has no remote URL".to_string(),
        ));
    }

    // This is the sole repository-wide Loop sync path. Surface fetch and
    // reconciliation failures so an administrator never mistakes a skipped
    // refresh for a successful approval revocation.
    let loops = loop_sync
        .sync_repo_loops_for(repo, ctx)
        .await
        .map_err(|error| RepositoryError::ProviderError(error.to_string()))?;

    let workspaces = log_workspace_sync_for(git_sync, repo, ctx, "sync-config", force).await;

    Ok((repo.clone(), loops, workspaces))
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_adapter_model::dao::PgAdapterConnectionDao;
    use moatless_loops::loops::dao::LoopDao;
    use moatless_ports::identity::{GitHostTokenError, GitHostTokenIssuer, PlainGitHost};
    use moatless_tasks::{ports::TagCatalog, tag::dao::PgTagDao};
    use moatless_types::{
        access_control::{Actor, Scope},
        repository::{GitHostRepo, RepositoryKind},
    };
    use moatless_workspaces::{repository::dao::MockRepositoryDao, workspace::dao::PgWorkspaceDao};
    use std::sync::Arc;

    struct NoGitTokens;

    #[async_trait::async_trait]
    impl GitHostTokenIssuer for NoGitTokens {
        async fn token(&self, _: &Actor, _: &GitHostRepo) -> Result<String, GitHostTokenError> {
            panic!("sync was refused before credentials")
        }
        async fn host_token(
            &self,
            _: &Actor,
            _: PlainGitHost,
        ) -> Result<String, GitHostTokenError> {
            panic!("sync was refused before credentials")
        }
        async fn github_is_usable(&self, _: &Actor) -> Result<(), GitHostTokenError> {
            panic!("sync was refused before credentials")
        }
    }

    fn services() -> (LoopSyncService, GitSyncService) {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .unwrap();
        let tokens: Arc<dyn GitHostTokenIssuer> = Arc::new(NoGitTokens);
        let loops = LoopSyncService::new(
            Arc::new(PgAdapterConnectionDao::new(pool.clone())),
            Arc::new(LoopDao::new(pool.clone())),
            Arc::new(TagCatalog::new(Arc::new(PgTagDao(pool.clone())))),
            tokens.clone(),
        );
        let git = GitSyncService::new(
            pool.clone(),
            Arc::new(PgWorkspaceDao(pool)),
            Arc::new(MockRepositoryDao::new()),
            tokens,
        );
        (loops, git)
    }

    #[tokio::test]
    async fn non_admin_is_refused_before_repository_or_sync_reads() {
        let (loops, git) = services();
        let result = sync_config(
            &MockRepositoryDao::new(),
            &loops,
            &git,
            &ScopeContext {
                user_id: "reader".into(),
                is_admin: false,
            },
            "repo",
        )
        .await;
        assert!(matches!(result, Err(RepositoryError::Forbidden(_))));
    }

    #[tokio::test]
    async fn missing_remote_is_refused_before_sync() {
        let (loops, git) = services();
        let mut repositories = MockRepositoryDao::new();
        repositories.expect_find_by_id().return_once(|_, _| {
            Ok(Some(Repository {
                id: "repo".into(),
                name: "repo".into(),
                kind: RepositoryKind::Remote,
                remote_url: None,
                default_branch: None,
                description: None,
                provider: None,
                scope: Scope::User,
                owner_user_id: None,
                created_by: None,
                deleted: false,
                created_at: None,
                updated_at: None,
            }))
        });
        let result = sync_config(
            &repositories,
            &loops,
            &git,
            &ScopeContext {
                user_id: "admin".into(),
                is_admin: true,
            },
            "repo",
        )
        .await;
        assert!(
            matches!(result, Err(RepositoryError::BadRequest(message)) if message == "Repository has no remote URL")
        );
    }
}
