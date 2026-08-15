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
    /// per-task PVC when `pvc_enabled`, and the node-local emptyDir otherwise.
    /// When unset, the global `K8S_SANDBOX_STORAGE_SIZE` default applies. Never
    /// copied into container resource specs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
    /// Per-repository size cap for the transient `build-cache` emptyDir mounted
    /// at `/opt/moatless/build-cache` (e.g. "40Gi"). Use with workspace env vars
    /// such as `CARGO_TARGET_DIR=/opt/moatless/build-cache/target` for large
    /// build artifacts that should not fill the workspace volume. Exceeding the
    /// cap gets the pod **evicted** by the kubelet. When unset, the global
    /// `K8S_SANDBOX_BUILD_CACHE_SIZE` default applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_cache: Option<String>,
    /// Storage backend for the `build-cache` volume mounted at
    /// `/opt/moatless/build-cache`:
    /// - `"emptyDir"` (default) — node-local emptyDir capped by `buildCache`.
    /// - `"node"` — node-local hostPath under `K8S_SANDBOX_CACHE_HOST_ROOT`.
    ///   Kubernetes does not enforce `buildCache` as a size cap for this mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_cache_mode: Option<String>,
    /// Scope for `buildCacheMode: "node"`:
    /// - `"task"` (default) — per-task hostPath, cleaned up with the task.
    /// - `"node"` — shared by all sandboxes scheduled on the same node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_cache_scope: Option<String>,
    /// Whether this repo's sandbox gets a per-task PVC mounted at
    /// `/opt/moatless`. The PVC is owned by the Deployment: it survives pod
    /// restarts and is deleted when the sandbox is removed. When `false`, the
    /// workspace is a node-local emptyDir that dies with the pod.
    ///
    /// When unset, the deployment-wide default applies (`K8S_SANDBOX_DEFAULT_PVC`).
    /// The global `K8S_SANDBOX_STORAGE_ENABLED` kill switch still wins over both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pvc_enabled: Option<bool>,
    /// Whether this repo's sandbox snapshots its workspace to S3 — the copy that
    /// outlives the Deployment. Pushed before a stop or remove, and pulled back
    /// on start when the volume holds nothing usable (a fresh PVC, a redeploy, or
    /// a fallback after the PVC could not attach).
    ///
    /// Independent of [`Self::pvc_enabled`]: with a PVC the volume is the live
    /// copy and S3 is read only when it is empty or broken; without one, S3 is
    /// the only persistence and is read on every start.
    ///
    /// When unset, the deployment-wide default applies (`K8S_SANDBOX_DEFAULT_S3`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_enabled: Option<bool>,
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

pub const BUILD_CACHE_MODES: [&str; 2] = ["emptyDir", "node"];
pub const BUILD_CACHE_SCOPES: [&str; 2] = ["task", "node"];
pub const DOCKER_STORAGE_MODES: [&str; 2] = ["emptyDir", "node"];

#[must_use]
pub fn is_valid_docker_storage_mode(mode: &str) -> bool {
    matches!(
        mode.trim().to_ascii_lowercase().as_str(),
        "emptydir" | "node"
    )
}

#[must_use]
pub fn is_valid_build_cache_mode(mode: &str) -> bool {
    matches!(
        mode.trim().to_ascii_lowercase().as_str(),
        "emptydir" | "node"
    )
}

#[must_use]
pub fn is_valid_build_cache_scope(scope: &str) -> bool {
    matches!(scope.trim().to_ascii_lowercase().as_str(), "task" | "node")
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
    /// Per-server resource requests/limits. Falls back to the repo-wide
    /// `resources` when unset. Only `requests`/`limits` apply; storage/cache
    /// fields are repo-wide and ignored here.
    #[serde(default)]
    pub resources: Option<ResourceConfig>,
}

// =============================================================================
// ScriptConfig
// =============================================================================

