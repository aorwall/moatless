//! Workspace request/response types.
//!
//! A Workspace owns the run-configuration and holds 0..n repository placements
//! (`WorkspaceRepo`). Run-config fields use the [`ResourceConfig`]
//! and [`ScriptConfig`] wire shapes defined here.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use moatless_types::access_control::Scope;

/// The glyph a client renders beside a Workspace name.
///
/// The domain enum itself — same shape the column stores, no wire conversion.
pub use crate::workspace::WorkspaceIcon;

pub use crate::workspace::LanguageServer;

// =============================================================================
// ResourceConfig
// =============================================================================

/// Where a node-local sandbox volume is backed.
///
/// The domain enum itself — same type the provider reads, no wire conversion.
pub use crate::workspace::StorageMedium;

/// A container's CPU and memory for one direction (requests or limits), as
/// Kubernetes quantity strings (`"250m"`, `"4Gi"`); each field overrides the
/// deployment default independently.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ResourceQuantities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

impl ResourceQuantities {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cpu.is_none() && self.memory.is_none()
    }
}

/// Per-repository K8s resource requests/limits configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ResourceConfig {
    #[serde(default, skip_serializing_if = "ResourceQuantities::is_empty")]
    pub requests: ResourceQuantities,
    #[serde(default, skip_serializing_if = "ResourceQuantities::is_empty")]
    pub limits: ResourceQuantities,
    /// Per-repository workspace storage size override (e.g. "8Gi").
    ///
    /// Sizes the per-task PVC when `pvc_enabled`, else the node-local emptyDir.
    /// Defaults to `K8S_SANDBOX_STORAGE_SIZE`; never copied into container
    /// resource specs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
    /// Whether this repo's sandbox gets a per-task PVC at `/opt/moatless`.
    ///
    /// Survives pod restarts, unlike the emptyDir fallback. Defaults to
    /// `K8S_SANDBOX_DEFAULT_PVC`; `K8S_SANDBOX_STORAGE_ENABLED` overrides both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pvc_enabled: Option<bool>,
    /// Whether this repo's sandbox snapshots its workspace to S3.
    ///
    /// The copy that outlives the Deployment: pushed before stop/remove, pulled
    /// back on start when the volume holds nothing usable. Defaults to
    /// `K8S_SANDBOX_DEFAULT_S3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_enabled: Option<bool>,
    /// Per-repository size cap for the `dind` sidecar's `/var/lib/docker`
    /// emptyDir.
    ///
    /// Only meaningful with `dockerAccess` + `dockerStorageMode: "emptyDir"`.
    /// Exceeding it gets the pod evicted, wiping all dind data. Defaults to
    /// `K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_storage: Option<String>,
    /// Storage backend for the `dind` sidecar's `/var/lib/docker`.
    ///
    /// `"node"` (default): best-effort hostPath, survives stop/start only on
    /// the same node. `"emptyDir"`: capped by `dockerStorage`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_storage_mode: Option<StorageMedium>,
}

// =============================================================================
// ScriptConfig
// =============================================================================

/// Named command run on demand inside a Sandbox.
///
/// `name` is the identity, since it must survive a git round trip; no id is
/// allocated by the backend.
pub use crate::workspace::ScriptConfig;

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
    #[serde(default)]
    pub icon: Option<WorkspaceIcon>,
    /// Derived from the placement count: `blank` (0), `single` (1), `multi`
    /// (n).
    pub kind: String,
    pub docker_image: Option<String>,
    pub docker_access: Option<bool>,
    pub sandbox_service_account: Option<String>,
    pub system_prompt: Option<String>,
    pub setup_commands: Option<Vec<String>>,
    pub env_vars: Option<HashMap<String, String>>,
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Object, ignore))]
    pub scripts: Option<Vec<ScriptConfig>>,
    pub resources: Option<ResourceConfig>,
    #[serde(default)]
    pub language_servers: Option<Vec<LanguageServer>>,
    #[serde(default)]
    pub network_profile: Option<String>,
    #[serde(default)]
    pub node_selector: Option<BTreeMap<String, String>>,
    /// Model catalog id a new Task on this Workspace starts with. An open
    /// string: the catalog is deployment configuration.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Reasoning-effort level paired with `default_model`.
    #[serde(default)]
    pub default_effort: Option<String>,
    /// `manual` or `git`. Git-synced workspaces are read-only in the UI and
    /// write API — only a re-sync from git changes them.
    #[serde(default)]
    pub source: String,
    /// Convenience flag derived from `source == "git"`.
    #[serde(default)]
    pub synced_from_git: bool,
    /// Path to the config file within the source repository that declared this
    /// workspace (`.moatless/workspaces.json`). Retained even after a manual
    /// override so the origin remains visible.
    #[serde(default)]
    pub source_config_path: Option<String>,
    /// The repository whose git tree declared this workspace (`source ==
    /// "git"`).
    #[serde(default)]
    pub source_repository_id: Option<String>,
    /// The entry's stable key within the declaring file (its `key`, else the
    /// slugified name). Retained after an override, like the other provenance.
    #[serde(default)]
    pub source_key: Option<String>,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    /// Identity every Task on this Workspace acts as on a git host. When set it
    /// outranks the Task's own owner.
    #[serde(default)]
    pub run_as_user_id: Option<String>,
    pub created_by: Option<String>,
    /// Soft-deleted workspaces are normally hidden. Git-origin tombstones
    /// remain visible in admin so users can restore them from git without
    /// automatic sync recreating them behind their back.
    #[serde(default)]
    pub deleted: bool,
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

/// Something the declaring file said that the sync did not act on.
///
/// An unknown key and a stripped admin-only field both parse without error and
/// both leave the sandbox running something the file does not describe, so each
/// is reported rather than only logged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSyncWarning {
    /// Where the key sits in the declaring file, as a JSON path
    /// (`workspaces[0].previewConfig`).
    pub path: String,
    /// What the sync did with it, and why.
    pub message: String,
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
    /// Legacy manual overrides taken back under git control, discarding their
    /// local edits.
    pub reclaimed: u32,
    /// Workspaces soft-deleted because git no longer declares them.
    pub removed: u32,
    /// Workspaces that were declared but not synced, with reasons.
    pub skipped: Vec<WorkspaceSyncSkip>,
    /// Keys the file declared that the sync read but did not apply.
    pub warnings: Vec<WorkspaceSyncWarning>,
}

// =============================================================================
// Workspace provider credentials
// =============================================================================

/// A provider whose credential can be pinned to a Workspace.
///
/// Limited to single static env-var credentials; Codex and Claude OAuth are
/// excluded since they refresh against the connected user's row, and a
/// workspace-level copy would go stale with nothing to rotate it.
pub use crate::workspace::WorkspaceCredentialProvider;

/// `PUT /api/v1/workspaces/{id}/credentials/{provider}` body.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SetWorkspaceCredentialRequest {
    /// The credential value. Write-only — no read path ever returns it.
    pub value: String,
}

/// Metadata for one workspace credential. The value is deliberately absent.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCredentialResponse {
    pub workspace_id: String,
    pub provider: WorkspaceCredentialProvider,
    /// The environment variable the sandbox receives this credential as.
    pub env_key: String,
    pub created_by: String,
    pub updated_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg(test)]
mod workspace_credential_tests {
    use super::WorkspaceCredentialProvider;
    use std::str::FromStr;

    #[test]
    fn every_provider_round_trips_through_its_persisted_name() {
        for provider in WorkspaceCredentialProvider::ALL {
            assert_eq!(
                WorkspaceCredentialProvider::from_str(provider.as_str()),
                Ok(*provider)
            );
        }
    }
}
