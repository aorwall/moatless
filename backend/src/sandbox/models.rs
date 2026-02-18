//! Sandbox domain types — config, provisioning, and K8s status types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

// Re-export pod_status functions that were previously defined here
pub use crate::sandbox::k8s::pod_status::extract_container_observed_statuses;

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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
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

/// Server snapshot from sandbox GET /status
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

impl ServerSnapshot {
    /// Build a snapshot from a `ServerConfig` with a given status (e.g. `ServerStatus::Stopped`).
    pub fn from_config(config: &ServerConfig, status: ServerStatus) -> Self {
        Self {
            name: config.name.clone(),
            label: config.label.clone().unwrap_or_else(|| config.name.clone()),
            port: config.port,
            builtin: false,
            default: config.default,
            enable_inspector: config.enable_inspector,
            status,
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
    /// Map container probes to a server status for the frontend.
    pub fn to_server_status(&self) -> ServerStatus {
        use crate::sandbox::k8s::pod_status::SERVER_FAILURE_REASONS;

        // Terminated containers are always stopped, regardless of started/ready flags.
        // During intentional shutdown (scale-to-0), K8s sends SIGTERM and containers
        // transition through Terminated while `started` may still be true.
        if matches!(self.state, ContainerState::Terminated) {
            return ServerStatus::Stopped;
        }

        if self.ready && self.started {
            ServerStatus::Started
        } else if self.started {
            ServerStatus::Starting
        } else if self.restart_count > 2 {
            ServerStatus::Failed
        } else {
            match (&self.state, self.waiting_reason.as_deref()) {
                (ContainerState::Waiting, Some(reason))
                    if SERVER_FAILURE_REASONS.contains(&reason) =>
                {
                    ServerStatus::Failed
                }
                (ContainerState::Running, _) => ServerStatus::Starting,
                (ContainerState::Waiting, _) => ServerStatus::Starting,
                (ContainerState::Terminated, _) => ServerStatus::Stopped,
                (ContainerState::Unknown, _) => ServerStatus::Stopped,
            }
        }
    }

    /// Return the waiting reason only if it is a failure reason.
    pub fn error_reason(&self) -> Option<String> {
        use crate::sandbox::k8s::pod_status::SERVER_FAILURE_REASONS;

        self.waiting_reason.as_deref().and_then(|r| {
            if SERVER_FAILURE_REASONS.contains(&r) {
                Some(r.to_string())
            } else {
                None
            }
        })
    }

    /// Return the waiting reason only if it is a non-error detail (e.g. ContainerCreating).
    pub fn detail_reason(&self) -> Option<String> {
        use crate::sandbox::k8s::pod_status::SERVER_FAILURE_REASONS;

        self.waiting_reason.as_deref().and_then(|r| {
            if SERVER_FAILURE_REASONS.contains(&r) {
                None
            } else {
                Some(r.to_string())
            }
        })
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

/// Derive capabilities from a `ServerConfig`.
pub fn server_capabilities_from_config(config: &ServerConfig) -> ServerCapabilities {
    ServerCapabilities {
        address_bar: config.enable_inspector,
        inspector: config.enable_inspector,
        reload: true,
        open_external: true,
        viewport: true,
        console: true,
    }
}

/// Derive capabilities from a `ServerSnapshot` (cached path).
pub fn server_capabilities_from_snapshot(snapshot: &ServerSnapshot) -> ServerCapabilities {
    ServerCapabilities {
        address_bar: snapshot.enable_inspector,
        inspector: snapshot.enable_inspector,
        reload: true,
        open_external: true,
        viewport: true,
        console: true,
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
        assert_eq!(cs.to_server_status(), ServerStatus::Started);
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
        assert_eq!(cs.to_server_status(), ServerStatus::Starting);
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
        assert_eq!(cs.to_server_status(), ServerStatus::Failed);
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
        assert_eq!(cs.to_server_status(), ServerStatus::Failed);
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
        assert_eq!(cs.to_server_status(), ServerStatus::Failed);
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
        assert_eq!(cs.to_server_status(), ServerStatus::Failed);
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
        assert_eq!(cs.to_server_status(), ServerStatus::Failed);
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
        assert_eq!(cs.to_server_status(), ServerStatus::Starting);
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
        assert_eq!(cs.to_server_status(), ServerStatus::Starting);
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
        assert_eq!(cs.to_server_status(), ServerStatus::Stopped);
    }

    #[test]
    fn server_status_terminated_with_started_flag_is_stopped() {
        // During shutdown, K8s may still report started=true while container is Terminated.
        // Must return Stopped, not Starting.
        let cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            true, // started=true (still set during graceful shutdown)
            ContainerState::Terminated,
            0,
            None,
        );
        assert_eq!(cs.to_server_status(), ServerStatus::Stopped);
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
        assert_eq!(cs.to_server_status(), ServerStatus::Stopped);
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
        assert_eq!(cs.to_server_status(), ServerStatus::Starting);
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

    // =========================================================================
    // ServerConfig deserialization (backwards compat)
    // =========================================================================

    #[test]
    fn server_config_minimal_json_deserializes_with_defaults() {
        let json = r#"{"name":"files","port":4000}"#;
        let config: ServerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.name, "files");
        assert_eq!(config.port, 4000);
        assert!(!config.default);
        assert!(!config.enable_inspector);
        assert!(config.docker_image.is_none());
        assert!(config.start_commands.is_none());
        assert!(config.env_vars.is_none());
        assert!(config.label.is_none());
    }

    // =========================================================================
    // ServerSnapshot::from_config
    // =========================================================================

    #[test]
    fn snapshot_from_config_copies_fields() {
        let config = ServerConfig {
            name: "preview".to_string(),
            label: Some("Preview".to_string()),
            port: 3000,
            default: true,
            enable_inspector: true,
            ..Default::default()
        };
        let snap = ServerSnapshot::from_config(&config, ServerStatus::Stopped);
        assert_eq!(snap.name, "preview");
        assert_eq!(snap.label, "Preview");
        assert_eq!(snap.port, 3000);
        assert!(snap.default);
        assert!(snap.enable_inspector);
        assert_eq!(snap.status, ServerStatus::Stopped);
    }

    // =========================================================================
    // Capability functions
    // =========================================================================

    #[test]
    fn capabilities_with_inspector_enabled() {
        let config = ServerConfig {
            name: "preview".to_string(),
            enable_inspector: true,
            ..Default::default()
        };
        let caps = server_capabilities_from_config(&config);
        assert!(caps.inspector);
        assert!(caps.address_bar);
        assert!(caps.reload);
        assert!(caps.viewport);
        assert!(caps.console);
    }

    #[test]
    fn capabilities_with_inspector_disabled() {
        let config = ServerConfig {
            name: "files".to_string(),
            enable_inspector: false,
            ..Default::default()
        };
        let caps = server_capabilities_from_config(&config);
        assert!(!caps.inspector);
        assert!(!caps.address_bar);
        assert!(caps.reload);
        assert!(caps.viewport);
        assert!(caps.console);
    }

    #[test]
    fn capabilities_from_snapshot_matches_config() {
        let snapshot = ServerSnapshot {
            name: "test".to_string(),
            label: "Test".to_string(),
            port: 3000,
            builtin: false,
            default: false,
            enable_inspector: true,
            status: ServerStatus::Started,
            error: None,
            pid: None,
        };
        let caps = server_capabilities_from_snapshot(&snapshot);
        assert!(caps.inspector);
        assert!(caps.address_bar);
    }

    // =========================================================================
    // AgentState serde + From
    // =========================================================================

    #[test]
    fn agent_state_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&AgentState::Idle).unwrap(), "\"idle\"");
        assert_eq!(serde_json::to_string(&AgentState::Running).unwrap(), "\"running\"");
        assert_eq!(serde_json::to_string(&AgentState::None).unwrap(), "\"none\"");
    }

