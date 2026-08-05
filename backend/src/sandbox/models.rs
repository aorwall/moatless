//! Sandbox domain types — config, provisioning, and K8s status types.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use utoipa::ToSchema;

pub use moatless_api_schemas::sandbox::{
    derive_mount_name, sanitize_mount_name, RepoCheckout, WORKSPACE_REPOS_ENV,
};

// Keep pod-status extraction available from the sandbox model surface.
pub use crate::sandbox::k8s::pod_status::extract_container_observed_statuses;
use crate::repository::models::ServerConfig;

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
    /// The primary repository's clone URL. Mirrors `repos[0]`, which is always
    /// the primary checkout; kept as its own field for the `GIT_CLONE_URL` env.
    pub git_clone_url: String,
    /// The primary repository's branch. Mirrors `repos[0].branch`.
    pub git_branch: Option<String>,
    /// The primary repository's git provider (e.g. "github", "gitlab", "gitness")
    #[serde(default = "default_git_provider")]
    pub git_provider: String,
    /// Custom environment variables
    pub env_vars: Option<HashMap<String, String>>,
    /// Every repository the task's workspace places in the sandbox, primary
    /// first. Empty only when `git_clone_url` is the sole checkout.
    #[serde(default)]
    pub repos: Vec<RepoCheckout>,
}

fn default_git_provider() -> String {
    "github".to_string()
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
    // --- Pass-through fields for REPO_CONFIG env var ---
    /// System prompt for the agent
    pub system_prompt: Option<String>,
    /// Code execution configuration (dockerImage, envVars, etc.)
    #[schema(value_type = Object)]
    pub code_execution: Option<serde_json::Value>,
    /// Preview server configuration
    #[schema(value_type = Object)]
    pub preview_config: Option<serde_json::Value>,
    // --- K8s resource configuration (opt-in per-repo) ---
    /// Per-repo K8s resource requests/limits override for server containers.
    pub resources: Option<crate::repository::models::ResourceConfig>,
    // --- Plugins (resolved from the task owner's activation records) ---
    /// The plugins this task's owner has turned on, each with the skills they
    /// picked. The sandbox clones each one and removes the skills not named.
    pub plugins: Option<Vec<moatless_api_schemas::plugin::PluginDelivery>>,
    // --- Docker access (opt-in) ---
    /// Mount Docker socket inside sandbox container (default: false).
    /// Overrides global DOCKER_ENABLE_SOCKET_MOUNT when set.
    #[serde(default)]
    pub docker_access: Option<bool>,
    /// Additional Docker networks to connect sandbox to.
    /// Overrides global DOCKER_EXTRA_NETWORKS when set.
    #[serde(default)]
    pub docker_networks: Option<Vec<String>>,
    // --- Kubernetes ServiceAccount (opt-in per-repo) ---
    /// Per-repo Kubernetes ServiceAccount for the sandbox pod. Overrides the
    /// global default (`K8S_SANDBOX_SERVICE_ACCOUNT`) when set. Ignored in Docker
    /// mode. Enables "shadow" dev sandboxes that run under an SA bound to the
    /// backend's RBAC.
    #[serde(default)]
    pub sandbox_service_account: Option<String>,
    // --- Network profile (opt-in per-repo) ---
    /// Per-repo network profile. When set, the sandbox pod carries the label
    /// `moatless.io/network-profile=<value>`, which a static (GitOps-owned)
    /// NetworkPolicy can select to govern the pod's egress/ingress. Selects a
    /// policy only; grants nothing on its own. Ignored in Docker mode.
    #[serde(default)]
    pub network_profile: Option<String>,
}

/// A live override of one preview server's start command and env.
///
/// Whole-state, not a delta: callers always send the complete set to the
/// sandbox, which never merges.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerOverride {
    /// Replaces the server's configured start command. `None` = use the
    /// workspace's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_command: Option<String>,
    /// Env entries exported after `server.env`, so they win over it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Keys to remove — the only way to drop a variable the container spec
    /// injected, since sourcing an env file can add but never remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unset_env: Vec<String>,
}

impl ServerOverride {
    /// True when nothing is overridden, so the entry can be dropped from the
    /// persisted map rather than stored empty.
    pub fn is_empty(&self) -> bool {
        self.start_command.is_none() && self.env.is_empty() && self.unset_env.is_empty()
    }
}

