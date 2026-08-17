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
    use crate::workspace::StorageMedium;

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
            docker_storage: Some("50Gi".to_string()),
            docker_storage_mode: Some(StorageMedium::Node),
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
        assert_eq!(cfg.docker_storage_mode, Some(StorageMedium::EmptyDir));
    }

    /// The medium is a closed set, so an unrecognised spelling is rejected at
    /// the API rather than silently falling back to the default backend.
    #[test]
    fn resource_config_rejects_an_unknown_docker_storage_mode() {
        for mode in ["tmpfs", "NODE", "emptydir"] {
            assert!(
                serde_json::from_value::<ResourceConfig>(
                    serde_json::json!({ "dockerStorageMode": mode })
                )
                .is_err(),
                "{mode} must be rejected"
            );
        }
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
