//! Types for the workspace K8s operations API.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

/// Generic error body returned by all error responses.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: String,
}

/// Provision plan sent from TS backend
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionRequest {
    pub plan: ProvisionPlan,
    pub user_id: String,
}

/// Provision plan with all configuration needed to create a workspace
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionPlan {
    pub repo_config: RepoConfig,
    pub git_clone_url: String,
    pub git_branch: Option<String>,
    pub git_provider_token: Option<String>,
    pub claude_oauth_token: Option<String>,
    pub anthropic_api_key: Option<String>,
    /// Custom environment variables
    pub env_vars: Option<HashMap<String, String>>,
}

/// Repository config for provisioning and sandbox environment
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoConfig {
    pub name: String,
    /// Per-repository sandbox image override
    pub docker_image: Option<String>,
    /// Servers with allocated ports
    pub servers: Option<Vec<ServerConfig>>,
    /// Plugin IDs (format: {name}@{marketplace}) — sandbox resolves to paths
    pub plugin_ids: Option<Vec<String>>,
    // --- Pass-through fields for REPO_CONFIG env var ---
    /// System prompt for the agent
    pub system_prompt: Option<String>,
    /// Code execution configuration (dockerImage, envVars, etc.)
    #[schema(value_type = Object)]
    pub code_execution: Option<serde_json::Value>,
    /// Preview server configuration
    #[schema(value_type = Object)]
    pub preview_config: Option<serde_json::Value>,
}

/// Server config entry
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub name: String,
    pub port: u16,
}

/// Per-user credentials for K8s Secret
#[derive(Debug, Clone, Serialize)]
pub struct UserCredentials {
    pub github_token: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub claude_oauth_token: Option<String>,
    pub user_auth_token: Option<String>,
}

/// Matches TS WorkspaceExistenceSchema: 'exists' | 'missing' | 'invalid'
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceState {
    Exists,
    Missing,
}

/// Matches TS EnvironmentStatusSchema
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentStatus {
    NotCreated,
    Initializing,
    Ready,
    Stopped,
    Removing,
    Error,
}

impl std::fmt::Display for EnvironmentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCreated => write!(f, "not_created"),
            Self::Initializing => write!(f, "initializing"),
            Self::Ready => write!(f, "ready"),
            Self::Stopped => write!(f, "stopped"),
            Self::Removing => write!(f, "removing"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Matches TS AgentStateSchema: 'idle' | 'running' | 'none'
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Idle,
    Running,
    None,
}

impl From<&str> for AgentState {
    fn from(s: &str) -> Self {
        match s {
            "running" => AgentState::Running,
            "idle" => AgentState::Idle,
            _ => AgentState::None,
        }
    }
}

/// Matches TS ServerStatusEnumSchema: 'stopped' | 'installing' | 'starting' | 'started' | 'failed'
#[derive(Debug, Clone, Serialize, ToSchema)]
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

/// Execution target returned after provisioning
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionTarget {
    pub mode: String,
    pub pod_name: Option<String>,
    pub pod_namespace: String,
    pub service_name: String,
    pub pod_phase: Option<String>,
    pub status: EnvironmentStatus,
    pub repository_clone_url: String,
    pub repository_branch: Option<String>,
    pub sandbox_port: u16,
    pub preview_host_port: u16,
    pub servers: HashMap<String, u16>,
}

/// Provision response
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionResponse {
    pub execution_target: ExecutionTarget,
}

/// Status response
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusResponse {
    pub status: EnvironmentStatus,
    pub pod_name: Option<String>,
    pub last_error: Option<String>,
}

/// Health response
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub healthy: bool,
}

/// Pod phase as returned by K8s API
#[derive(Debug, Clone, PartialEq)]
pub enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
    Terminating,
    NotFound,
    /// Deployment exists but scaled to 0 replicas.
    Stopped,
}

impl PodPhase {
    pub fn as_str(&self) -> &str {
        match self {
            PodPhase::Pending => "Pending",
            PodPhase::Running => "Running",
            PodPhase::Succeeded => "Succeeded",
            PodPhase::Failed => "Failed",
            PodPhase::Unknown => "Unknown",
            PodPhase::Terminating => "Terminating",
            PodPhase::NotFound => "NotFound",
            PodPhase::Stopped => "Stopped",
        }
    }

    /// Map to unified environment status
    pub fn to_environment_status(&self) -> EnvironmentStatus {
        match self {
            PodPhase::Pending => EnvironmentStatus::Initializing,
            PodPhase::Running => EnvironmentStatus::Ready,
            PodPhase::Succeeded | PodPhase::Stopped => EnvironmentStatus::Stopped,
            PodPhase::Terminating => EnvironmentStatus::Removing,
            PodPhase::NotFound => EnvironmentStatus::NotCreated,
            PodPhase::Failed | PodPhase::Unknown => EnvironmentStatus::Error,
        }
    }
}

