//! Sandbox API request/response schemas.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::enums::SandboxDesiredState;

// =============================================================================
// Sandbox model types (shared between schemas and backend)
// =============================================================================

pub use moatless_types::sandbox::{
    AgentStatus, CommandSummary, LiveStatus, ManagedCommandState, SandboxStatus,
    ServerCapabilities, ServerEntry, ServerStatus,
};

/// Server snapshot from sandbox GET /status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ServerSnapshot {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub enable_inspector: bool,
    pub status: ServerStatus,
    pub error: Option<String>,
    pub pid: Option<u32>,
}

/// Direct access to one task's workspace `restic` repository, for the `restic`
/// CLI.
///
/// Every field is a credential: the session reaches only this task's prefix,
/// the password opens only this task's repository, and both are live until
/// `expires_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRepositoryAccessResponse {
    /// The repository as `restic -r` addresses it.
    pub repository: String,
    pub bucket: String,
    /// Key prefix the repository is rooted at, with no trailing slash.
    pub prefix: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    /// RFC 3339. The credentials stop working here; the password does not
    /// expire.
    pub expires_at: String,
    pub password: String,
}

/// Resource metrics from sandbox GET /status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ResourceMetrics {
    pub memory_usage_mb: f64,
    pub memory_limit_mb: Option<f64>,
    pub memory_usage_percent: Option<f64>,
    pub cpu_usage_percent: Option<f64>,
    pub event_loop_latency_ms: f64,
    pub uptime_seconds: f64,
}

// =============================================================================
// API response schemas
// =============================================================================

/// A provider-agnostic runtime event associated with a sandbox.
///
/// K8s mode sources these from K8s Event objects (`FailedScheduling`,
/// `BackOff`, Pulling); Docker mode populates from `docker events`.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEvent {
    /// Event reason (e.g. "`BackOff`", "`FailedScheduling`", "Pulling",
    /// "`OOMKilled`")
    pub reason: String,
    /// Human-readable message
    pub message: String,
    /// Severity: "normal" or "warning"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    /// Number of times this event has occurred
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<i32>,
    /// Last time the event occurred (ISO 8601)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_timestamp: Option<String>,
}

/// Container-level diagnostics for troubleshooting sandbox issues.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ContainerDiagnostics {
    pub name: String,
    /// Container state: "running", "waiting", "terminated", "unknown"
    pub state: String,
    pub restart_count: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminated_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_terminated_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_reason: Option<String>,
    /// CPU usage in millicores (from K8s Metrics API, if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_millicores: Option<u64>,
    /// Memory usage in megabytes (from K8s Metrics API, if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u64>,
}

/// Wakeup timer armed by the agent via `ScheduleWakeup`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct WakeupJobInfo {
    pub reason: String,
    pub prompt_preview: String,
    pub wakeup_at_unix_ms: i64,
}

/// Watch that re-triggers the agent once one of the registered commands
/// captured at the end of a turn finishes (subprocess mode only).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct BackgroundWatchJobInfo {
    pub command_count: u32,
    pub commands: Vec<String>,
}

/// Per-kind payload for a scheduled job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScheduledJobKind {
    Wakeup(WakeupJobInfo),
    BackgroundWatch(BackgroundWatchJobInfo),
}

/// Snapshot of one scheduled job currently registered in the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ScheduledJobInfo {
    pub id: String,
    pub registered_at_unix_ms: i64,
    #[serde(flatten)]
    pub kind: ScheduledJobKind,
}

/// One long-running command the agent registered through `moat cmd`. The
/// sandbox owns the process, so it is the only work that defers the idle reaper
/// — a process forked any other way never appears here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ManagedCommand {
    pub id: String,
    pub command: String,
    pub label: String,
    pub pid: u32,
    pub started_at_unix_ms: i64,
    /// Seconds the sandbox gives it before killing it and marking it timed out.
    pub timeout_secs: u64,
    pub deadline_unix_ms: i64,
    pub state: ManagedCommandState,
    #[cfg_attr(feature = "openapi", schema(required = true))]
    pub exit_code: Option<i32>,
    /// Combined stdout/stderr, readable from inside the sandbox.
    pub log_path: String,
    pub finished_at_unix_ms: i64,
}

impl ManagedCommand {
    /// Narrow to what a live-status reader needs. See [`CommandSummary`].
    pub fn summary(&self) -> CommandSummary {
        CommandSummary {
            id: self.id.clone(),
            label: self.label.clone(),
            state: self.state,
            started_at_unix_ms: self.started_at_unix_ms,
            deadline_unix_ms: self.deadline_unix_ms,
            exit_code: self.exit_code,
        }
    }
}

