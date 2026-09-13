//! Workspaces capabilities, built over the shared pool.
//!
//! A host passes the pool and takes a capability handle. The stores behind each
//! handle stay private to this crate.

use std::sync::Arc;

use moatless_core::db::DbPool;
use moatless_identity::credentials::CredentialService;
use moatless_identity::secret::audit::SecretAudit;
use moatless_identity::secret::service::SecretCipher;
use moatless_ports::identity::GitHostTokenIssuer;
use moatless_workspace_ports::RepositoryReader;

use crate::plugin::dao::PgPluginDao;
use crate::plugin::operations::Plugins;
use crate::plugin::sync::PluginSyncService;
use crate::ports::{RepositoryProjection, WorkspaceProjection};
use crate::repository::branches::RepositoryBranchService;
use crate::repository::dao::PgRepositoryDao;
use crate::repository::operations::Repositories;
use crate::skill::fetcher::SkillFetcher;
use crate::skill::service::SkillService;
use crate::workspace::credentials::dao::PgWorkspaceCredentialDao;
use crate::workspace::credentials::service::WorkspaceCredentialService;
use crate::workspace::dao::PgWorkspaceDao;
use crate::workspace::git_sync::GitSyncService;
use crate::workspace::operations::Workspaces;

/// The `repositories` table, as a host and the repository routes reach it.
pub fn repositories(db: DbPool) -> Arc<Repositories> {
    Arc::new(Repositories::new(Arc::new(PgRepositoryDao(db))))
}

/// The `workspaces` and `workspace_repos` tables, as a host and the workspace
/// routes reach them.
pub fn workspaces(db: DbPool) -> Arc<Workspaces> {
    Arc::new(Workspaces::new(Arc::new(PgWorkspaceDao(db))))
}

/// The plugin, skill and activation tables, as a host and the plugin routes
/// reach them.
pub fn plugins(db: DbPool) -> Arc<Plugins> {
    Arc::new(Plugins::new(Arc::new(PgPluginDao(db))))
}

/// The workspace, placement and plugin stores behind the `WorkspaceResolver`
/// port.
pub fn workspace_resolver(db: DbPool) -> Arc<WorkspaceProjection> {
    Arc::new(WorkspaceProjection::new(
        Arc::new(PgWorkspaceDao(db.clone())),
        Arc::new(PgRepositoryDao(db.clone())),
        plugins(db),
    ))
}

/// The `repositories` store narrowed to what a peer band reads.
pub fn repository_reader(db: DbPool) -> Arc<dyn RepositoryReader> {
    Arc::new(RepositoryProjection::new(Arc::new(PgRepositoryDao(db))))
}

/// Reconciles a repository's git-declared workspaces.
pub fn git_sync(db: DbPool, git_host_tokens: Arc<dyn GitHostTokenIssuer>) -> Arc<GitSyncService> {
    Arc::new(GitSyncService::new(
        db.clone(),
        Arc::new(PgWorkspaceDao(db.clone())),
        Arc::new(PgRepositoryDao(db)),
        git_host_tokens,
    ))
}

/// Lists the branches of a repository the caller may read.
pub fn repository_branches(
    db: DbPool,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    credentials: Arc<CredentialService>,
    http: Arc<reqwest::Client>,
) -> Arc<RepositoryBranchService> {
    Arc::new(RepositoryBranchService::new(
        Arc::new(PgRepositoryDao(db.clone())),
        Arc::new(PgWorkspaceDao(db)),
        git_host_tokens,
        credentials,
        http,
    ))
}

/// Provider credentials pinned to a Workspace.
///
/// `cipher` is `None` without an encryption key, forcing every path to 503
/// instead of an unencrypted write.
pub fn workspace_credentials(
    db: DbPool,
    audits: Arc<SecretAudit>,
    cipher: Option<Arc<dyn SecretCipher>>,
) -> Arc<WorkspaceCredentialService> {
    Arc::new(WorkspaceCredentialService::new(
        Arc::new(PgWorkspaceCredentialDao(db.clone())),
        Arc::new(PgWorkspaceDao(db)),
        audits,
        cipher,
    ))
}

/// Syncs plugin content into stored skills, and reads them back.
pub fn plugin_sync(
    db: DbPool,
    fetcher: Arc<dyn SkillFetcher>,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
) -> Arc<PluginSyncService> {
    Arc::new(PluginSyncService::new(
        Arc::new(PgPluginDao(db)),
        fetcher,
        git_host_tokens,
    ))
}

/// Skill discovery across a workspace and the plugins its activations name.
pub fn skills(
    db: DbPool,
    git_host_tokens: Arc<dyn GitHostTokenIssuer>,
    fetcher: Arc<dyn SkillFetcher>,
    plugin_sync: Arc<PluginSyncService>,
    ttl: std::time::Duration,
) -> Arc<SkillService> {
    Arc::new(SkillService::new(
        Arc::new(PgRepositoryDao(db.clone())),
        Arc::new(PgWorkspaceDao(db.clone())),
        plugins(db),
        git_host_tokens,
        fetcher,
        plugin_sync,
        ttl,
    ))
}

/// Seed the built-in repositories a fresh deployment starts with.
pub async fn seed_default_repositories(db: DbPool) {
    crate::repository::seed::seed_default_repositories(&PgRepositoryDao(db)).await;
}

/// Write one repository row, for a consumer's PostgreSQL tests.
#[cfg(any(test, feature = "test-support"))]
pub async fn seed_repository(
    db: &DbPool,
    row: &crate::repository::models::Repository,
) -> Result<(), sqlx::Error> {
    use crate::repository::dao::RepositoryDao;

    PgRepositoryDao(db.clone()).insert(row).await
}

/// Read one repository row under a caller's scope, for a consumer's PostgreSQL
/// tests.
#[cfg(any(test, feature = "test-support"))]
pub async fn find_repository(
    db: &DbPool,
    id: &str,
    ctx: &moatless_types::access_control::ScopeContext,
) -> Result<Option<crate::repository::models::Repository>, sqlx::Error> {
    use crate::repository::dao::RepositoryDao;

    PgRepositoryDao(db.clone()).find_by_id(id, ctx).await
}
