use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;
use moatless_types::store::StoreError;
use moatless_workspace_types::plugin::PluginDelivery;
use moatless_workspace_types::repository::Repository;
use moatless_workspace_types::workspace::ResourceConfig;
use moatless_workspace_types::workspace::Workspace;
use std::collections::HashMap;
use uuid::Uuid;

/// A Workspace's run configuration, plus the identity it runs as.
///
/// Exactly the columns a Sandbox plan and an agent prompt read; provenance,
/// access-control, and timestamp columns stay behind the seam.
#[derive(Debug, Clone)]
pub struct ResolvedWorkspace {
    pub id: String,
    pub name: String,
    pub docker_image: Option<String>,
    pub docker_access: Option<bool>,
    pub sandbox_service_account: Option<String>,
    pub system_prompt: Option<String>,
    pub setup_commands: Option<Vec<String>>,
    pub env_vars: Option<HashMap<String, String>>,
    pub resources: Option<ResourceConfig>,
    pub network_profile: Option<String>,
    /// Extra node labels sandbox pods must match, merged over the
    /// deployment-wide selector.
    pub node_selector: Option<std::collections::BTreeMap<String, String>>,
    /// The identity every Task on this Workspace acts as on a git host. When
    /// set it outranks the Task's own owner.
    pub run_as_user_id: Option<String>,
    pub deleted: bool,
}

/// One repository placed in a Workspace, with the repository it points at
/// already resolved.
///
/// Ordered primary-first, then by `position`. `repository` is `None` when the
/// placement points at a row that no longer exists — a configuration problem
/// the caller reports, not a read failure.
#[derive(Debug, Clone)]
pub struct RepoPlacement {
    pub repository_id: String,
    /// Overrides the repository's default branch for this placement.
    pub branch: Option<String>,
    /// Subdirectory under the workspace directory.
    pub mount_name: Option<String>,
    pub is_primary: bool,
    pub position: i32,
    pub repository: Option<Repository>,
}

/// A Workspace's run configuration, its repository placements, and the plugins
/// an identity has turned on.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait WorkspaceResolver: Send + Sync {
    /// The Workspace's run configuration, or `None` when no such Workspace
    /// exists. Resolves across owners: the caller's authorization decided the
    /// Workspace may be run before it got here.
    async fn resolve(&self, workspace_id: &str) -> Result<Option<ResolvedWorkspace>, StoreError>;

    /// Every repository the Workspace places, primary first.
    async fn placements(&self, workspace_id: &str) -> Result<Vec<RepoPlacement>, StoreError>;

    /// The effective plugins for an identity working in a Workspace, each with
    /// the skills to keep. `user_id: None` is nobody in particular and yields
    /// the global defaults; the Workspace's own records outrank both.
    async fn plugin_deliveries(
        &self,
        user_id: Option<Uuid>,
        workspace_id: &str,
    ) -> Result<Vec<PluginDelivery>, StoreError>;

    /// Display name per Workspace id. Ids with no Workspace are absent from the
    /// map. Resolves across owners, like the rest of this port.
    async fn names(&self, workspace_ids: &[String]) -> Result<HashMap<String, String>, StoreError>;

    /// The Workspace `ctx` may see, or `None` when there is none to show.
    ///
    /// Scoped, unlike the rest of this port: a Workspace the caller may not see
    /// reads exactly as one that does not exist.
    async fn describe(
        &self,
        ctx: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<Workspace>, StoreError>;

    /// The live Workspaces placing this repository, primary placement first
    /// then oldest first. Deleted Workspaces are absent, and the result is not
    /// scope-filtered.
    ///
    /// The first element is what a Loop declaring none resolves to at sync, so
    /// the order is the contract, not a convenience.
    async fn for_repository(&self, repository_id: &str) -> Result<Vec<Workspace>, StoreError>;
}
