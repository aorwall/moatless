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
    /// Per-repository workspace storage size override (e.g. "8Gi"). Sizes the
    /// per-task PVC (`ebs`) and node-local emptyDir (`ephemeral`/`s3`) tiers.
    /// The `node` hostPath tier is bounded by node disk capacity instead. When
    /// unset, the global `K8S_SANDBOX_STORAGE_SIZE` default applies. Never copied
    /// into container resource specs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
    /// Per-repository storage tier. One of [`STORAGE_MODES`]:
    /// - `"ebs"` (default) — a per-task EBS PVC mounted at `/opt/moatless`,
    ///   surviving pod restarts.
    /// - `"ephemeral"` — a node-local emptyDir, no PVC; transcripts persist only
    ///   via the S3 sync, the workspace does not survive the pod.
    /// - `"s3"` — a node-local emptyDir plus a direct git-working-tree sync to S3,
    ///   giving workspace persistence without an EBS attach/detach per restart.
    /// - `"node"` — a best-effort node-local hostPath. The workspace survives
    ///   stop/start only when Kubernetes schedules the task back onto the same node.
    ///
    /// When unset, the deployment-wide default applies
    /// (`K8S_SANDBOX_DEFAULT_STORAGE_MODE`, itself defaulting to `"ebs"`).
    /// Unknown values are ignored (treated as unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mode: Option<String>,
    /// Per-repository size cap for the `dind` sidecar's `/var/lib/docker`
    /// emptyDir (e.g. "50Gi"). Only meaningful for repos with `dockerAccess`
    /// and `dockerStorageMode: "emptyDir"`. Exceeding the cap gets the pod
    /// **evicted** by the kubelet, wiping all dind container data. When unset,
    /// the global `K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE` default applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_storage: Option<String>,
    /// Storage backend for the `dind` sidecar's `/var/lib/docker`:
    /// - `"node"` (default) — best-effort node-local hostPath. Docker layers,
    ///   images, and volumes survive stop/start only on the same node.
    /// - `"emptyDir"` — node-local emptyDir capped by `dockerStorage`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_storage_mode: Option<String>,
}

/// Accepted values for [`ResourceConfig::storage_mode`]. `"ebs"` is the built-in
/// fallback; the effective default for repos that leave `storage_mode` unset is
/// set per-deployment via `K8S_SANDBOX_DEFAULT_STORAGE_MODE`.
pub const STORAGE_MODES: [&str; 4] = ["ebs", "ephemeral", "s3", "node"];
pub const DOCKER_STORAGE_MODES: [&str; 2] = ["emptyDir", "node"];

/// Whether `mode` is a recognized [`ResourceConfig::storage_mode`] value.
///
/// Matching is case-insensitive and ignores surrounding whitespace, mirroring
/// the backend's `StorageBackend::from_plan` resolution.
#[must_use]
pub fn is_valid_storage_mode(mode: &str) -> bool {
    let normalized = mode.trim().to_ascii_lowercase();
    STORAGE_MODES.contains(&normalized.as_str())
}

#[must_use]
pub fn is_valid_docker_storage_mode(mode: &str) -> bool {
    matches!(
        mode.trim().to_ascii_lowercase().as_str(),
        "emptydir" | "node"
    )
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

/// A repository is a connection to a git repo. Run-configuration lives on the
/// workspace, not here.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct CreateRepositoryRequest {
    pub name: String,
    pub kind: RepositoryKind,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub description: Option<String>,
    pub provider: Option<RepositoryProvider>,
    pub scope: Option<Scope>,
}

// =============================================================================
// UpdateRepositoryRequest
// =============================================================================

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateRepositoryRequest {
    pub name: Option<String>,
    /// Change the repository's access scope (`global` or `user`).
    /// Re-submitting `user` preserves the repository's existing immutable owner.
    /// An ownerless repository cannot move to `user`.
    /// Requesting `global` requires admin.
    pub scope: Option<Scope>,
    pub remote_url: Option<String>,
    pub default_branch: Option<String>,
    pub description: Option<String>,
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
    pub description: Option<String>,
    pub provider: Option<RepositoryProvider>,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    /// Provider reachability of `remote_url`: `unchecked` | `ok` | `not_found`
    /// | `forbidden` | `error`. Stamped when the repo is auto-created by a
    /// git-synced workspace.
    #[serde(default)]
    pub access_status: String,
    #[serde(default)]
    pub access_checked_at: Option<String>,
    #[serde(default)]
    pub access_error: Option<String>,
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
        assert_eq!(
            value.get("storageMode"),
            Some(&serde_json::json!("ephemeral"))
        );
        assert!(
            value.get("storage_mode").is_none(),
            "snake_case key must not be emitted"
        );
    }

    #[test]
    fn resource_config_serializes_docker_storage_as_camel_case() {
        let cfg = ResourceConfig {
            docker_storage: Some("50Gi".to_string()),
            docker_storage_mode: Some("node".to_string()),
            ..Default::default()
        };
        let value = serde_json::to_value(&cfg).expect("serialize ResourceConfig");
        assert_eq!(value.get("dockerStorage"), Some(&serde_json::json!("50Gi")));
        assert_eq!(
            value.get("dockerStorageMode"),
            Some(&serde_json::json!("node"))
        );
        assert!(
            value.get("docker_storage").is_none(),
            "snake_case key must not be emitted"
        );
        assert!(
            value.get("docker_storage_mode").is_none(),
            "snake_case key must not be emitted"
        );
    }

    #[test]
    fn resource_config_deserializes_camel_case_docker_storage() {
        let cfg: ResourceConfig =
            serde_json::from_value(serde_json::json!({ "dockerStorage": "50Gi" }))
                .expect("deserialize ResourceConfig");
        assert_eq!(cfg.docker_storage.as_deref(), Some("50Gi"));
    }

    #[test]
    fn resource_config_deserializes_camel_case_docker_storage_mode() {
        let cfg: ResourceConfig =
            serde_json::from_value(serde_json::json!({ "dockerStorageMode": "emptyDir" }))
                .expect("deserialize ResourceConfig");
        assert_eq!(cfg.docker_storage_mode.as_deref(), Some("emptyDir"));
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
        assert!(is_valid_storage_mode("node"));
        // Case-insensitive + whitespace-tolerant, matching the backend resolver.
        assert!(is_valid_storage_mode("  S3 "));
        assert!(is_valid_storage_mode("EBS"));
        // Unknown values are rejected so the CLI can catch typos early.
        assert!(!is_valid_storage_mode("s33"));
        assert!(!is_valid_storage_mode("pvc"));
        assert!(!is_valid_storage_mode(""));
    }

    #[test]
    fn docker_storage_mode_validation_accepts_known_and_normalizes() {
        assert!(is_valid_docker_storage_mode("node"));
        assert!(is_valid_docker_storage_mode("emptyDir"));
        assert!(is_valid_docker_storage_mode("  EMPTYDIR "));
        assert!(!is_valid_docker_storage_mode("s3"));
        assert!(!is_valid_docker_storage_mode(""));
    }
}
