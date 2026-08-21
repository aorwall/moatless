//! The seams the workspace domain publishes to peer crates.
//!
//! A Workspace is the run-configuration source for every Sandbox and every
//! agent prompt. Both consumers read a fixed projection of the row and the
//! repositories it places, so that projection is what crosses the seam —
//! not the `workspaces` store. `moatless-workspaces` implements both ports.

use std::collections::HashMap;

use async_trait::async_trait;
use uuid::Uuid;

pub use moatless_api_schemas::plugin::PluginDelivery;
pub use moatless_types::repository::Repository;
use moatless_types::workspace::{ResourceConfig, ServerConfig};

use crate::store::StoreError;

/// A Workspace's run configuration, plus the identity it runs as.
///
/// Exactly the columns a Sandbox plan and an agent prompt read. Provenance,
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
    pub servers: Option<Vec<ServerConfig>>,
    pub resources: Option<ResourceConfig>,
    pub network_profile: Option<String>,
    /// The identity every Task on this Workspace acts as on a git host. When
    /// set it outranks the Task's own owner.
    pub run_as_user_id: Option<String>,
    pub deleted: bool,
}

/// One repository placed in a Workspace, with the repository it points at
/// already resolved.
///
/// Ordered by the Workspace's own precedence: the primary placement first,
/// then by `position`. `repository` is `None` when the placement points at a
/// row that no longer exists — a configuration problem the caller reports,
/// not a read failure.
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

    /// The effective plugins for an identity, each with the skills to keep.
    /// `None` is nobody in particular and yields the global defaults.
    async fn plugin_deliveries(
        &self,
        user_id: Option<Uuid>,
    ) -> Result<Vec<PluginDelivery>, StoreError>;
}

/// The provider credentials pinned to a Workspace, decrypted and keyed by
/// environment variable.
///
/// Takes no actor: by the time these are read the caller's own authorization
/// already decided the Workspace may be run. A Workspace that holds credentials
/// whose values cannot be read is an error — falling back would run under a
/// different identity than the Workspace was configured with.
#[cfg_attr(feature = "test-support", mockall::automock)]
#[async_trait]
pub trait WorkspaceCredentialReader: Send + Sync {
    async fn resolve_env(&self, workspace_id: &str)
    -> Result<HashMap<String, String>, StoreError>;
}
