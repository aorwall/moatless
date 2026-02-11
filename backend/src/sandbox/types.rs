//! Types for the workspace K8s operations API.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

// Re-export pod_status functions that were previously defined here
pub use crate::sandbox::k8s::pod_status::extract_container_observed_statuses;

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

/// Provision response
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionResponse {
    pub status: EnvironmentStatus,
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

impl ServerSnapshot {
    /// Build a snapshot from config with a given status (e.g. "stopped").
    pub fn from_config(name: &str, port: u16, status: &str) -> Self {
        Self {
            name: name.to_string(),
            label: name.to_string(),
            port,
            builtin: false,
            status: status.to_string(),
            error: None,
            pid: None,
        }
    }
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

// =============================================================================
// Container status types (K8s API-based server monitoring)
// =============================================================================

/// Observed state of a K8s container.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerState {
    Running,
    Waiting,
    Terminated,
    Unknown,
}

/// Observed status of a single container in a pod (from K8s API).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerObservedStatus {
    pub name: String,
    pub ready: bool,
    pub started: bool,
    pub state: ContainerState,
    pub restart_count: i32,
    pub waiting_reason: Option<String>,
}

impl ContainerObservedStatus {
    /// Map container probes to a server status string for the frontend.
    pub fn to_server_status(&self) -> &'static str {
        use crate::sandbox::k8s::pod_status::SERVER_FAILURE_REASONS;

        if self.ready && self.started {
            "started"
        } else if self.started {
            "starting"
        } else if self.restart_count > 2 {
            "failed"
        } else {
            match (&self.state, self.waiting_reason.as_deref()) {
                (ContainerState::Waiting, Some(reason))
                    if SERVER_FAILURE_REASONS.contains(&reason) =>
                {
                    "failed"
                }
                (ContainerState::Running, _) => "starting",
                (ContainerState::Waiting, _) => "starting",
                (ContainerState::Terminated, _) => "stopped",
                (ContainerState::Unknown, _) => "stopped",
            }
        }
    }
}

/// Build a `ContainerObservedStatus` for testing convenience.
#[cfg(test)]
impl ContainerObservedStatus {
    fn test(
        name: &str,
        ready: bool,
        started: bool,
        state: ContainerState,
        restart_count: i32,
        waiting_reason: Option<&str>,
    ) -> Self {
        Self {
            name: name.to_string(),
            ready,
            started,
            state,
            restart_count,
            waiting_reason: waiting_reason.map(String::from),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // ContainerObservedStatus::to_server_status()
    // =========================================================================

    #[test]
    fn server_status_ready_and_started_is_started() {
        let cs = ContainerObservedStatus::test(
            "preview-server",
            true,
            true,
            ContainerState::Running,
            0,
            None,
        );
        assert_eq!(cs.to_server_status(), "started");
    }

    #[test]
    fn server_status_started_not_ready_is_starting() {
        let cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            true,
            ContainerState::Running,
            0,
            None,
        );
        assert_eq!(cs.to_server_status(), "starting");
    }

    #[test]
    fn server_status_high_restarts_is_failed() {
        let cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            false,
            ContainerState::Running,
            3,
            None,
        );
        assert_eq!(cs.to_server_status(), "failed");
    }

    #[test]
    fn server_status_waiting_err_image_pull_is_failed() {
        let cs = ContainerObservedStatus::test(
            "files-server",
            false,
            false,
            ContainerState::Waiting,
            0,
            Some("ErrImagePull"),
        );
        assert_eq!(cs.to_server_status(), "failed");
    }

    #[test]
    fn server_status_waiting_image_pull_backoff_is_failed() {
        let cs = ContainerObservedStatus::test(
            "files-server",
            false,
            false,
            ContainerState::Waiting,
            0,
            Some("ImagePullBackOff"),
        );
        assert_eq!(cs.to_server_status(), "failed");
    }

    #[test]
    fn server_status_waiting_invalid_image_name_is_failed() {
        let cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            false,
            ContainerState::Waiting,
            0,
            Some("InvalidImageName"),
        );
        assert_eq!(cs.to_server_status(), "failed");
    }

    #[test]
    fn server_status_waiting_crash_loop_backoff_is_failed() {
        let cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            false,
            ContainerState::Waiting,
            0,
            Some("CrashLoopBackOff"),
        );
        assert_eq!(cs.to_server_status(), "failed");
    }

    #[test]
    fn server_status_waiting_container_creating_is_starting() {
        let cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            false,
            ContainerState::Waiting,
            0,
            Some("ContainerCreating"),
        );
        assert_eq!(cs.to_server_status(), "starting");
    }

    #[test]
    fn server_status_waiting_no_reason_is_starting() {
        let cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            false,
            ContainerState::Waiting,
            0,
            None,
        );
        assert_eq!(cs.to_server_status(), "starting");
    }

    #[test]
    fn server_status_terminated_is_stopped() {
        let cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            false,
            ContainerState::Terminated,
            0,
            None,
        );
        assert_eq!(cs.to_server_status(), "stopped");
    }

    #[test]
    fn server_status_unknown_is_stopped() {
        let cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            false,
            ContainerState::Unknown,
            0,
            None,
        );
        assert_eq!(cs.to_server_status(), "stopped");
    }

    #[test]
    fn server_status_running_not_started_is_starting() {
        let cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            false,
            ContainerState::Running,
            0,
            None,
        );
        assert_eq!(cs.to_server_status(), "starting");
    }

    // =========================================================================
    // PodPhase::to_environment_status()
    // =========================================================================

    #[test]
    fn pod_phase_pending_maps_to_initializing() {
        assert_eq!(
            PodPhase::Pending.to_environment_status(),
            EnvironmentStatus::Initializing
        );
    }

    #[test]
    fn pod_phase_running_maps_to_ready() {
        assert_eq!(
            PodPhase::Running.to_environment_status(),
            EnvironmentStatus::Ready
        );
    }

    #[test]
    fn pod_phase_stopped_maps_to_stopped() {
        assert_eq!(
            PodPhase::Stopped.to_environment_status(),
            EnvironmentStatus::Stopped
        );
    }

    #[test]
    fn pod_phase_not_found_maps_to_not_created() {
        assert_eq!(
            PodPhase::NotFound.to_environment_status(),
            EnvironmentStatus::NotCreated
        );
    }

    #[test]
    fn pod_phase_failed_maps_to_error() {
        assert_eq!(
            PodPhase::Failed.to_environment_status(),
            EnvironmentStatus::Error
        );
    }

    #[test]
    fn pod_phase_terminating_maps_to_removing() {
        assert_eq!(
            PodPhase::Terminating.to_environment_status(),
            EnvironmentStatus::Removing
        );
    }

}
