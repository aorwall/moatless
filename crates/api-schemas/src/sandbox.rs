//! Sandbox API request/response schemas.

use serde::{Deserialize, Serialize};

// =============================================================================
// Sandbox model types (shared between schemas and backend)
// =============================================================================

/// Matches TS SandboxStateSchema: 'exists' | 'missing'
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum SandboxState {
    Exists,
    Missing,
}

/// Matches TS AgentStateSchema: 'idle' | 'running' | 'none' | 'unknown'
///
/// `Unknown` means the sandbox is running/ready but the agent could not be
/// reached over gRPC — distinct from `None` (idle / never started).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Idle,
    Running,
    None,
    Unknown,
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Idle => write!(f, "idle"),
            AgentState::Running => write!(f, "running"),
            AgentState::None => write!(f, "none"),
            AgentState::Unknown => write!(f, "unknown"),
        }
    }
}

impl From<&str> for AgentState {
    fn from(s: &str) -> Self {
        match s {
            "running" => AgentState::Running,
            "idle" => AgentState::Idle,
            "unknown" => AgentState::Unknown,
            _ => AgentState::None,
        }
    }
}

/// Matches TS ServerStatusEnumSchema: 'stopped' | 'installing' | 'starting' | 'started' | 'failed'
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ServerStatus {
    Stopped,
    Installing,
    Starting,
    Started,
    Failed,
}

impl From<&str> for ServerStatus {
    fn from(s: &str) -> Self {
        match s {
            "stopped" => ServerStatus::Stopped,
            "installing" => ServerStatus::Installing,
            "starting" => ServerStatus::Starting,
            "started" => ServerStatus::Started,
            "failed" => ServerStatus::Failed,
            _ => ServerStatus::Stopped,
        }
    }
}

impl std::fmt::Display for ServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped => write!(f, "stopped"),
            Self::Installing => write!(f, "installing"),
            Self::Starting => write!(f, "starting"),
            Self::Started => write!(f, "started"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

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

/// Server capabilities (deterministic from server name)
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub address_bar: bool,
    pub inspector: bool,
    pub reload: bool,
    pub open_external: bool,
    pub viewport: bool,
    pub console: bool,
}

// =============================================================================
// API response schemas
// =============================================================================

/// Single server entry in the workspace status response
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerEntry {
    pub name: String,
    pub label: String,
    pub port: u16,
    pub status: ServerStatus,
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub default: bool,
    pub capabilities: ServerCapabilities,
}

/// A provider-agnostic runtime event associated with a sandbox.
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
}

/// Full sandbox status response to frontend (matches TS SandboxStatusFullResponse)
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SandboxStatusFullResponse {
    pub sandbox_state: SandboxState,
    /// Uses `String` here — the concrete `SandboxStatus` enum lives in the events crate.
    /// The backend populates this from `moatless_events::SandboxStatus`.
    pub environment_status: String,
    pub environment_error: Option<String>,
    pub agent_state: AgentState,
    pub servers: Vec<ServerEntry>,
    pub sandbox_ready: bool,
    pub can_execute: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_diagnostics: Option<Vec<ContainerDiagnostics>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_events: Option<Vec<RuntimeEvent>>,
}

/// Request body for file search in sandbox.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SearchFilesRequest {
    pub query: String,
    pub limit: Option<u32>,
}

/// Single file search result from sandbox.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct FileSearchResult {
    pub path: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_type: Option<String>,
}

/// Server status response proxied from sandbox pod.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusResponse {
    pub servers: Vec<ServerSnapshot>,
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
