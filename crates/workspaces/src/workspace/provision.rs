//! Find or create the Workspace a GitHub repository's triggered Task runs in,
//! registering the repository when Moatless has never seen it.

use tracing::info;

use moatless_core::db::DbPool;
use moatless_types::access_control::{Scope, ScopeContext};
use moatless_workspace_types::repository::{RepositoryKind, RepositoryProvider};

use super::operations::Workspaces;
use crate::repository::error::RepositoryError;
use crate::repository::operations::Repositories;
use crate::repository::remote::normalize_remote_url;
use crate::repository::schemas::CreateRepositoryRequest;
use crate::workspace::git_sync::GitSyncService;

/// One GitHub repository a triggered Task needs a Workspace for.
#[derive(Debug, Clone)]
pub struct ProvisionRequest {
    pub owner: String,
    pub repo: String,
    /// From the webhook's `repository` object; stored on a new repository.
    pub default_branch: Option<String>,
    pub description: Option<String>,
    /// `provision.workspace_template`. Set, it lets an unknown repository be
    /// registered, which gives it its default Workspace; `None` finds an
    /// existing Workspace but never registers one. Its value is not read.
    pub template: Option<String>,
    /// Chosen over the repository's other Workspaces when it holds the
    /// repository, so a loop's named Workspace wins for its own repository.
    pub preferred_workspace_id: Option<String>,
}

/// Where a triggered Task on the repository runs, or why it has nowhere to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provisioned {
    Workspace(String),
    /// The repository was soft-deleted: an administrator removed it, so it is
    /// not registered again behind their back.
    RepositoryDeleted,
    /// No Workspace holds the repository and the request named no template.
    NoTemplate,
}

#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    /// The git sync registering the repository ran gave it no Workspace, as
    /// when its host could not be read.
    #[error("repository `{0}` has no workspace after its git sync")]
    NoWorkspace(String),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// The stores [`provision_workspace`] reads and writes.
pub struct ProvisionStores<'a> {
    pub workspaces: &'a Workspaces,
    pub repositories: &'a Repositories,
    pub git_sync: &'a GitSyncService,
    pub pool: &'a DbPool,
}

/// Find or register the global repository for `request`, then its preferred
/// or first Workspace.
///
/// Serialized per repository URL by a Postgres advisory lock, so two triggers
/// for one new repository create one repository and one Workspace.
/// `ctx` must be an administrator: everything created is global.
pub async fn provision_workspace(
    stores: &ProvisionStores<'_>,
    ctx: &ScopeContext,
    request: &ProvisionRequest,
) -> Result<Provisioned, ProvisionError> {
    let remote_url = format!("https://github.com/{}/{}", request.owner, request.repo);
    let url_key = normalize_remote_url(&remote_url);

    let lock = super::dao::lock_workspace_provision(stores.pool, &url_key).await?;
    let outcome = provision_locked(stores, ctx, request, &remote_url, &url_key).await;
    let unlocked = super::dao::unlock_workspace_provision(lock, &url_key).await;
    let provisioned = outcome?;
    unlocked?;
    Ok(provisioned)
}

async fn provision_locked(
    stores: &ProvisionStores<'_>,
    ctx: &ScopeContext,
    request: &ProvisionRequest,
    remote_url: &str,
    url_key: &str,
) -> Result<Provisioned, ProvisionError> {
    let repositories = stores.repositories.store();
    let known = repositories
        .find_by_normalized_remote_url(url_key, Scope::Global, None)
        .await?;
    let repository = if let Some(repository) = known {
        repository
    } else {
        if repositories
            .exists_by_normalized_remote_url_including_deleted(url_key, Scope::Global, None)
            .await?
        {
            return Ok(Provisioned::RepositoryDeleted);
        }
        // Checked before registering, so a loop with no template leaves no
        // repository behind for an administrator to clean up.
        if request.template.is_none() {
            return Ok(Provisioned::NoTemplate);
        }
        let repository = crate::repository::mutations::create_repository(
            repositories,
            stores.git_sync,
            ctx,
            CreateRepositoryRequest {
                name: request.repo.clone(),
                kind: RepositoryKind::Remote,
                remote_url: Some(remote_url.to_string()),
                default_branch: request.default_branch.clone(),
                description: request.description.clone(),
                provider: Some(RepositoryProvider::Github),
                scope: Some(Scope::Global),
            },
        )
        .await?;
        info!(
            repository_id = %repository.id,
            remote_url,
            "Registered a repository for a check-run trigger"
        );
        repository
    };

    // The git sync `create_repository` ran declared at least the default one;
    // a repository registered before every repository had one syncs it here.
    let mut holding = stores
        .workspaces
        .store()
        .find_by_repository(&repository.id)
        .await?;
    if holding.is_empty() {
        stores
            .git_sync
            .sync_repo_workspaces_for(&repository, ctx)
            .await?;
        holding = stores
            .workspaces
            .store()
            .find_by_repository(&repository.id)
            .await?;
    }
    let preferred = holding
        .iter()
        .find(|w| Some(&w.id) == request.preferred_workspace_id.as_ref());
    preferred
        .or(holding.first())
        .map(|workspace| Provisioned::Workspace(workspace.id.clone()))
        .ok_or_else(|| ProvisionError::NoWorkspace(repository.id.clone()))
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "provision_db_tests.rs"]
mod db_tests;
