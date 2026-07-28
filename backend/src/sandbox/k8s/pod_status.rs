//! Pure, synchronous pod inspection functions and shared failure-reason constants.
//!
//! Everything in this module operates on `&Pod` (or `&[Pod]`) without
//! making any async calls.  This is the single source of truth for:
//!
//! * sandbox container readiness checks
//! * permanent failure detection
//! * container waiting-reason extraction
//! * multi-pod ranking ("pick best pod")
//! * container observed-status extraction (for server monitoring)
//! * `SandboxStatus` ↔ `PodPhase` conversion

use k8s_openapi::api::core::v1::Pod;

use crate::sandbox::models::{ContainerObservedStatus, ContainerState, PodPhase, SandboxStatus};

// =============================================================================
// Shared failure-reason constants
// =============================================================================

/// Waiting reasons that indicate a **permanent** failure of the *sandbox*
/// container.  Used by [`check_sandbox_permanent_failure`].
pub const SANDBOX_PERMANENT_FAILURE_REASONS: &[&str] = &[
    "ImagePullBackOff",
    "ErrImagePull",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CrashLoopBackOff",
];

/// Waiting reasons that map a *server* container to `"failed"` status.
/// Used by `ContainerObservedStatus::to_server_status()` in `types.rs`.
pub const SERVER_FAILURE_REASONS: &[&str] = &[
    "ErrImagePull",
    "ImagePullBackOff",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CrashLoopBackOff",
];

// =============================================================================
// Pod inspection functions
// =============================================================================

/// Check if the sandbox container in a pod is ready (started + ready probes passed).
///
/// We check the sandbox container specifically rather than the pod-level Ready
/// condition because server containers (files-server, preview-server) may have
/// image pull issues that block the pod-level Ready condition even though the
/// sandbox is fine.
pub fn is_sandbox_container_ready(pod: &Pod) -> bool {
    pod.status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref())
        .and_then(|statuses| statuses.iter().find(|cs| cs.name == "sandbox"))
        .is_some_and(|cs| cs.started.unwrap_or(false) && cs.ready)
}

/// Check for permanent container failures in the **sandbox** container.
///
/// Only checks the `sandbox` container — server containers (files-server,
/// preview-server) may have image pull issues that shouldn't block the
/// main sandbox readiness.
///
/// Returns `Some(message)` describing the failure, or `None` if no permanent
/// failure is detected.
pub fn check_sandbox_permanent_failure(pod: &Pod) -> Option<String> {
    let container_statuses = pod.status.as_ref()?.container_statuses.as_ref()?;
    for cs in container_statuses {
        if cs.name != "sandbox" {
            continue;
        }
        if let Some(waiting) = cs.state.as_ref().and_then(|s| s.waiting.as_ref()) {
            let reason = waiting.reason.as_deref().unwrap_or("");
            if SANDBOX_PERMANENT_FAILURE_REASONS.contains(&reason) {
                // Try to extract termination message from last_state.terminated.message
                // (written to /dev/termination-log by the sandbox on init failure)
                let detail = cs
                    .last_state
                    .as_ref()
                    .and_then(|s| s.terminated.as_ref())
                    .and_then(|t| t.message.as_deref())
                    .filter(|m| !m.is_empty());

                return match detail {
                    Some(msg) => Some(format!("Sandbox failed: {msg}")),
                    None => Some(format!("Container error: {reason}")),
                };
            }
        }
    }
    None
}

/// Extract the first container's waiting reason, if any.
pub fn extract_first_waiting_reason(pod: &Pod) -> Option<String> {
    pod.status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref())
        .and_then(|cs| cs.first())
        .and_then(|cs| cs.state.as_ref())
        .and_then(|s| s.waiting.as_ref())
        .and_then(|w| w.reason.clone())
}

