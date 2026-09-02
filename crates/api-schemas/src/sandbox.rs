//! Sandbox API request/response schemas.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::enums::SandboxDesiredState;

// =============================================================================
// Sandbox model types (shared between schemas and backend)
// =============================================================================

pub use moatless_types::sandbox::{
    AgentStatus, LiveStatus, SandboxStatus, ServerCapabilities, ServerEntry, ServerStatus,
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
/// In K8s mode, these are sourced from K8s Event objects (FailedScheduling,
/// BackOff, Pulling, etc.). Docker mode can populate from `docker events`.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEvent {
    /// Event reason (e.g. "BackOff", "FailedScheduling", "Pulling", "OOMKilled")
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

/// Lifecycle of a command registered through `moat cmd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub enum ManagedCommandState {
    Running,
    Exited,
    /// Killed by the sandbox because it outlived its deadline.
    TimedOut,
    /// Killed on an explicit `moat cmd kill`.
    Killed,
}

/// One long-running command the agent registered through `moat cmd`. The
/// sandbox owns the process, so it is the only work that defers the idle
/// reaper — a process forked any other way never appears here.
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

/// Full sandbox status response to frontend (matches TS SandboxStatusFullResponse)
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
    /// Image a fresh (re)deploy would use for the sandbox container: the
    /// per-repo `docker_image` override when set, otherwise the configured
    /// default. Compare against `current_sandbox_image` to know whether a
    /// redeploy would change the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_sandbox_image: Option<String>,
    /// `true` when `current_sandbox_image` matches `latest_sandbox_image`.
    /// `None` when either image is unknown (e.g. no sandbox provisioned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_up_to_date: Option<bool>,
}

/// Server status response proxied from sandbox pod.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusResponse {
    pub servers: Vec<ServerSnapshot>,
}

/// A live override of one preview server's start command and env.
///
/// Stored only as control files on the sandbox's workspace volume — there is no
/// database copy — so every read of one needs a running sandbox.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_command: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unset_env: Vec<String>,
}

/// Update one preview server's live config.
///
/// `port` and `dockerImage` exist only so a caller that sends them gets an
/// explicit "redeploy required" rather than a silent no-op.
#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct UpdateServerConfigRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_commands: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unset_env: Option<Vec<String>>,
    pub restart: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_image: Option<String>,
}

/// Result of a live server config change or restart.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerConfigResponse {
    pub server_name: String,
    pub config_override: ServerOverride,
    pub generation: u64,
    pub restarted: bool,
}

/// One server's override as it currently exists on the workspace volume.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerOverrideEntry {
    pub server_name: String,
    pub config_override: ServerOverride,
    /// The counter the sidecar watches; `0` means it has never been restarted.
    pub generation: u64,
}

/// Every override in effect on a task. Servers with no override are omitted.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerOverridesResponse {
    pub overrides: Vec<ServerOverrideEntry>,
}

// ── File operation DTOs ──────────────────────────────────────────

/// File tree response with git change summary.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FileTreeResponse {
    pub tree: Vec<FileTreeNodeDto>,
    pub git_summary: GitSummaryDto,
}

/// A node in the file tree (file or directory).
#[derive(Debug, Serialize, Deserialize)]
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
#[derive(Debug, Serialize, Deserialize)]
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
#[derive(Debug, Serialize, Deserialize)]
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
}

/// Request body for writing a file.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WriteFileRequestBody {
    pub path: String,
    pub content: String,
}

/// Response for listing files.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ListFilesResponse {
    pub files: Vec<String>,
}

/// Request body for searching files by name.
#[derive(Debug, Serialize, Deserialize)]
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
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SearchByNameResponse {
    pub files: Vec<FileNameMatchDto>,
    pub total_matches: i32,
}

/// A file name match in the search results.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FileNameMatchDto {
    pub path: String,
    pub matches: i32,
}

// ── Moatless tool names ──────────────────────────────────────────

/// Response from provisioning a sandbox.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ProvisionResponse {
    pub status: String,
}

/// All available Moatless tool names injected into the agent sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