    #[test]
    fn agent_state_from_str() {
        assert!(matches!(AgentState::from("running"), AgentState::Running));
        assert!(matches!(AgentState::from("idle"), AgentState::Idle));
        assert!(matches!(AgentState::from("unknown"), AgentState::None));
        assert!(matches!(AgentState::from(""), AgentState::None));
    }

    // =========================================================================
    // ServerStatus serde + From
    // =========================================================================

    #[test]
    fn server_status_enum_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&ServerStatus::Stopped).unwrap(), "\"stopped\"");
        assert_eq!(serde_json::to_string(&ServerStatus::Starting).unwrap(), "\"starting\"");
        assert_eq!(serde_json::to_string(&ServerStatus::Started).unwrap(), "\"started\"");
        assert_eq!(serde_json::to_string(&ServerStatus::Failed).unwrap(), "\"failed\"");
        assert_eq!(serde_json::to_string(&ServerStatus::Installing).unwrap(), "\"installing\"");
    }

    #[test]
    fn server_status_enum_from_str() {
        assert!(matches!(ServerStatus::from("stopped"), ServerStatus::Stopped));
        assert!(matches!(ServerStatus::from("installing"), ServerStatus::Installing));
        assert!(matches!(ServerStatus::from("starting"), ServerStatus::Starting));
        assert!(matches!(ServerStatus::from("started"), ServerStatus::Started));
        assert!(matches!(ServerStatus::from("failed"), ServerStatus::Failed));
        assert!(matches!(ServerStatus::from("unknown"), ServerStatus::Stopped));
    }

    // =========================================================================
    // EnvironmentStatus Display
    // =========================================================================

    #[test]
    fn environment_status_display() {
        assert_eq!(EnvironmentStatus::NotCreated.to_string(), "not_created");
        assert_eq!(EnvironmentStatus::Initializing.to_string(), "initializing");
        assert_eq!(EnvironmentStatus::Ready.to_string(), "ready");
        assert_eq!(EnvironmentStatus::Stopped.to_string(), "stopped");
        assert_eq!(EnvironmentStatus::Removing.to_string(), "removing");
        assert_eq!(EnvironmentStatus::Error.to_string(), "error");
    }

    #[test]
    fn environment_status_serde_roundtrip() {
        let json = serde_json::to_string(&EnvironmentStatus::Ready).unwrap();
        assert_eq!(json, "\"ready\"");
        let back: EnvironmentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, EnvironmentStatus::Ready);
    }

    // =========================================================================
    // PodPhase::from(&str) and as_str
    // =========================================================================

    #[test]
    fn pod_phase_from_str_known() {
        assert_eq!(PodPhase::from("Pending"), PodPhase::Pending);
        assert_eq!(PodPhase::from("Running"), PodPhase::Running);
        assert_eq!(PodPhase::from("Succeeded"), PodPhase::Succeeded);
        assert_eq!(PodPhase::from("Failed"), PodPhase::Failed);
        assert_eq!(PodPhase::from("Terminating"), PodPhase::Terminating);
    }

    #[test]
    fn pod_phase_from_str_unknown() {
        assert_eq!(PodPhase::from("SomethingElse"), PodPhase::Unknown);
    }

    #[test]
    fn pod_phase_as_str() {
        assert_eq!(PodPhase::Pending.as_str(), "Pending");
        assert_eq!(PodPhase::Running.as_str(), "Running");
        assert_eq!(PodPhase::NotFound.as_str(), "NotFound");
        assert_eq!(PodPhase::Stopped.as_str(), "Stopped");
    }

    // =========================================================================
    // ContainerObservedStatus helper methods
    // =========================================================================

    #[test]
    fn error_reason_returns_failure_reasons_only() {
        let cs = ContainerObservedStatus::test(
            "test", false, false, ContainerState::Waiting, 0, Some("ErrImagePull"),
        );
        assert_eq!(cs.error_reason().as_deref(), Some("ErrImagePull"));

        let cs2 = ContainerObservedStatus::test(
            "test", false, false, ContainerState::Waiting, 0, Some("ContainerCreating"),
        );
        assert!(cs2.error_reason().is_none());
    }

    #[test]
    fn detail_reason_returns_non_failure_reasons() {
        let cs = ContainerObservedStatus::test(
            "test", false, false, ContainerState::Waiting, 0, Some("ContainerCreating"),
        );
        assert_eq!(cs.detail_reason().as_deref(), Some("ContainerCreating"));

        let cs2 = ContainerObservedStatus::test(
            "test", false, false, ContainerState::Waiting, 0, Some("CrashLoopBackOff"),
        );
        assert!(cs2.detail_reason().is_none());
    }

    #[test]
    fn error_reason_none_when_no_waiting_reason() {
        let cs = ContainerObservedStatus::test(
            "test", false, false, ContainerState::Running, 0, None,
        );
        assert!(cs.error_reason().is_none());
        assert!(cs.detail_reason().is_none());
    }

    // =========================================================================
    // WorkspaceState serde
    // =========================================================================

    #[test]
    fn workspace_state_serde() {
        assert_eq!(serde_json::to_string(&WorkspaceState::Exists).unwrap(), "\"exists\"");
        assert_eq!(serde_json::to_string(&WorkspaceState::Missing).unwrap(), "\"missing\"");
    }

    // =========================================================================
    // ProvisionRequest deserialization
    // =========================================================================

    #[test]
    fn provision_request_deserializes() {
        let json = r#"{
            "plan": {
                "repoConfig": {
                    "name": "test-repo"
                },
                "gitCloneUrl": "https://github.com/test/repo.git"
            },
            "userId": "user-1"
        }"#;
        let req: super::ProvisionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.user_id, "user-1");
        assert_eq!(req.plan.repo_config.name, "test-repo");
        assert_eq!(req.plan.git_clone_url, "https://github.com/test/repo.git");
        assert!(req.plan.git_branch.is_none());
    }
}
