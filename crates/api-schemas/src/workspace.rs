//! Workspace request/response types.
//!
//! A Workspace owns the run-configuration and holds 0..n repository placements
//! (`WorkspaceRepo`). Run-config fields mirror the repository DTOs and reuse the
//! same [`ServerConfig`]/[`ResourceConfig`] shapes.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::enums::Scope;
use crate::repository::{ResourceConfig, ServerConfig};

// =============================================================================
// WorkspaceRepoInput — a repository placement in create/add requests
// =============================================================================

/// A repository placement inside a workspace (create/add payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRepoInput {
    pub repository_id: String,
    /// Override the repository's default branch for this placement.
    #[serde(default)]
    pub branch: Option<String>,
    /// Subdir under the workspace dir (defaults to the repo name at runtime).
    #[serde(default)]
    pub mount_name: Option<String>,
    /// Mark this placement as the primary (diff/PR/CLAUDE.md) repo. At most one
    /// primary is kept per workspace; when several are flagged the first wins.
    #[serde(default)]
    pub is_primary: Option<bool>,
    #[serde(default)]
    pub position: Option<i32>,
}

// =============================================================================
// CreateWorkspaceRequest
// =============================================================================

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub scope: Option<Scope>,
    // --- Run configuration (mirrors repositories) ---
    #[serde(default)]
    pub docker_image: Option<String>,
    #[serde(default)]
    pub docker_access: Option<bool>,
    #[serde(default)]
    pub sandbox_service_account: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub setup_commands: Option<Vec<String>>,
    #[serde(default)]
    pub env_vars: Option<HashMap<String, String>>,
    #[serde(default)]
    pub servers: Option<Vec<ServerConfig>>,
    #[serde(default)]
    pub resources: Option<ResourceConfig>,
    #[serde(default)]
    pub network_profile: Option<String>,
    // --- Composition ---
    /// Repository placements. May be empty (a blank/scratch workspace).
    #[serde(default)]
    pub repos: Vec<WorkspaceRepoInput>,
}

// =============================================================================
// UpdateWorkspaceRequest
// =============================================================================

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkspaceRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    /// Change the workspace's access scope (`global` or `user`). When set,
    /// `owner_user_id` is re-resolved from the new scope. Requesting `global`
    /// requires admin.
    pub scope: Option<Scope>,
    pub docker_image: Option<String>,
    pub docker_access: Option<bool>,
    pub sandbox_service_account: Option<String>,
    pub system_prompt: Option<String>,
    pub setup_commands: Option<Vec<String>>,
    pub env_vars: Option<HashMap<String, String>>,
    pub servers: Option<Vec<ServerConfig>>,
    pub resources: Option<ResourceConfig>,
    pub network_profile: Option<String>,
}

// =============================================================================
// AddRepoRequest
// =============================================================================

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AddRepoRequest {
    pub repository_id: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub mount_name: Option<String>,
    #[serde(default)]
    pub is_primary: Option<bool>,
    #[serde(default)]
    pub position: Option<i32>,
}

// =============================================================================
// WorkspaceRepoResponse
// =============================================================================

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRepoResponse {
    pub id: String,
    pub workspace_id: String,
    pub repository_id: String,
    pub branch: Option<String>,
    pub mount_name: Option<String>,
    pub is_primary: bool,
    pub position: i32,
    pub created_at: Option<String>,
}

// =============================================================================
// WorkspaceResponse
// =============================================================================

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Derived from the placement count: `blank` (0), `single` (1), `multi` (n).
    pub kind: String,
    pub docker_image: Option<String>,
    pub docker_access: Option<bool>,
    pub sandbox_service_account: Option<String>,
    pub system_prompt: Option<String>,
    pub setup_commands: Option<Vec<String>>,
    pub env_vars: Option<HashMap<String, String>>,
    pub servers: Option<Vec<ServerConfig>>,
    pub resources: Option<ResourceConfig>,
    #[serde(default)]
    pub network_profile: Option<String>,
    /// `manual` or `git`. Git-synced workspaces are read-only in the UI and
    /// write API — only a re-sync from git changes them.
    #[serde(default)]
    pub source: String,
    /// Convenience flag derived from `source == "git"`.
    #[serde(default)]
    pub synced_from_git: bool,
    /// Path to the config file within the source repository that declared this
    /// workspace (e.g. `.moatless/workspaces.json` or `.moatless/config.json`).
    /// Retained even after a manual override so the origin remains visible.
    #[serde(default)]
    pub source_config_path: Option<String>,
    /// The repository whose git tree declared this workspace (`source == "git"`).
    #[serde(default)]
    pub source_repository_id: Option<String>,
    /// The entry's stable key within the declaring file (its `key`, else the
    /// slugified name). Retained after an override, like the other provenance.
    #[serde(default)]
    pub source_key: Option<String>,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub created_by: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub repos: Vec<WorkspaceRepoResponse>,
}

// =============================================================================
// Git sync summaries
// =============================================================================

/// One git-declared workspace that was present but not synced, with a reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSyncSkip {
    /// The entry's stable key within the declaring file.
    pub source_key: String,
    /// Why the workspace was skipped.
    pub reason: String,
}

/// Summary of reconciling a repo's git-declared workspaces into `workspaces`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSyncSummary {
    /// Workspaces inserted as new rows.
    pub created: u32,
    /// Existing git-synced workspaces updated in place.
    pub updated: u32,
    /// Manually overridden workspaces taken back under git control by an
    /// explicit sync, discarding the local edits.
    pub reclaimed: u32,
    /// Workspaces soft-deleted because git no longer declares them.
    pub removed: u32,
    /// Workspaces that were declared but not synced, with reasons.
    pub skipped: Vec<WorkspaceSyncSkip>,
}

/// Response for `POST /api/v1/workspaces/{id}/reset-git`.
///
/// The workspace only returns to `source = "git"` when the re-sync actually
/// found it still declared in the repository, so `resynced` reports whether the
/// reset took effect.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ResetGitWorkspaceResponse {
    pub workspace: WorkspaceResponse,
    /// Whether the workspace was reclaimed from git during this call.
    pub resynced: bool,
    /// Why the re-sync did not take effect (unreachable provider, or the entry
    /// is no longer declared in the config file). `None` when `resynced`.
    pub error: Option<String>,
}
