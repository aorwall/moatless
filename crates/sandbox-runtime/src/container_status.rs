//! Container and pod observation, projected to server and sandbox status.
//!
//! The status resolvers read a container's probes ([`ContainerObservedStatus`])
//! and a pod's phase ([`PodPhase`]) and turn them into the
//! `ServerStatus`/`SandboxStatus` the surface shows;
//! [`ContainerObservedStatus::to_server_status`] is the seam. Pure data and
//! projection, unit-tested without a runtime.

use serde::{Deserialize, Serialize};

use moatless_types::sandbox::{SandboxStatus, ServerCapabilities, ServerStatus};

/// Waiting reasons that map a *server* container to `"failed"` status, read by
/// [`ContainerObservedStatus::to_server_status`]. Both the k8s and docker
/// status resolvers reach it here.
pub const SERVER_FAILURE_REASONS: &[&str] = &[
    "ErrImagePull",
    "ImagePullBackOff",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CrashLoopBackOff",
];

/// Default readiness grace period for the pod watcher, used when no per-server
/// timeout is configured.
pub const DEFAULT_SERVER_READINESS_TIMEOUT_SECS: u32 = 120;

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

    /// Project this K8s runtime phase into the public Sandbox display/API
    /// state.
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

pub use moatless_api_schemas::sandbox::ServerSnapshot;

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
    /// Reason from `state.terminated` (e.g. "`OOMKilled`", "Error",
    /// "Completed").
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

    /// Like `to_server_status()` but with a configurable readiness timeout. If
    /// the container has been running for longer than `timeout_secs` without
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

    /// Return the error reason — terminated reasons take priority, then waiting
    /// reasons.
    pub fn error_reason(&self) -> Option<String> {
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

    /// Return the waiting reason only if it is a non-error detail (e.g.
    /// `ContainerCreating`).
    pub fn detail_reason(&self) -> Option<String> {
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
    // Capability functions
    // =========================================================================

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
            detail: None,
            pid: None,
        };
        let caps = server_capabilities_from_snapshot(&snapshot);
        assert!(caps.inspector);
        assert!(caps.address_bar);
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
