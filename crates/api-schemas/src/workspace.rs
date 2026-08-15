//! Workspace request/response types.
//!
//! A Workspace owns the run-configuration and holds 0..n repository placements
//! (`WorkspaceRepo`). Run-config fields use the [`ServerConfig`] and
//! [`ResourceConfig`] wire shapes defined here.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::enums::Scope;

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
    /// Mark this placement as the primary (diff/PR/AGENTS.md) repo. At most one
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
    #[cfg_attr(feature = "openapi", schema(value_type = Object, ignore))]
    pub scripts: Option<Vec<ScriptConfig>>,
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
    #[cfg_attr(feature = "openapi", schema(value_type = Object, ignore))]
    pub scripts: Option<Vec<ScriptConfig>>,
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
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Object, ignore))]
    pub scripts: Option<Vec<ScriptConfig>>,
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
    /// Soft-deleted workspaces are normally hidden. Git-origin tombstones remain
    /// visible in admin so users can restore them from git without automatic
    /// sync recreating them behind their back.
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

// =============================================================================
// Workspace provider credentials
// =============================================================================

/// A provider whose credential can be pinned to a Workspace.
///
/// Only providers whose sandbox credential is a single static environment
/// variable are here. Codex (`CODEX_AUTH_JSON`) and the Claude OAuth token are
/// refreshed and written back against the connected user's own row, so a
/// workspace-level copy of either would go stale with nothing to rotate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceCredentialProvider {
    Github,
    Gitlab,
    Gitness,
    Openai,
    Anthropic,
}

impl WorkspaceCredentialProvider {
    /// Every provider a workspace credential may name, in the order the CLI
    /// lists them.
    pub const ALL: &'static [Self] = &[
        Self::Github,
        Self::Gitlab,
        Self::Gitness,
        Self::Openai,
        Self::Anthropic,
    ];

    /// Persisted `workspace_credentials.provider` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Gitness => "gitness",
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    /// Comma-separated provider names, for error messages and CLI help.
    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl std::fmt::Display for WorkspaceCredentialProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for WorkspaceCredentialProvider {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|p| p.as_str() == value)
            .ok_or_else(|| {
                format!(
                    "Unknown credential provider '{value}'. Must be one of: {}",
                    Self::names()
                )
            })
    }
}

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

    #[test]
    fn an_unknown_provider_names_the_supported_set() {
        let err = WorkspaceCredentialProvider::from_str("codex")
            .expect_err("codex is deliberately unsupported");
        assert!(err.contains("github"), "{err}");
    }
}