pub use super::live_status::SandboxStatus;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Project this K8s runtime phase into the public Sandbox display/API state.
    pub fn sandbox_status(self) -> SandboxStatus {
        match self {
            PodPhase::Pending => SandboxStatus::Initializing,
            PodPhase::Running => SandboxStatus::Ready,
            PodPhase::Succeeded | PodPhase::Stopped => SandboxStatus::Stopped,
            PodPhase::Terminating => SandboxStatus::Removing,
            PodPhase::NotFound => SandboxStatus::NotCreated,
            PodPhase::Failed | PodPhase::Unknown => SandboxStatus::Error,
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
    /// Reason from `state.terminated` (e.g. "OOMKilled", "Error", "Completed").
    pub terminated_reason: Option<String>,
    /// Exit code from `state.terminated`.
    pub exit_code: Option<i32>,
    /// Reason from `last_state.terminated` (after a restart).
    pub last_terminated_reason: Option<String>,
    /// Exit code from `last_state.terminated`.
    pub last_exit_code: Option<i32>,
    /// When the container entered Running state (from K8s `started_at`).
    pub running_since: Option<chrono::DateTime<chrono::Utc>>,
}

impl ContainerObservedStatus {
    /// Map container probes to a server status for the frontend.
    pub fn to_server_status(&self) -> ServerStatus {
        use crate::sandbox::k8s::pod_status::SERVER_FAILURE_REASONS;

        // Terminated containers: OOMKilled or non-zero exit → Failed, else Stopped.
        if matches!(self.state, ContainerState::Terminated) {
            if self.terminated_reason.as_deref() == Some("OOMKilled") {
                return ServerStatus::Failed;
            }
            if self.exit_code.is_some_and(|c| c != 0)
                && self.terminated_reason.as_deref() != Some("Completed")
            {
                return ServerStatus::Failed;
            }
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

    /// Like `to_server_status()` but with a configurable readiness timeout.
    /// If the container has been running for longer than `timeout_secs` without
    /// becoming ready, returns `Failed` instead of `Starting`.
    pub fn to_server_status_with_timeout(&self, timeout_secs: u32) -> ServerStatus {
        let base = self.to_server_status();
        if base == ServerStatus::Starting {
            if let Some(since) = self.running_since {
                let elapsed = chrono::Utc::now() - since;
                if elapsed > chrono::Duration::seconds(timeout_secs as i64) {
                    return ServerStatus::Failed;
                }
            }
        }
        base
    }

    /// Return the error reason — terminated reasons take priority, then waiting reasons.
    pub fn error_reason(&self) -> Option<String> {
        use crate::sandbox::k8s::pod_status::SERVER_FAILURE_REASONS;

        // Terminated reason (e.g. OOMKilled, Error)
        if let Some(reason) = &self.terminated_reason {
            if reason != "Completed" {
                return Some(reason.clone());
            }
        }
        // Post-restart context from last_state
        if let Some(reason) = &self.last_terminated_reason {
            if reason == "OOMKilled" {
                return Some(format!("Restarted after {reason}"));
            }
        }
        // Waiting reason
        self.waiting_reason.as_deref().and_then(|r| {
            if SERVER_FAILURE_REASONS.contains(&r) {
                Some(r.to_string())
            } else {
                None
            }
        })
    }

    /// Like `error_reason()` but also reports readiness timeout failures.
    pub fn error_reason_with_timeout(&self, timeout_secs: u32) -> Option<String> {
        if let Some(reason) = self.error_reason() {
            return Some(reason);
        }
        // Check for readiness timeout
        if self.to_server_status() == ServerStatus::Starting {
            if let Some(since) = self.running_since {
                let elapsed = chrono::Utc::now() - since;
                if elapsed > chrono::Duration::seconds(timeout_secs as i64) {
                    return Some("Server failed to become ready within timeout".to_string());
                }
            }
        }
        None
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
            terminated_reason: None,
            exit_code: None,
            last_terminated_reason: None,
            last_exit_code: None,
            running_since: None,
        }
    }

    fn test_terminated(
        name: &str,
        terminated_reason: Option<&str>,
        exit_code: Option<i32>,
    ) -> Self {
        Self {
            name: name.to_string(),
            ready: false,
            started: false,
            state: ContainerState::Terminated,
            restart_count: 0,
            waiting_reason: None,
            terminated_reason: terminated_reason.map(String::from),
            exit_code,
            last_terminated_reason: None,
            last_exit_code: None,
            running_since: None,
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
    // PodPhase::sandbox_status()
    // =========================================================================

    #[test]
    fn pod_phase_projects_to_sandbox_status() {
        let cases = [
            (PodPhase::Pending, SandboxStatus::Initializing),
            (PodPhase::Running, SandboxStatus::Ready),
            (PodPhase::Succeeded, SandboxStatus::Stopped),
            (PodPhase::Failed, SandboxStatus::Error),
            (PodPhase::Unknown, SandboxStatus::Error),
            (PodPhase::Terminating, SandboxStatus::Removing),
            (PodPhase::NotFound, SandboxStatus::NotCreated),
            (PodPhase::Stopped, SandboxStatus::Stopped),
        ];

        for (phase, expected) in cases {
            assert_eq!(phase.sandbox_status(), expected, "{phase:?}");
        }
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
    // ServerStatus serde + From
    // =========================================================================

    #[test]
    fn server_status_enum_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&ServerStatus::Stopped).unwrap(),
            "\"stopped\""
        );
        assert_eq!(
            serde_json::to_string(&ServerStatus::Starting).unwrap(),
            "\"starting\""
        );
        assert_eq!(
            serde_json::to_string(&ServerStatus::Started).unwrap(),
            "\"started\""
        );
        assert_eq!(
            serde_json::to_string(&ServerStatus::Failed).unwrap(),
            "\"failed\""
        );
        assert_eq!(
            serde_json::to_string(&ServerStatus::Installing).unwrap(),
            "\"installing\""
        );
    }

    #[test]
    fn server_status_enum_from_str() {
        assert!(matches!(
            ServerStatus::from("stopped"),
            ServerStatus::Stopped
        ));
        assert!(matches!(
            ServerStatus::from("installing"),
            ServerStatus::Installing
        ));
        assert!(matches!(
            ServerStatus::from("starting"),
            ServerStatus::Starting
        ));
        assert!(matches!(
            ServerStatus::from("started"),
            ServerStatus::Started
        ));
        assert!(matches!(ServerStatus::from("failed"), ServerStatus::Failed));
        assert!(matches!(
            ServerStatus::from("unknown"),
            ServerStatus::Stopped
        ));
    }

    // =========================================================================
    // SandboxStatus Display
    // =========================================================================

    #[test]
    fn sandbox_status_display() {
        assert_eq!(SandboxStatus::NotCreated.to_string(), "not_created");
        assert_eq!(SandboxStatus::Initializing.to_string(), "initializing");
        assert_eq!(SandboxStatus::Ready.to_string(), "ready");
        assert_eq!(SandboxStatus::Stopped.to_string(), "stopped");
        assert_eq!(SandboxStatus::Removing.to_string(), "removing");
        assert_eq!(SandboxStatus::Error.to_string(), "error");
    }

    #[test]
    fn sandbox_status_serde_roundtrip() {
        let json = serde_json::to_string(&SandboxStatus::Ready).unwrap();
        assert_eq!(json, "\"ready\"");
        let back: SandboxStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SandboxStatus::Ready);
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
            "test",
            false,
            false,
            ContainerState::Waiting,
            0,
            Some("ErrImagePull"),
        );
        assert_eq!(cs.error_reason().as_deref(), Some("ErrImagePull"));

        let cs2 = ContainerObservedStatus::test(
            "test",
            false,
            false,
            ContainerState::Waiting,
            0,
            Some("ContainerCreating"),
        );
        assert!(cs2.error_reason().is_none());
    }

    #[test]
    fn detail_reason_returns_non_failure_reasons() {
        let cs = ContainerObservedStatus::test(
            "test",
            false,
            false,
            ContainerState::Waiting,
            0,
            Some("ContainerCreating"),
        );
        assert_eq!(cs.detail_reason().as_deref(), Some("ContainerCreating"));

        let cs2 = ContainerObservedStatus::test(
            "test",
            false,
            false,
            ContainerState::Waiting,
            0,
            Some("CrashLoopBackOff"),
        );
        assert!(cs2.detail_reason().is_none());
    }

    #[test]
    fn error_reason_none_when_no_waiting_reason() {
        let cs =
            ContainerObservedStatus::test("test", false, false, ContainerState::Running, 0, None);
        assert!(cs.error_reason().is_none());
        assert!(cs.detail_reason().is_none());
    }

    #[test]
    fn error_reason_returns_terminated_reason() {
        let cs = ContainerObservedStatus::test_terminated("test", Some("OOMKilled"), Some(137));
        assert_eq!(cs.error_reason().as_deref(), Some("OOMKilled"));
    }

    #[test]
    fn error_reason_skips_completed_terminated() {
        let cs = ContainerObservedStatus::test_terminated("test", Some("Completed"), Some(0));
        assert!(cs.error_reason().is_none());
    }

    #[test]
    fn error_reason_returns_last_state_oom() {
        let mut cs =
            ContainerObservedStatus::test("test", false, false, ContainerState::Running, 1, None);
        cs.last_terminated_reason = Some("OOMKilled".to_string());
        assert_eq!(
            cs.error_reason().as_deref(),
            Some("Restarted after OOMKilled")
        );
    }

    // =========================================================================
    // ContainerObservedStatus::to_server_status() — termination variants
    // =========================================================================

    #[test]
    fn server_status_terminated_oom_killed_is_failed() {
        let cs = ContainerObservedStatus::test_terminated(
            "preview-server",
            Some("OOMKilled"),
            Some(137),
        );
        assert_eq!(cs.to_server_status(), ServerStatus::Failed);
    }

    #[test]
    fn server_status_terminated_nonzero_exit_is_failed() {
        let cs = ContainerObservedStatus::test_terminated("preview-server", Some("Error"), Some(1));
        assert_eq!(cs.to_server_status(), ServerStatus::Failed);
    }

    #[test]
    fn server_status_terminated_completed_is_stopped() {
        let cs =
            ContainerObservedStatus::test_terminated("preview-server", Some("Completed"), Some(0));
        assert_eq!(cs.to_server_status(), ServerStatus::Stopped);
    }

    #[test]
    fn server_status_terminated_zero_exit_no_reason_is_stopped() {
        let cs = ContainerObservedStatus::test_terminated("preview-server", None, Some(0));
        assert_eq!(cs.to_server_status(), ServerStatus::Stopped);
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

    // =========================================================================
    // to_server_status_with_timeout()
    // =========================================================================

    #[test]
    fn timeout_running_past_deadline_is_failed() {
        let mut cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            true, // started but not ready → Starting
            ContainerState::Running,
            0,
            None,
        );
        cs.running_since = Some(chrono::Utc::now() - chrono::Duration::seconds(150));
        assert_eq!(cs.to_server_status_with_timeout(120), ServerStatus::Failed);
    }

    #[test]
    fn timeout_running_within_deadline_is_starting() {
        let mut cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            true,
            ContainerState::Running,
            0,
            None,
        );
        cs.running_since = Some(chrono::Utc::now() - chrono::Duration::seconds(10));
        assert_eq!(
            cs.to_server_status_with_timeout(120),
            ServerStatus::Starting
        );
    }

    #[test]
    fn timeout_ready_container_unaffected() {
        let mut cs = ContainerObservedStatus::test(
            "preview-server",
            true,
            true,
            ContainerState::Running,
            0,
            None,
        );
        cs.running_since = Some(chrono::Utc::now() - chrono::Duration::seconds(150));
        assert_eq!(cs.to_server_status_with_timeout(120), ServerStatus::Started);
    }

    #[test]
    fn timeout_no_running_since_stays_starting() {
        let cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            true,
            ContainerState::Running,
            0,
            None,
        );
        // running_since is None by default
        assert_eq!(
            cs.to_server_status_with_timeout(120),
            ServerStatus::Starting
        );
    }

    #[test]
    fn error_reason_with_timeout_reports_readiness_timeout() {
        let mut cs = ContainerObservedStatus::test(
            "preview-server",
            false,
            true,
            ContainerState::Running,
            0,
            None,
        );
        cs.running_since = Some(chrono::Utc::now() - chrono::Duration::seconds(150));
        assert!(cs.error_reason().is_none()); // base has no error
        assert!(cs.error_reason_with_timeout(120).is_some());
    }
}