/// Pick the best pod from a list when multiple pods exist for a task.
///
/// During Deployment rolling updates, both old (terminating) and new pods may
/// coexist.  We prefer the non-terminating pod so that status reflects the
/// replacement rather than the dying pod.
///
/// Priority among non-terminating pods: Running > Pending > Unknown > everything else.
/// If ALL pods are terminating, return the first one (genuinely removing).
pub fn pick_best_pod(pods: &[Pod]) -> Option<&Pod> {
    if pods.is_empty() {
        return None;
    }

    // Split into non-terminating and terminating
    let (alive, terminating): (Vec<_>, Vec<_>) = pods
        .iter()
        .partition(|p| p.metadata.deletion_timestamp.is_none());

    if alive.is_empty() {
        // All pods are terminating — return first (genuinely removing)
        return pods.first();
    }

    // Among alive pods, pick by phase priority
    fn phase_priority(pod: &Pod) -> u8 {
        match pod
            .status
            .as_ref()
            .and_then(|s| s.phase.as_deref())
            .unwrap_or("Unknown")
        {
            "Running" => 0,
            "Pending" => 1,
            "Unknown" => 2,
            _ => 3,
        }
    }

    alive
        .into_iter()
        .min_by_key(|p| phase_priority(p))
        .or(terminating.first().copied())
}

/// Default readiness grace period for pod watcher (no per-server config available).
pub const DEFAULT_SERVER_READINESS_TIMEOUT_SECS: u32 = 120;

/// Extract observed statuses for **all** containers in a K8s Pod.
///
/// Unlike [`extract_container_observed_statuses`], this includes the `sandbox`
/// container. Used for diagnostics/troubleshooting.
pub fn extract_all_container_statuses(pod: &Pod) -> Vec<ContainerObservedStatus> {
    extract_container_statuses_filtered(pod, false)
}

/// Extract container observed statuses from a K8s Pod.
///
/// Excludes the `sandbox` container — this is used for monitoring *server*
/// containers only (files-server, preview-server, etc.).
pub fn extract_container_observed_statuses(pod: &Pod) -> Vec<ContainerObservedStatus> {
    extract_container_statuses_filtered(pod, true)
}

/// Shared implementation for container status extraction.
///
/// When `exclude_sandbox` is `true`, the `sandbox` container is filtered out.
fn extract_container_statuses_filtered(
    pod: &Pod,
    exclude_sandbox: bool,
) -> Vec<ContainerObservedStatus> {
    let container_statuses = match pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref())
    {
        Some(cs) => cs,
        None => return vec![],
    };

    container_statuses
        .iter()
        .filter(|cs| !exclude_sandbox || cs.name != "sandbox")
        .map(|cs| {
            let (state, waiting_reason) = match cs.state.as_ref() {
                Some(s) if s.running.is_some() => (ContainerState::Running, None),
                Some(s) if s.waiting.is_some() => {
                    let reason = s.waiting.as_ref().and_then(|w| w.reason.clone());
                    (ContainerState::Waiting, reason)
                }
                Some(s) if s.terminated.is_some() => (ContainerState::Terminated, None),
                _ => (ContainerState::Unknown, None),
            };

            let terminated_reason = cs
                .state
                .as_ref()
                .and_then(|s| s.terminated.as_ref())
                .and_then(|t| t.reason.clone());

            let exit_code = cs
                .state
                .as_ref()
                .and_then(|s| s.terminated.as_ref())
                .map(|t| t.exit_code);

            let last_terminated_reason = cs
                .last_state
                .as_ref()
                .and_then(|s| s.terminated.as_ref())
                .and_then(|t| t.reason.clone());

            let last_exit_code = cs
                .last_state
                .as_ref()
                .and_then(|s| s.terminated.as_ref())
                .map(|t| t.exit_code);

            let running_since = cs
                .state
                .as_ref()
                .and_then(|s| s.running.as_ref())
                .and_then(|r| r.started_at.as_ref())
                .and_then(|t| {
                    chrono::DateTime::from_timestamp(
                        t.0.as_second(),
                        t.0.subsec_nanosecond() as u32,
                    )
                });

            ContainerObservedStatus {
                name: cs.name.clone(),
                ready: cs.ready,
                started: cs.started.unwrap_or(false),
                state,
                restart_count: cs.restart_count,
                waiting_reason,
                terminated_reason,
                exit_code,
                last_terminated_reason,
                last_exit_code,
                running_since,
            }
        })
        .collect()
}

/// Convert `SandboxStatus` back to an approximate `PodPhase` for L1 cache compatibility.
pub fn sandbox_status_to_pod_phase(status: &SandboxStatus) -> PodPhase {
    match status {
        SandboxStatus::NotCreated => PodPhase::NotFound,
        SandboxStatus::Initializing => PodPhase::Pending,
        SandboxStatus::Ready => PodPhase::Running,
        SandboxStatus::Stopped => PodPhase::Stopped,
        SandboxStatus::Removing => PodPhase::Terminating,
        SandboxStatus::Removed => PodPhase::NotFound,
        SandboxStatus::Error => PodPhase::Failed,
    }
}