/// Full sandbox status response to frontend (matches TS
/// `SandboxStatusFullResponse`)
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SandboxStatusFullResponse {
    pub sandbox_status: SandboxStatus,
    #[cfg_attr(feature = "openapi", schema(required = true))]
    pub sandbox_error: Option<String>,
    #[cfg_attr(feature = "openapi", schema(required = true))]
    pub agent_status: Option<AgentStatus>,
    pub servers: Vec<ServerEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_diagnostics: Option<Vec<ContainerDiagnostics>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_events: Option<Vec<RuntimeEvent>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub scheduled_jobs: Vec<ScheduledJobInfo>,
    /// Commands registered through `moat cmd`, running and recently finished.
    /// Empty when the agent registered none.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub commands: Vec<ManagedCommand>,
    /// Desired sandbox lifecycle intent recorded on the task
    /// (`running`/`stopped`/`removed`). Reflects the last user/system action,
    /// independent of the live pod phase. `None` when it can't be resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_desired_state: Option<SandboxDesiredState>,
    /// Image the currently-provisioned sandbox container is running, read from
    /// the live Deployment/container. `None` when no sandbox exists yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_sandbox_image: Option<String>,
    /// Image a fresh (re)deploy would use: the per-repo `docker_image` override
    /// when set, otherwise the configured default.
    ///
    /// Compare against `current_sandbox_image` to know whether a redeploy would
    /// change the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_sandbox_image: Option<String>,
    /// `true` when `current_sandbox_image` matches `latest_sandbox_image`.
    /// `None` when either image is unknown (e.g. no sandbox provisioned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_up_to_date: Option<bool>,
}

/// Server status response proxied from sandbox pod.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusResponse {
    pub servers: Vec<ServerSnapshot>,
}

pub use moatless_workspace_types::workspace::ServerOverride;

/// Update one preview server's live config.
///
/// Only fields a sidecar can pick up on an in-place restart are accepted;
/// `port` and `dockerImage` are declared so sending them errors "redeploy
/// required" instead of silently no-op.
#[derive(Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateServerConfigRequest {
    /// Replaces the workspace's start command. Joined with `&&`, as at
    /// provision time.
    #[serde(default)]
    pub start_commands: Option<Vec<String>>,
    /// Env entries to set, merged into any existing override.
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
    /// Keys to remove from the server's environment. Also drops them from the
    /// override's `env`, so setting and unsetting the same key is coherent.
    #[serde(default)]
    pub unset_env: Option<Vec<String>>,
    /// Restart the server so the change takes effect (default `true`).
    #[serde(default = "default_restart")]
    pub restart: bool,
    /// Not applicable live — the port is wired into the Service and
    /// `IngressRoute` at provision time.
    #[serde(default)]
    pub port: Option<u16>,
    /// Not applicable live — the image is the container's identity.
    #[serde(default)]
    pub docker_image: Option<String>,
}

/// Result of a live server config change or restart.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerConfigResponse {
    pub server_name: String,
    /// The override after the call — empty when it was reset.
    pub config_override: ServerOverride,
    /// Sidecar restart generation after the call.
    pub generation: u64,
    pub restarted: bool,
}

/// One server's override as it currently exists on the workspace volume.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerOverrideEntry {
    pub server_name: String,
    pub config_override: ServerOverride,
    /// The counter the sidecar watches; `0` means it has never been restarted.
    pub generation: u64,
}

/// Every override in effect on a task. Servers with no override are omitted.
///
/// Read from the sandbox's workspace volume, the only place an override exists
/// — a stopped sandbox cannot answer this.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerOverridesResponse {
    pub overrides: Vec<ServerOverrideEntry>,
}

// ── File operation DTOs ──────────────────────────────────────────

/// File tree response with git change summary.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FileTreeResponse {
    pub tree: Vec<FileTreeNodeDto>,
    pub git_summary: GitSummaryDto,
    pub origin: ContentOrigin,
}

/// A node in the file tree (file or directory).
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FileTreeNodeDto {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub is_changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additions: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletions: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_staged: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(no_recursion))]
    pub children: Option<Vec<FileTreeNodeDto>>,
}

