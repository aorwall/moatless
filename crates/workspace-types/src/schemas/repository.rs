//! Repository request/response types.

use serde::{Deserialize, Serialize};

use crate::repository::{RepositoryKind, RepositoryProvider};
use moatless_types::access_control::Scope;

// Workspace run configuration is defined in the Workspace wire module.
#[deprecated(note = "use moatless_workspace_types::schemas::workspace::ResourceConfig")]
pub use crate::schemas::workspace::ResourceConfig;
#[deprecated(note = "use moatless_workspace_types::schemas::workspace::ScriptConfig")]
pub use crate::schemas::workspace::ScriptConfig;

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
    pub provider: Option<RepositoryProvider>,
    pub name: Option<String>,
    /// Change the repository's access scope (`global` or `user`); requires
    /// admin for `global`.
    ///
    /// Re-submitting `user` preserves the existing immutable owner; an
    /// ownerless repository cannot move to `user`.
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
    #[serde(deserialize_with = "Option::deserialize")]
    #[cfg_attr(feature = "openapi", schema(required = true))]
    pub remote: Option<RepositoryRemoteResponse>,
    pub id: String,
    pub name: String,
    pub kind: RepositoryKind,
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
// ListBranchesQuery / BranchListResponse / BranchEntry
// =============================================================================

/// `GET /api/v1/repositories/{repository_id}/branches` query.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[serde(rename_all = "camelCase")]
pub struct ListBranchesQuery {
    /// List on behalf of this workspace, so a repository only the workspace's
    /// pinned credential can reach still answers. Omitted, the listing is made
    /// as the caller's own identity.
    pub workspace_id: Option<String>,
}

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

/// Resolved facts about the repository's remote.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RepositoryRemoteResponse {
    pub url: String,
    pub uses_ssh: bool,
    pub host_repo: Option<GitHostRepoResponse>,
}

/// A repository's identity on its declared git host.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GitHostRepoResponse {
    pub provider: RepositoryProvider,
    pub host: String,
    pub owner: String,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use crate::schemas::workspace::StorageMedium;

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
