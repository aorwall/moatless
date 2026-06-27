//! Repository request/response types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::enums::{RepositoryKind, RepositoryProvider, Scope};

// =============================================================================
// ResourceConfig
// =============================================================================

/// Per-repository K8s resource requests/limits configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ResourceConfig {
    pub requests: Option<HashMap<String, String>>,
    pub limits: Option<HashMap<String, String>>,
    /// Per-repository workspace storage size override (e.g. "8Gi"). Sizes both
    /// tiers: the per-task PVC (`ebs`) and the node-local emptyDir (`ephemeral`).
    /// When unset, the global `K8S_SANDBOX_STORAGE_SIZE` default applies. Never
    /// copied into container resource specs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
    /// Per-repository storage tier: `"ebs"` (default — a per-task EBS PVC mounted
    /// at `/opt/moatless`, surviving pod restarts) or `"ephemeral"` (a node-local
    /// emptyDir, no PVC; transcripts persist only via the S3 sync). Unset / unknown
    /// values are treated as `"ebs"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mode: Option<String>,
}

// =============================================================================
// ServerConfig
// =============================================================================

/// Server config entry.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub start_commands: Option<Vec<String>>,
    #[serde(default)]
    pub startup_timeout_seconds: Option<u32>,
    #[serde(default)]
    pub auto_reload: bool,
    #[serde(default)]
    pub enable_inspector: bool,
    #[serde(default)]
    pub default: bool,
    /// Per-server Docker image. Falls back to sandbox_image if not set.
    #[serde(default)]
    pub docker_image: Option<String>,
    /// Per-server environment variables.
    #[serde(default)]
    pub env_vars: Option<HashMap<String, String>>,
}

// =============================================================================
// CreateRepositoryRequest
// =============================================================================

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateRepositoryRequest {
    pub name: String,
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub docker_image: Option<String>,
    pub system_prompt: Option<String>,
    pub description: Option<String>,
    pub provider: Option<RepositoryProvider>,
    pub scope: Option<Scope>,
    pub team_id: Option<String>,
    pub servers: Option<Vec<ServerConfig>>,
    pub plugins: Option<Vec<String>>,
    pub env_vars: Option<HashMap<String, String>>,
    pub setup_commands: Option<Vec<String>>,
    pub server_start_commands: Option<Vec<String>>,
    pub startup_timeout_seconds: Option<i32>,
    pub build_command: Option<String>,
    pub build_output_directory: Option<String>,
    pub enable_inspector: Option<bool>,
    pub auto_reload: Option<bool>,
    pub resources: Option<ResourceConfig>,
}

// =============================================================================
// UpdateRepositoryRequest
// =============================================================================

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateRepositoryRequest {
    pub name: Option<String>,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub docker_image: Option<String>,
    pub system_prompt: Option<String>,
    pub description: Option<String>,
    pub servers: Option<Vec<ServerConfig>>,
    pub plugins: Option<Vec<String>>,
    pub env_vars: Option<HashMap<String, String>>,
    pub setup_commands: Option<Vec<String>>,
    pub server_start_commands: Option<Vec<String>>,
    pub startup_timeout_seconds: Option<i32>,
    pub build_command: Option<String>,
    pub build_output_directory: Option<String>,
    pub enable_inspector: Option<bool>,
    pub auto_reload: Option<bool>,
    pub resources: Option<ResourceConfig>,
}

// =============================================================================
// RepositoryResponse
// =============================================================================

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RepositoryResponse {
    pub id: String,
    pub name: String,
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub docker_image: Option<String>,
    pub system_prompt: Option<String>,
    pub description: Option<String>,
    pub provider: Option<RepositoryProvider>,
    pub scope: Scope,
    pub team_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub servers: Option<Vec<ServerConfig>>,
    pub plugins: Option<Vec<String>>,
    pub env_vars: Option<HashMap<String, String>>,
    pub setup_commands: Option<Vec<String>>,
    pub server_start_commands: Option<Vec<String>>,
    pub startup_timeout_seconds: Option<i32>,
    pub build_command: Option<String>,
    pub build_output_directory: Option<String>,
    pub enable_inspector: Option<bool>,
    pub auto_reload: Option<bool>,
    pub resources: Option<ResourceConfig>,
    pub created_by: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

// =============================================================================
// BranchListResponse / BranchEntry
// =============================================================================

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct BranchListResponse {
    pub branches: Vec<BranchEntry>,
    pub default_branch: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct BranchEntry {
    pub name: String,
    pub is_default: bool,
}