/// A named command someone runs on demand inside the sandbox, hosted by a
/// terminal session rather than a supervised container.
///
/// Unlike a [`ServerConfig`] a script is not supervised, has no container of
/// its own and is expected to exit. When it declares a `port`, the port is
/// published at the task's preview hostname as the script starts.
///
/// `name` is the identity — it must survive a git round trip, so no id is
/// allocated by the backend.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ScriptConfig {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Written into the terminal verbatim, followed by a carriage return.
    pub command: String,
    /// One of `play`, `test`, `lint`, `configure`, `build`, `debug`. Anything
    /// else is reported to clients as `play`.
    #[serde(default)]
    pub icon: Option<String>,
    /// The port the script serves on, if any.
    #[serde(default)]
    pub port: Option<u16>,
    /// Extra environment for the script's terminal session.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct BranchListResponse {
    pub branches: Vec<BranchEntry>,
    pub default_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    fn resource_config_serializes_storage_switches_as_camel_case() {
        let cfg = ResourceConfig {
            pvc_enabled: Some(true),
            s3_enabled: Some(true),
            ..Default::default()
        };
        let value = serde_json::to_value(&cfg).expect("serialize ResourceConfig");
        assert_eq!(value.get("pvcEnabled"), Some(&serde_json::json!(true)));
        assert_eq!(value.get("s3Enabled"), Some(&serde_json::json!(true)));
        assert!(
            value.get("pvc_enabled").is_none() && value.get("s3_enabled").is_none(),
            "snake_case keys must not be emitted"
        );
    }

    #[test]
    fn resource_config_omits_unset_storage_switches() {
        let value = serde_json::to_value(ResourceConfig::default()).expect("serialize");
        assert!(
            value.get("pvcEnabled").is_none() && value.get("s3Enabled").is_none(),
            "unset switches must be omitted so they fall back to the deployment default"
        );
    }

    #[test]
    fn resource_config_serializes_docker_storage_as_camel_case() {
        let cfg = ResourceConfig {
            build_cache: Some("40Gi".to_string()),
            build_cache_mode: Some("node".to_string()),
            build_cache_scope: Some("task".to_string()),
            docker_storage: Some("50Gi".to_string()),
            docker_storage_mode: Some("node".to_string()),
            ..Default::default()
        };
        let value = serde_json::to_value(&cfg).expect("serialize ResourceConfig");
        assert_eq!(value.get("buildCache"), Some(&serde_json::json!("40Gi")));
        assert_eq!(
            value.get("buildCacheMode"),
            Some(&serde_json::json!("node"))
        );
        assert_eq!(
            value.get("buildCacheScope"),
            Some(&serde_json::json!("task"))
        );
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
        assert!(
            value.get("build_cache").is_none(),
            "snake_case key must not be emitted"
        );
        assert!(
            value.get("build_cache_mode").is_none(),
            "snake_case key must not be emitted"
        );
        assert!(
            value.get("build_cache_scope").is_none(),
            "snake_case key must not be emitted"
        );
    }

    #[test]
    fn resource_config_deserializes_camel_case_build_cache() {
        let cfg: ResourceConfig =
            serde_json::from_value(serde_json::json!({ "buildCache": "40Gi" }))
                .expect("deserialize ResourceConfig");
        assert_eq!(cfg.build_cache.as_deref(), Some("40Gi"));
    }

    #[test]
    fn resource_config_deserializes_camel_case_build_cache_mode_and_scope() {
        let cfg: ResourceConfig = serde_json::from_value(serde_json::json!({
            "buildCacheMode": "node",
            "buildCacheScope": "task"
        }))
        .expect("deserialize ResourceConfig");
        assert_eq!(cfg.build_cache_mode.as_deref(), Some("node"));
        assert_eq!(cfg.build_cache_scope.as_deref(), Some("task"));
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
    fn resource_config_deserializes_camel_case_storage_switches() {
        let cfg: ResourceConfig =
            serde_json::from_value(serde_json::json!({ "pvcEnabled": true, "s3Enabled": false }))
                .expect("deserialize ResourceConfig");
        assert_eq!(cfg.pvc_enabled, Some(true));
        assert_eq!(cfg.s3_enabled, Some(false));
    }

    /// A repo config written against the removed `storageMode` field must still
    /// load — the switches then fall back to the deployment default rather than
    /// the whole workspace config failing to parse.
    #[test]
    fn resource_config_ignores_a_stale_storage_mode() {
        let cfg: ResourceConfig =
            serde_json::from_value(serde_json::json!({ "storageMode": "ephemeral" }))
                .expect("a stale storageMode must not fail deserialization");
        assert_eq!(cfg.pvc_enabled, None);
        assert_eq!(cfg.s3_enabled, None);
    }

    #[test]
    fn docker_storage_mode_validation_accepts_known_and_normalizes() {
        assert!(is_valid_docker_storage_mode("node"));
        assert!(is_valid_docker_storage_mode("emptyDir"));
        assert!(is_valid_docker_storage_mode("  EMPTYDIR "));
        assert!(!is_valid_docker_storage_mode("s3"));
        assert!(!is_valid_docker_storage_mode(""));
    }

    #[test]
    fn build_cache_mode_validation_accepts_known_and_normalizes() {
        assert!(is_valid_build_cache_mode("node"));
        assert!(is_valid_build_cache_mode("emptyDir"));
        assert!(is_valid_build_cache_mode("  EMPTYDIR "));
        assert!(!is_valid_build_cache_mode("s3"));
        assert!(!is_valid_build_cache_mode(""));
    }

    #[test]
    fn build_cache_scope_validation_accepts_known_and_normalizes() {
        assert!(is_valid_build_cache_scope("task"));
        assert!(is_valid_build_cache_scope("node"));
        assert!(is_valid_build_cache_scope("  NODE "));
        assert!(!is_valid_build_cache_scope("shared"));
        assert!(!is_valid_build_cache_scope(""));
    }
}
