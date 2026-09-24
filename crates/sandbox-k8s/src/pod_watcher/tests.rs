//! Unit tests for the watcher's transition detection.

use super::remediation::{
    CleanExitAction, clean_exit_action_for_intent, maybe_run_storage_fallback,
};
use super::watch::WatcherDeps;
use super::*;
use crate::pod_status::test_helpers::{
    pod_with_phase_and_containers, running_container, terminated_container,
};

/// A terminated sandbox container exposes its exit code + K8s reason so the
/// fatal/termination payloads can distinguish OOM (137) from a crash (1).
#[test]
fn extract_captures_sandbox_exit_code_and_reason() {
    let pod =
        pod_with_phase_and_containers("Failed", vec![terminated_container("sandbox", "Error", 1)]);
    let state = extract_pod_observed_state(&pod, "pod-1", "task-1");

    assert_eq!(state.exit_code, Some(1));
    assert_eq!(state.terminated_reason.as_deref(), Some("Error"));
    assert_eq!(state.effective_exit_code(), Some(1));
    assert_eq!(
        state.effective_terminated_reason().as_deref(),
        Some("Error")
    );
}

/// After K8s restarts the container the *current* state is Running, but the
/// exit code/reason survive in `last_state` and must still be recoverable.
#[test]
fn extract_falls_back_to_last_terminated_state() {
    use k8s_openapi::api::core::v1::{
        ContainerState as K8sState, ContainerStateRunning, ContainerStateTerminated,
        ContainerStatus,
    };

    let sandbox = ContainerStatus {
        name: "sandbox".to_string(),
        ready: true,
        started: Some(true),
        restart_count: 1,
        state: Some(K8sState {
            running: Some(ContainerStateRunning::default()),
            ..Default::default()
        }),
        last_state: Some(K8sState {
            terminated: Some(ContainerStateTerminated {
                reason: Some("OOMKilled".to_string()),
                exit_code: 137,
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let pod = pod_with_phase_and_containers("Running", vec![sandbox]);
    let state = extract_pod_observed_state(&pod, "pod-1", "task-1");

    // Current-state fields stay empty (container is Running) so transition
    // detection still compares current-vs-previous correctly...
    assert_eq!(state.exit_code, None);
    assert_eq!(state.terminated_reason, None);
    // ...but the last-terminated diagnostics are preserved.
    assert_eq!(state.last_exit_code, Some(137));
    assert_eq!(state.last_terminated_reason.as_deref(), Some("OOMKilled"));
    assert_eq!(state.effective_exit_code(), Some(137));
    assert_eq!(
        state.effective_terminated_reason().as_deref(),
        Some("OOMKilled")
    );
}

/// When the sandbox container itself is not ready, the state is not ready even
/// if other containers are healthy.
#[test]
fn not_ready_when_sandbox_container_not_ready() {
    let pod = pod_with_phase_and_containers(
        "Running",
        vec![
            running_container("sandbox", true, false),
            running_container("dind", true, true),
        ],
    );

    let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
    assert!(!state.ready);
}

/// `extract_pod_observed_state` captures the scheduled node and the structured
/// `DisruptionTarget` reason so node-level correlation and the disruption cause
/// are readable directly from the event.
#[test]
fn extract_captures_node_and_disruption_reason() {
    use k8s_openapi::api::core::v1::{PodCondition, PodSpec, PodStatus};

    let pod = Pod {
        spec: Some(PodSpec {
            node_name: Some("ip-10-0-1-42".to_string()),
            ..Default::default()
        }),
        status: Some(PodStatus {
            phase: Some("Running".to_string()),
            reason: Some("Evicted".to_string()),
            conditions: Some(vec![PodCondition {
                type_: "DisruptionTarget".to_string(),
                reason: Some("PreemptionByScheduler".to_string()),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
    assert_eq!(state.node_name.as_deref(), Some("ip-10-0-1-42"));
    // The structured DisruptionTarget reason wins over status.reason.
    assert_eq!(
        state.disruption_reason.as_deref(),
        Some("PreemptionByScheduler")
    );
}

/// With no `DisruptionTarget` condition, the disruption reason falls back to
/// the coarser pod-level `status.reason`.
#[test]
fn disruption_reason_falls_back_to_pod_status_reason() {
    use k8s_openapi::api::core::v1::PodStatus;

    let pod = Pod {
        status: Some(PodStatus {
            phase: Some("Failed".to_string()),
            reason: Some("Evicted".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
    assert_eq!(state.disruption_reason.as_deref(), Some("Evicted"));
    assert!(state.node_name.is_none());
}

#[test]
fn pod_observation_carries_operation_generation() {
    let mut pod = Pod::default();
    pod.metadata.labels = Some(std::collections::BTreeMap::from([(
        OPERATION_GENERATION_LABEL.to_string(),
        "12".to_string(),
    )]));
    let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
    assert_eq!(state.operation_generation, Some(OperationGeneration(12)));
}

/// A remove deletes the Deployment before the pod's exit-0 event arrives, so
/// "confirming" a scale-to-0 here would patch a Deployment that is already gone
/// and log a spurious `NotFound` warning.
#[test]
fn clean_exit_after_remove_leaves_the_deleted_deployment_alone() {
    assert_eq!(
        clean_exit_action_for_intent(Some(SandboxDesiredState::Removed)),
        CleanExitAction::AlreadyRemoved
    );
}

/// A stop scales to 0 and keeps the Deployment, so the clean exit confirms it.
#[test]
fn clean_exit_after_stop_confirms_scale_to_zero() {
    assert_eq!(
        clean_exit_action_for_intent(Some(SandboxDesiredState::Stopped)),
        CleanExitAction::ConfirmStop
    );
}

/// A clean exit while a replica is still wanted is an involuntary node event
/// (spot reclaim, drain). Scaling to 0 would cancel a live task.
#[test]
fn clean_exit_while_running_is_left_for_kubernetes() {
    assert_eq!(
        clean_exit_action_for_intent(Some(SandboxDesiredState::Running)),
        CleanExitAction::LeaveForReschedule
    );
}

/// Unreadable intent fails safe the same way: never scale down on a guess.
#[test]
fn clean_exit_with_unknown_intent_fails_safe() {
    assert_eq!(
        clean_exit_action_for_intent(None),
        CleanExitAction::LeaveForReschedule
    );
}

/// A `StorageFallback` double that records its calls and returns a fixed
/// outcome, so the watcher's remediation glue is testable without a cluster.
struct FakeFallback {
    outcome: StorageFallbackOutcome,
    calls: std::sync::Mutex<Vec<(String, OperationGeneration)>>,
}

impl FakeFallback {
    fn new(outcome: StorageFallbackOutcome) -> Self {
        Self {
            outcome,
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn calls(&self) -> Vec<(String, OperationGeneration)> {
        self.calls.lock().expect("poisoned").clone()
    }
}

#[async_trait::async_trait]
impl StorageFallback for FakeFallback {
    async fn rebuild_without_pvc(
        &self,
        task_id: &str,
        generation: OperationGeneration,
    ) -> StorageFallbackOutcome {
        self.calls
            .lock()
            .expect("poisoned")
            .push((task_id.to_string(), generation));
        self.outcome
    }
}

fn deps_with_fallback(fallback: Arc<dyn StorageFallback>) -> WatcherDeps {
    WatcherDeps {
        node_archive_enabled: false,
        lifecycle: None,
        storage_fallback: Some(fallback),
    }
}

fn pending_state_with_generation(gen: Option<i64>) -> PodObservedState {
    let mut state = extract_pod_observed_state(
        &pod_with_phase_and_containers("Pending", vec![]),
        "pod-1",
        "task-1",
    );
    state.operation_generation = gen.map(OperationGeneration);
    state
}

#[tokio::test]
async fn storage_fallback_rebuild_publishes_degradation_and_guards() {
    use moatless_core::events::test_support::RecordingPublisher;

    let fallback = Arc::new(FakeFallback::new(StorageFallbackOutcome::Rebuilt));
    let deps = deps_with_fallback(fallback.clone());
    let publisher = RecordingPublisher::new();
    let mut state = pending_state_with_generation(Some(7));

    maybe_run_storage_fallback(&publisher, &mut state, &deps).await;

    assert_eq!(
        fallback.calls(),
        vec![("task-1".to_string(), OperationGeneration(7))],
        "the rebuild must be invoked with the pod's task and generation"
    );
    assert!(
        state.storage_fallback_requested,
        "the fire-once guard must be set after a rebuild"
    );
    assert_eq!(
        publisher.sandbox_statuses().len(),
        1,
        "a rebuild must surface exactly one degradation event"
    );
}

#[tokio::test]
async fn storage_fallback_skip_sets_guard_without_event() {
    use moatless_core::events::test_support::RecordingPublisher;

    let fallback = Arc::new(FakeFallback::new(StorageFallbackOutcome::Skipped));
    let deps = deps_with_fallback(fallback.clone());
    let publisher = RecordingPublisher::new();
    let mut state = pending_state_with_generation(Some(7));

    maybe_run_storage_fallback(&publisher, &mut state, &deps).await;

    assert_eq!(fallback.calls().len(), 1);
    assert!(
        state.storage_fallback_requested,
        "a skipped swap still guards so it doesn't retry every sweep"
    );
    assert!(
        publisher.sandbox_statuses().is_empty(),
        "a skipped swap (e.g. s3 off) publishes no degradation"
    );
}

#[tokio::test]
async fn storage_fallback_without_generation_does_not_rebuild() {
    use moatless_core::events::test_support::RecordingPublisher;

    let fallback = Arc::new(FakeFallback::new(StorageFallbackOutcome::Rebuilt));
    let deps = deps_with_fallback(fallback.clone());
    let publisher = RecordingPublisher::new();
    let mut state = pending_state_with_generation(None);

    maybe_run_storage_fallback(&publisher, &mut state, &deps).await;

    assert!(
        fallback.calls().is_empty(),
        "an unlabelled pod cannot be correlated, so no rebuild is attempted"
    );
    assert!(
        state.storage_fallback_requested,
        "the guard is still set so the unlabelled pod isn't reconsidered"
    );
    assert!(publisher.sandbox_statuses().is_empty());
}