impl From<&str> for PodPhase {
    fn from(s: &str) -> Self {
        match s {
            "Pending" => PodPhase::Pending,
            "Running" => PodPhase::Running,
            "Succeeded" => PodPhase::Succeeded,
            "Failed" => PodPhase::Failed,
            "Terminating" => PodPhase::Terminating,
            _ => PodPhase::Unknown,
        }
    }
}

// =============================================================================
// Workspace status types (sandbox snapshot + frontend response)
// =============================================================================

/// Agent snapshot from sandbox GET /status
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct AgentSnapshot {
    #[serde(alias = "status")]
    pub state: String,
    #[serde(rename = "hasAgent", default)]
    pub has_agent: bool,
    #[serde(alias = "lastError")]
    pub error: Option<String>,
}

/// Server snapshot from sandbox GET /status
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[allow(dead_code)]
pub struct ServerSnapshot {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub builtin: bool,
    pub status: String,
    pub error: Option<String>,
    pub pid: Option<u32>,
}

/// Resource metrics from sandbox GET /status
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceMetrics {
    pub memory_usage_mb: f64,
    pub memory_limit_mb: Option<f64>,
    pub memory_usage_percent: Option<f64>,
    pub cpu_usage_percent: Option<f64>,
    pub event_loop_latency_ms: f64,
    pub uptime_seconds: f64,
}

/// Combined snapshot from sandbox GET /status
#[derive(Debug, Deserialize)]
pub struct SandboxSnapshotStatus {
    pub agent: AgentSnapshot,
    pub servers: Vec<ServerSnapshot>,
    pub resources: Option<ResourceMetrics>,
}

/// Server capabilities (deterministic from server name)
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    pub address_bar: bool,
    pub inspector: bool,
    pub reload: bool,
    pub open_external: bool,
    pub viewport: bool,
    pub console: bool,
}

/// Single server entry in the workspace status response
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerEntry {
    pub name: String,
    pub label: String,
    pub port: u16,
    pub status: ServerStatus,
    pub url: Option<String>,
    pub error: Option<String>,
    pub default: bool,
    pub capabilities: ServerCapabilities,
}

/// Cached sandbox state stored in Redis Hash (`moatless:sandbox:state`).
///
/// Stores the full workspace state for cross-replica caching. Writers include
/// the pod lifecycle watcher (environment fields) and the status endpoint (full state).
/// `ServerSnapshot` is stored instead of `ServerEntry` to keep the cache config-independent
/// — proxy URLs and capabilities are computed on read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedSandboxState {
    pub environment_status: EnvironmentStatus,
    pub environment_error: Option<String>,
    pub workspace_state: WorkspaceState,
    pub pod_name: Option<String>,
    pub agent_state: AgentState,
    pub servers: Vec<ServerSnapshot>,
    pub resources: Option<ResourceMetrics>,
    pub is_running: bool,
    /// Unix timestamp millis — for freshness check
    pub updated_at: i64,
}

/// Full workspace status response to frontend (matches TS WorkspaceStatusResponse)
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStatusFullResponse {
    pub workspace_state: WorkspaceState,
    pub environment_status: EnvironmentStatus,
    pub environment_error: Option<String>,
    pub agent_state: AgentState,
    pub servers: Vec<ServerEntry>,
    pub is_running: bool,
    pub workspace_ready: bool,
    pub can_execute: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceMetrics>,
}

// =============================================================================
// Preview / workspace types
// =============================================================================

/// Request body for file search in sandbox.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchFilesRequest {
    pub query: String,
    pub limit: Option<u32>,
}

/// Single file search result from sandbox.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileSearchResult {
    pub path: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_type: Option<String>,
}

/// Server status response proxied from sandbox pod.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusResponse {
    pub servers: Vec<ServerSnapshot>,
}

/// Get capabilities for a server by name (mirrors TS getServerCapabilities)
pub fn get_server_capabilities(name: &str) -> ServerCapabilities {
    match name {
        "files" => ServerCapabilities {
            address_bar: false,
            inspector: false,
            reload: true,
            open_external: true,
            viewport: false,
            console: false,
        },
        "preview" => ServerCapabilities {
            address_bar: true,
            inspector: true,
            reload: true,
            open_external: true,
            viewport: true,
            console: true,
        },
        _ => ServerCapabilities {
            address_bar: true,
            inspector: false,
            reload: true,
            open_external: true,
            viewport: false,
            console: false,
        },
    }
}
