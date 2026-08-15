//! Repository request/response types.

use serde::{Deserialize, Serialize};

use crate::enums::{RepositoryKind, RepositoryProvider, Scope};

// Workspace run configuration is defined in the Workspace wire module.
#[deprecated(note = "use moatless_api_schemas::workspace::ResourceConfig")]
pub use crate::workspace::ResourceConfig;
#[deprecated(note = "use moatless_api_schemas::workspace::ScriptConfig")]
pub use crate::workspace::ScriptConfig;
#[deprecated(note = "use moatless_api_schemas::workspace::ServerConfig")]
pub use crate::workspace::ServerConfig;

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
}
