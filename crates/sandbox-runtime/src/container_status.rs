//! Container and pod observation, projected to sandbox status.
//!
//! The status resolvers read a pod's phase ([`PodPhase`]) and its containers'
//! probes ([`ContainerObservedStatus`]) and turn them into the `SandboxStatus`
//! the surface shows. Pure data and projection, unit-tested without a runtime.

use serde::{Deserialize, Serialize};

use moatless_types::sandbox::SandboxStatus;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