// =============================================================================
// Test helpers — shared across sibling modules
// =============================================================================

#[cfg(test)]
pub mod test_helpers {
    use k8s_openapi::api::core::v1::{
        ContainerState as K8sState, ContainerStateRunning, ContainerStateTerminated,
        ContainerStateWaiting, ContainerStatus, Pod, PodStatus,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    pub fn running_container(name: &str, started: bool, ready: bool) -> ContainerStatus {
        ContainerStatus {
            name: name.to_string(),
            ready,
            started: Some(started),
            restart_count: 0,
            state: Some(K8sState {
                running: Some(ContainerStateRunning { started_at: None }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    pub fn waiting_container(name: &str, reason: &str) -> ContainerStatus {
        ContainerStatus {
            name: name.to_string(),
            ready: false,
            started: Some(false),
            restart_count: 0,
            state: Some(K8sState {
                waiting: Some(ContainerStateWaiting {
                    reason: Some(reason.to_string()),
                    message: None,
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    pub fn pod_with_containers(statuses: Vec<ContainerStatus>) -> Pod {
        Pod {
            status: Some(PodStatus {
                container_statuses: Some(statuses),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    pub fn terminated_container(name: &str, reason: &str, exit_code: i32) -> ContainerStatus {
        ContainerStatus {
            name: name.to_string(),
            ready: false,
            started: Some(false),
            restart_count: 0,
            state: Some(K8sState {
                terminated: Some(ContainerStateTerminated {
                    reason: Some(reason.to_string()),
                    exit_code,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    pub fn pod_with_phase_and_containers(phase: &str, containers: Vec<ContainerStatus>) -> Pod {
        Pod {
            metadata: ObjectMeta::default(),
            status: Some(PodStatus {
                phase: Some(phase.to_string()),
                container_statuses: Some(containers),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::test_helpers::*;
    use super::*;
    use crate::sandbox::models::ServerStatus;
    use k8s_openapi::api::core::v1::{
        ContainerState as K8sState, ContainerStateRunning, ContainerStateWaiting, ContainerStatus,
        Pod, PodStatus,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    // =========================================================================
    // is_sandbox_container_ready()
    // =========================================================================

    #[test]
    fn sandbox_ready_when_started_and_ready() {
        let pod = pod_with_phase_and_containers(
            "Running",
            vec![
                running_container("sandbox", true, true),
                waiting_container("files-server", "ErrImagePull"),
            ],
        );
        assert!(is_sandbox_container_ready(&pod));
    }

    #[test]
    fn sandbox_not_ready_when_only_started() {
        let pod = pod_with_phase_and_containers(
            "Running",
            vec![running_container("sandbox", true, false)],
        );
        assert!(!is_sandbox_container_ready(&pod));
    }

    #[test]
    fn sandbox_not_ready_when_not_started() {
        let pod = pod_with_containers(vec![running_container("sandbox", false, false)]);
        assert!(!is_sandbox_container_ready(&pod));
    }

    #[test]
    fn sandbox_not_ready_when_no_sandbox_container() {
        let pod = pod_with_containers(vec![running_container("files-server", true, true)]);
        assert!(!is_sandbox_container_ready(&pod));
    }

    #[test]
    fn sandbox_not_ready_on_empty_pod() {
        let pod = Pod::default();
        assert!(!is_sandbox_container_ready(&pod));
    }

    #[test]
    fn sandbox_ready_independent_of_server_containers() {
        let pod = pod_with_containers(vec![
            running_container("sandbox", true, true),
            waiting_container("preview-server", "ImagePullBackOff"),
            waiting_container("files-server", "ErrImagePull"),
        ]);
        assert!(is_sandbox_container_ready(&pod));
    }

    // =========================================================================
    // check_sandbox_permanent_failure()
    // =========================================================================

    #[test]
    fn no_permanent_failure_when_sandbox_running() {
        let pod = pod_with_containers(vec![
            running_container("sandbox", true, true),
            waiting_container("files-server", "ErrImagePull"),
        ]);
        assert!(check_sandbox_permanent_failure(&pod).is_none());
    }

    #[test]
    fn permanent_failure_when_sandbox_has_image_pull_error() {
        let pod = pod_with_containers(vec![
            waiting_container("sandbox", "ErrImagePull"),
            running_container("files-server", true, true),
        ]);
        let msg = check_sandbox_permanent_failure(&pod);
        assert!(msg.is_some());
        assert!(msg.unwrap().contains("ErrImagePull"));
    }

    #[test]
    fn permanent_failure_when_sandbox_image_pull_backoff() {
        let pod = pod_with_containers(vec![waiting_container("sandbox", "ImagePullBackOff")]);
        assert!(check_sandbox_permanent_failure(&pod).is_some());
    }

    #[test]
    fn permanent_failure_when_sandbox_invalid_image_name() {
        let pod = pod_with_containers(vec![waiting_container("sandbox", "InvalidImageName")]);
        assert!(check_sandbox_permanent_failure(&pod).is_some());
    }

    #[test]
    fn permanent_failure_when_sandbox_create_container_config_error() {
        let pod = pod_with_containers(vec![waiting_container(
            "sandbox",
            "CreateContainerConfigError",
        )]);
        assert!(check_sandbox_permanent_failure(&pod).is_some());
    }

    #[test]
    fn permanent_failure_when_sandbox_crash_loop_backoff() {
        let pod = pod_with_containers(vec![waiting_container("sandbox", "CrashLoopBackOff")]);
        let msg = check_sandbox_permanent_failure(&pod);
        assert!(msg.is_some());
        assert!(msg.unwrap().contains("CrashLoopBackOff"));
    }

    #[test]
    fn permanent_failure_extracts_termination_message() {
        use k8s_openapi::api::core::v1::{
            ContainerState as K8sState, ContainerStateTerminated, ContainerStateWaiting,
            ContainerStatus,
        };

        let pod = Pod {
            status: Some(PodStatus {
                container_statuses: Some(vec![ContainerStatus {
                    name: "sandbox".to_string(),
                    ready: false,
                    started: Some(false),
                    restart_count: 3,
                    state: Some(K8sState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some("CrashLoopBackOff".to_string()),
                            message: None,
                        }),
                        ..Default::default()
                    }),
                    last_state: Some(K8sState {
                        terminated: Some(ContainerStateTerminated {
                            message: Some("Failed to clone repository: auth error".to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let msg = check_sandbox_permanent_failure(&pod).unwrap();
        assert_eq!(
            msg,
            "Sandbox failed: Failed to clone repository: auth error"
        );
    }

    #[test]
    fn permanent_failure_falls_back_when_no_termination_message() {
        let pod = pod_with_containers(vec![waiting_container("sandbox", "CrashLoopBackOff")]);
        let msg = check_sandbox_permanent_failure(&pod).unwrap();
        assert_eq!(msg, "Container error: CrashLoopBackOff");
    }

    #[test]
    fn no_permanent_failure_when_sandbox_container_creating() {
        let pod = pod_with_containers(vec![waiting_container("sandbox", "ContainerCreating")]);
        assert!(check_sandbox_permanent_failure(&pod).is_none());
    }

    #[test]
    fn no_permanent_failure_ignores_server_container_errors() {
        let pod = pod_with_containers(vec![
            running_container("sandbox", false, false),
            waiting_container("preview-server", "ErrImagePull"),
            waiting_container("files-server", "ImagePullBackOff"),
        ]);
        assert!(check_sandbox_permanent_failure(&pod).is_none());
    }

    #[test]
    fn no_permanent_failure_when_no_statuses() {
        let pod = Pod::default();
        assert!(check_sandbox_permanent_failure(&pod).is_none());
    }

    // =========================================================================
    // extract_first_waiting_reason()
    // =========================================================================

    #[test]
    fn extract_waiting_reason_returns_reason() {
        let pod = pod_with_containers(vec![waiting_container("sandbox", "ContainerCreating")]);
        assert_eq!(
            extract_first_waiting_reason(&pod),
            Some("ContainerCreating".to_string())
        );
    }

    #[test]
    fn extract_waiting_reason_none_when_running() {
        let pod = pod_with_containers(vec![running_container("sandbox", true, true)]);
        assert!(extract_first_waiting_reason(&pod).is_none());
    }

    // =========================================================================
    // pick_best_pod()
    // =========================================================================

    #[test]
    fn pick_best_pod_prefers_running_over_pending() {
        let running = pod_with_phase_and_containers(
            "Running",
            vec![running_container("sandbox", true, true)],
        );
        let pending = pod_with_phase_and_containers(
            "Pending",
            vec![waiting_container("sandbox", "ContainerCreating")],
        );
        let pods = vec![pending, running];
        let best = pick_best_pod(&pods).unwrap();
        let best_phase = best.status.as_ref().unwrap().phase.as_deref().unwrap();
        assert_eq!(best_phase, "Running");
    }

    #[test]
    fn pick_best_pod_skips_terminating() {
        let terminating = Pod {
            metadata: ObjectMeta {
                deletion_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
                    k8s_openapi::jiff::Timestamp::now(),
                )),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let pending = pod_with_phase_and_containers(
            "Pending",
            vec![waiting_container("sandbox", "ContainerCreating")],
        );
        let pods = vec![terminating, pending];
        let best = pick_best_pod(&pods).unwrap();
        assert!(best.metadata.deletion_timestamp.is_none());
    }

    #[test]
    fn pick_best_pod_empty_returns_none() {
        assert!(pick_best_pod(&[]).is_none());
    }

    // =========================================================================
    // extract_container_observed_statuses()
    // =========================================================================

    #[test]
    fn extract_statuses_excludes_sandbox_container() {
        let pod = Pod {
            status: Some(PodStatus {
                container_statuses: Some(vec![
                    ContainerStatus {
                        name: "sandbox".to_string(),
                        ready: true,
                        started: Some(true),
                        restart_count: 0,
                        state: Some(K8sState {
                            running: Some(ContainerStateRunning { started_at: None }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    ContainerStatus {
                        name: "preview-server".to_string(),
                        ready: false,
                        started: Some(false),
                        restart_count: 0,
                        state: Some(K8sState {
                            waiting: Some(ContainerStateWaiting {
                                reason: Some("ErrImagePull".to_string()),
                                message: None,
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let statuses = extract_container_observed_statuses(&pod);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].name, "preview-server");
        assert_eq!(statuses[0].to_server_status(), ServerStatus::Failed);
    }

    #[test]
    fn extract_statuses_empty_when_no_container_statuses() {
        let pod = Pod::default();
        let statuses = extract_container_observed_statuses(&pod);
        assert!(statuses.is_empty());
    }

    #[test]
    fn extract_statuses_captures_oom_killed_reason() {
        let pod = pod_with_containers(vec![
            running_container("sandbox", true, true),
            terminated_container("preview-server", "OOMKilled", 137),
        ]);

        let statuses = extract_container_observed_statuses(&pod);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].name, "preview-server");
        assert_eq!(statuses[0].terminated_reason.as_deref(), Some("OOMKilled"));
        assert_eq!(statuses[0].exit_code, Some(137));
        assert_eq!(statuses[0].to_server_status(), ServerStatus::Failed);
    }

    #[test]
    fn extract_statuses_captures_error_termination() {
        let pod = pod_with_containers(vec![
            running_container("sandbox", true, true),
            terminated_container("preview-server", "Error", 1),
        ]);

        let statuses = extract_container_observed_statuses(&pod);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].terminated_reason.as_deref(), Some("Error"));
        assert_eq!(statuses[0].exit_code, Some(1));
        assert_eq!(statuses[0].to_server_status(), ServerStatus::Failed);
    }

    #[test]
    fn extract_statuses_completed_is_stopped() {
        let pod = pod_with_containers(vec![
            running_container("sandbox", true, true),
            terminated_container("preview-server", "Completed", 0),
        ]);

        let statuses = extract_container_observed_statuses(&pod);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].to_server_status(), ServerStatus::Stopped);
    }

    // =========================================================================
    // sandbox_status_to_pod_phase()
    // =========================================================================

    #[test]
    fn env_status_roundtrip() {
        let cases = [
            (SandboxStatus::NotCreated, PodPhase::NotFound),
            (SandboxStatus::Initializing, PodPhase::Pending),
            (SandboxStatus::Ready, PodPhase::Running),
            (SandboxStatus::Stopped, PodPhase::Stopped),
            (SandboxStatus::Removing, PodPhase::Terminating),
            (SandboxStatus::Removed, PodPhase::NotFound),
            (SandboxStatus::Error, PodPhase::Failed),
        ];

        for (status, expected) in cases {
            assert_eq!(sandbox_status_to_pod_phase(&status), expected, "{status:?}");
        }
    }
}