/// Git change summary across the workspace.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct GitSummaryDto {
    pub total_files: i32,
    pub total_additions: i32,
    pub total_deletions: i32,
    pub total_staged: i32,
    pub total_modified: i32,
    pub compare_base: String,
    pub has_changes: bool,
}

/// File content response with optional git change info.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FileContentResponse {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additions: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletions: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_modified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_binary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub origin: ContentOrigin,
}

/// Request body for writing a file.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WriteFileRequestBody {
    pub path: String,
    pub content: String,
}

/// Response for listing files.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ListFilesResponse {
    pub files: Vec<String>,
    pub origin: ContentOrigin,
}

/// Request body for searching files by name.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SearchByNameRequest {
    pub query: String,
    #[serde(default)]
    pub case_sensitive: Option<bool>,
    #[serde(default)]
    pub max_results: Option<i32>,
}

/// Response for searching files by name.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SearchByNameResponse {
    pub files: Vec<FileNameMatchDto>,
    pub total_matches: i32,
    pub origin: ContentOrigin,
}

/// A file name match in the search results.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FileNameMatchDto {
    pub path: String,
    pub matches: i32,
}

// ── Moatless tool names ──────────────────────────────────────────

/// Provision response
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ProvisionResponse {
    pub status: SandboxStatus,
}

/// All available Moatless tool names injected into the agent sandbox.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum MoatlessToolName {
    #[serde(rename = "updateTaskMetadata")]
    UpdateTaskMetadata,
    #[serde(rename = "install")]
    Install,
    #[serde(rename = "updateRepositoryConfig")]
    UpdateRepositoryConfig,
    #[serde(rename = "getRepositoryConfig")]
    GetRepositoryConfig,
    #[serde(rename = "startOrRestartPreviewServer")]
    StartOrRestartPreviewServer,
    #[serde(rename = "getPreviewServerLogs")]
    GetPreviewServerLogs,
    #[serde(rename = "navigatePreview")]
    NavigatePreview,
    #[serde(rename = "clickElement")]
    ClickElement,
    #[serde(rename = "evaluateJavaScript")]
    EvaluateJavaScript,
    #[serde(rename = "fillPreviewForm")]
    FillPreviewForm,
    #[serde(rename = "takePreviewSnapshot")]
    TakePreviewSnapshot,
}

/// Which copy of the workspace answered a file read.
///
/// A stopped sandbox is still readable from the stored workspace snapshot, so a
/// caller must be able to tell live content from content frozen when the pod
/// stopped.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub enum ContentSource {
    /// A running sandbox read the file from its own disk.
    Sandbox,
    /// The stored workspace answered, because no sandbox is running.
    Snapshot,
}

/// Where a file answer came from, carried by every file response.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ContentOrigin {
    pub source: ContentSource,
    /// When the snapshot this came from was written, RFC 3339. Absent for
    /// [`ContentSource::Sandbox`], whose answer is current by construction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<String>,
}

impl ContentOrigin {
    pub fn sandbox() -> Self {
        Self {
            source: ContentSource::Sandbox,
            synced_at: None,
        }
    }

    pub fn snapshot(synced_at: chrono::DateTime<chrono::Utc>) -> Self {
        Self {
            source: ContentSource::Snapshot,
            synced_at: Some(synced_at.to_rfc3339()),
        }
    }
}

fn default_restart() -> bool {
    true
}

#[cfg(test)]
mod wire_contract_tests {
    use super::*;

    #[test]
    fn omitted_restart_matches_the_live_endpoint_default() {
        let request: UpdateServerConfigRequest = serde_json::from_str("{}").expect("request");
        assert!(request.restart);
        let request: UpdateServerConfigRequest =
            serde_json::from_str(r#"{"restart":false}"#).expect("request");
        assert!(!request.restart);
    }

    #[test]
    fn binary_snapshot_content_round_trips_without_losing_provenance() {
        let wire = serde_json::json!({
            "content": "AAEC", "isBinary": true, "mediaType": "image/png",
            "origin": { "source": "snapshot", "syncedAt": "2026-09-06T10:00:00+00:00" }
        });
        let response: FileContentResponse = serde_json::from_value(wire.clone()).expect("response");
        assert_eq!(serde_json::to_value(response).expect("serialized"), wire);
    }

    #[test]
    fn live_origin_does_not_claim_a_snapshot_timestamp() {
        assert_eq!(
            serde_json::to_value(ContentOrigin::sandbox()).expect("origin"),
            serde_json::json!({"source": "sandbox"})
        );
    }
}
