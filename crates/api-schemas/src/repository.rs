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
#[serde(rename_all = "camelCase")]
pub struct ResourceConfig {
    pub requests: Option<HashMap<String, String>>,
    pub limits: Option<HashMap<String, String>>,
    /// Per-repository workspace storage size override (e.g. "8Gi"). Sizes both
    /// tiers: the per-task PVC (`ebs`) and the node-local emptyDir (`ephemeral`).
    /// When unset, the global `K8S_SANDBOX_STORAGE_SIZE` default applies. Never
    /// copied into container resource specs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
    /// Per-repository storage tier. One of [`STORAGE_MODES`]:
    /// - `"ebs"` (default) — a per-task EBS PVC mounted at `/opt/moatless`,
    ///   surviving pod restarts.
    /// - `"ephemeral"` — a node-local emptyDir, no PVC; transcripts persist only
    ///   via the S3 sync, the workspace does not survive the pod.
    /// - `"s3"` — a node-local emptyDir plus a direct git-working-tree sync to S3,
    ///   giving workspace persistence without an EBS attach/detach per restart.
    ///
    /// When unset, the deployment-wide default applies
    /// (`K8S_SANDBOX_DEFAULT_STORAGE_MODE`, itself defaulting to `"ebs"`).
    /// Unknown values are ignored (treated as unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mode: Option<String>,
}

/// Accepted values for [`ResourceConfig::storage_mode`]. `"ebs"` is the built-in
/// fallback; the effective default for repos that leave `storage_mode` unset is
/// set per-deployment via `K8S_SANDBOX_DEFAULT_STORAGE_MODE`.
pub const STORAGE_MODES: [&str; 3] = ["ebs", "ephemeral", "s3"];

/// Whether `mode` is a recognized [`ResourceConfig::storage_mode`] value.
///
/// Matching is case-insensitive and ignores surrounding whitespace, mirroring
/// the backend's `StorageBackend::from_plan` resolution.
#[must_use]
pub fn is_valid_storage_mode(mode: &str) -> bool {
    let normalized = mode.trim().to_ascii_lowercase();
    STORAGE_MODES.contains(&normalized.as_str())
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
    /// Opt-in Docker-in-Docker access (privileged `dind` sidecar) — enables
    /// Testcontainers inside the sandbox. Defaults to false.
    #[serde(default)]
    pub docker_access: Option<bool>,
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
    /// Opt-in Docker-in-Docker access (privileged `dind` sidecar) — enables
    /// Testcontainers inside the sandbox. Defaults to false.
    #[serde(default)]
    pub docker_access: Option<bool>,
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
    #[serde(default)]
    pub docker_access: Option<bool>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_config_serializes_storage_mode_as_camel_case() {
        let cfg = ResourceConfig {
            storage_mode: Some("ephemeral".to_string()),
            ..Default::default()
        };
        let value = serde_json::to_value(&cfg).expect("serialize ResourceConfig");
        assert_eq!(value.get("storageMode"), Some(&serde_json::json!("ephemeral")));
        assert!(
            value.get("storage_mode").is_none(),
            "snake_case key must not be emitted"
        );
    }

    #[test]
    fn resource_config_deserializes_camel_case_storage_mode() {
        let cfg: ResourceConfig =
            serde_json::from_value(serde_json::json!({ "storageMode": "ephemeral" }))
                .expect("deserialize ResourceConfig");
        assert_eq!(cfg.storage_mode.as_deref(), Some("ephemeral"));
    }

    #[test]
    fn storage_mode_validation_accepts_known_and_normalizes() {
        assert!(is_valid_storage_mode("ebs"));
        assert!(is_valid_storage_mode("ephemeral"));
        assert!(is_valid_storage_mode("s3"));
        // Case-insensitive + whitespace-tolerant, matching the backend resolver.
        assert!(is_valid_storage_mode("  S3 "));
        assert!(is_valid_storage_mode("EBS"));
        // Unknown values are rejected so the CLI can catch typos early.
        assert!(!is_valid_storage_mode("s33"));
        assert!(!is_valid_storage_mode("pvc"));
        assert!(!is_valid_storage_mode(""));
    }
}
