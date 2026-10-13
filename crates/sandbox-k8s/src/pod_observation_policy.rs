use moatless_types::sandbox::SandboxStatus;

use moatless_sandbox_runtime::classify::ObservedTransition;

use super::pod_watcher::PodObservedState;

const CONTAINER_OOM_KILLED: &str = "container_oom_killed";
const POD_EVICTED: &str = "pod_evicted";

/// Context that affects how an observation is projected.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ObservationContext {
    /// The runtime object is terminating because lifecycle already requested a
    /// stop. Error-level transitions become a clean stop in this case.
    pub intentional_stop: bool,
}

/// Complete pure policy result for one observed state transition.
#[derive(Debug, Clone, Default)]
pub(super) struct ObservationPlan {
    pub sandbox_transitions: Vec<SandboxTransition>,
    pub has_error_transition: bool,
    pub has_fatal_transition: bool,
    pub has_completed_transition: bool,
    pub should_record_startup: bool,
}

#[derive(Debug, Clone)]
pub(super) struct SandboxTransition {
    pub sub_type: String,
    pub error: Option<String>,
    pub status: SandboxStatus,
    pub is_error_event: bool,
    pub skip_duplicate_publish: bool,
    pub lifecycle_observation: Option<ObservedTransition>,
}

pub(super) struct PodObservationPolicy;

impl PodObservationPolicy {
    pub(super) fn evaluate(
        previous: Option<&PodObservedState>,
        current: &PodObservedState,
        context: ObservationContext,
    ) -> ObservationPlan {
        let raw_sandbox_transitions = detect_transitions(previous, current);
        let has_error_transition = raw_sandbox_transitions.iter().any(|(sub_type, _)| {
            matches!(
                lifecycle_sub_type_to_sandbox_status(sub_type),
                SandboxStatus::Error
            )
        });

        let sandbox_transitions = raw_sandbox_transitions
            .into_iter()
            .map(|(sub_type, error)| {
                let natural_status = lifecycle_sub_type_to_sandbox_status(&sub_type);
                let status =
                    if context.intentional_stop && matches!(natural_status, SandboxStatus::Error) {
                        SandboxStatus::Stopped
                    } else {
                        natural_status
                    };
                let is_error_event = matches!(status, SandboxStatus::Error);
                let skip_duplicate_publish = should_skip_duplicate_publish(
                    &sub_type,
                    status,
                    current.last_published_sandbox_status,
                );
                let lifecycle_observation = lifecycle_observed_transition(status, &sub_type);
                SandboxTransition {
                    sub_type,
                    error,
                    status,
                    is_error_event,
                    skip_duplicate_publish,
                    lifecycle_observation,
                }
            })
            .collect::<Vec<_>>();

        let has_fatal_transition = sandbox_transitions
            .iter()
            .any(|transition| matches!(transition.sub_type.as_str(), "pod_failed" | "pod_evicted"));
        let has_completed_transition = sandbox_transitions
            .iter()
            .any(|transition| transition.sub_type == "sandbox_completed");
        let should_record_startup = sandbox_transitions
            .iter()
            .any(|transition| transition.sub_type == "pod_ready")
            && !current.startup_recorded;

        ObservationPlan {
            sandbox_transitions,
            has_error_transition,
            has_fatal_transition,
            has_completed_transition,
            should_record_startup,
        }
    }
}

/// Whether a pod qualifies for a `scheduling_blocked` event: still Pending and
/// unscheduled beyond the threshold, not already reported.
pub(super) fn should_emit_scheduling_blocked(state: &PodObservedState, age_secs: i64) -> bool {
    !state.scheduling_blocked_published
        && state.phase == "Pending"
        && state.node_name.is_none()
        && age_secs >= super::pod_watcher::SCHEDULING_BLOCKED_THRESHOLD_SECS
}

/// Whether a pod qualifies for a `startup_slow` event: scheduled onto a node
/// but the sandbox container still isn't ready beyond the threshold.
pub(super) fn should_emit_startup_slow(state: &PodObservedState, age_secs: i64) -> bool {
    !state.startup_slow_published
        && state.node_name.is_some()
        && !state.ready
        && !matches!(state.phase.as_str(), "Failed" | "Succeeded")
        && age_secs >= super::pod_watcher::STARTUP_SLOW_THRESHOLD_SECS
}

/// Whether a stuck-Pending pod is a candidate for the attach-failure storage
/// fallback.
///
/// Allows a bound node, unlike `should_emit_scheduling_blocked`; the caller
/// confirms the cause via [`cause_is_attach_failure`].
pub(super) fn should_attempt_storage_fallback(state: &PodObservedState, age_secs: i64) -> bool {
    !state.storage_fallback_requested
        && state.phase == "Pending"
        && !state.ready
        && age_secs >= super::pod_watcher::SCHEDULING_BLOCKED_THRESHOLD_SECS
}

/// Whether a pod's newest Warning event names a volume-attach failure that the
/// storage fallback recovers from: matches `FailedAttachVolume` or
/// `Multi-Attach`, case-insensitively.
pub(super) fn cause_is_attach_failure(cause: &str) -> bool {
    let c = cause.to_ascii_lowercase();
    c.contains("failedattachvolume") || c.contains("multi-attach")
}

pub(super) fn should_gc_failed_pod(state: &PodObservedState, now_ms: i64) -> bool {
    state.phase == "Failed"
        && state.failed_since_ms.is_some_and(|failed_since| {
            now_ms - failed_since >= super::pod_watcher::FAILED_POD_GC_GRACE_MS
        })
        && !state.gc_requested
}

/// Lifecycle `sub_types` that can fire repeatedly during a `CrashLoopBackOff`.
///
/// Deduplicated to one publish per restart cycle so history isn't flooded;
/// genuine transitions always publish.
const HIGH_FREQUENCY_LIFECYCLE_SUB_TYPES: &[&str] = &[
    "container_restarted",
    "container_crash_loop",
    "container_terminated",
];

fn should_skip_duplicate_publish(
    sub_type: &str,
    status: SandboxStatus,
    last_published: Option<SandboxStatus>,
) -> bool {
    HIGH_FREQUENCY_LIFECYCLE_SUB_TYPES.contains(&sub_type) && last_published == Some(status)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleSandboxSubType {
    PodDeleted,
    PodEvicted,
    PodFailed,
    ContainerOomKilled,
    ContainerTerminated,
    ContainerCrashLoop,
    PodReady,
    PodSucceeded,
    SandboxCompleted,
    ContainerRestarted,
    PodRecovering,
    PodScheduled,
    NodeReclaimed,
}

impl LifecycleSandboxSubType {
    fn sandbox_status(self) -> SandboxStatus {
        match self {
            Self::PodDeleted => SandboxStatus::NotCreated,
            Self::PodEvicted | Self::PodFailed => SandboxStatus::Error,
            Self::ContainerOomKilled | Self::ContainerTerminated | Self::ContainerCrashLoop => {
                SandboxStatus::Initializing
            }
            Self::PodReady => SandboxStatus::Ready,
            Self::PodSucceeded | Self::SandboxCompleted => SandboxStatus::Stopped,
            Self::ContainerRestarted
            | Self::PodRecovering
            | Self::PodScheduled
            | Self::NodeReclaimed => SandboxStatus::Initializing,
        }
    }
}

impl TryFrom<&str> for LifecycleSandboxSubType {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pod_deleted" => Ok(Self::PodDeleted),
            "pod_evicted" => Ok(Self::PodEvicted),
            "pod_failed" => Ok(Self::PodFailed),
            "container_oom_killed" => Ok(Self::ContainerOomKilled),
            "container_terminated" => Ok(Self::ContainerTerminated),
            "container_crash_loop" => Ok(Self::ContainerCrashLoop),
            "pod_ready" => Ok(Self::PodReady),
            "pod_succeeded" => Ok(Self::PodSucceeded),
            "sandbox_completed" => Ok(Self::SandboxCompleted),
            "container_restarted" => Ok(Self::ContainerRestarted),
            "pod_recovering" => Ok(Self::PodRecovering),
            "pod_scheduled" => Ok(Self::PodScheduled),
            "node_reclaimed" => Ok(Self::NodeReclaimed),
            other => Err(format!("unknown lifecycle sandbox sub_type: {other}")),
        }
    }
}

/// Map a lifecycle event `sub_type` to the corresponding `SandboxStatus`.
pub(super) fn lifecycle_sub_type_to_sandbox_status(sub_type: &str) -> SandboxStatus {
    LifecycleSandboxSubType::try_from(sub_type).map_or(
        SandboxStatus::Error,
        LifecycleSandboxSubType::sandbox_status,
    )
}

pub(super) fn lifecycle_observed_transition(
    status: SandboxStatus,
    lifecycle_event: &str,
) -> Option<ObservedTransition> {
    if lifecycle_event == CONTAINER_OOM_KILLED {
        return Some(ObservedTransition::OomKilled);
    }
    if lifecycle_event == POD_EVICTED {
        return Some(ObservedTransition::Evicted);
    }
    match status {
        SandboxStatus::Ready => Some(ObservedTransition::Ready),
        SandboxStatus::Stopped => Some(ObservedTransition::CleanStop),
        SandboxStatus::Removed => Some(ObservedTransition::Deleted),
        SandboxStatus::Error => Some(ObservedTransition::Crash),
        _ => None,
    }
}

/// Whether an eviction's kubelet status message names a graceful node-shutdown
/// or drain cause rather than resource pressure.
fn eviction_is_node_lifecycle(message: Option<&str>) -> bool {
    let Some(msg) = message else {
        return false;
    };
    let m = msg.to_ascii_lowercase();
    m.contains("node shutdown")
        || m.contains("shutting down")
        || m.contains("terminated in response to")
        || m.contains("drain")
}

fn is_node_removal_reason(reason: &str) -> bool {
    matches!(
        reason,
        "DeletionByPodGC" | "NodeLost" | "NodeShutdown" | "Terminated" | "DeletionByTaintManager"
    )
}

/// Detect lifecycle transitions between previous and current pod state.
fn detect_transitions(
    prev: Option<&PodObservedState>,
    current: &PodObservedState,
) -> Vec<(String, Option<String>)> {
    let mut events = Vec::new();

    if current.terminated_reason.as_deref() == Some("OOMKilled") {
        let prev_reason = prev.and_then(|p| p.terminated_reason.as_deref());
        if prev_reason != Some("OOMKilled") {
            events.push((
                "container_oom_killed".to_string(),
                Some("Container was OOMKilled".to_string()),
            ));
        }
    }

    if let Some(ref reason) = current.terminated_reason {
        if reason != "OOMKilled" && reason != "Completed" {
            let prev_reason = prev.and_then(|p| p.terminated_reason.as_deref());
            if prev_reason != Some(reason) {
                events.push((
                    "container_terminated".to_string(),
                    Some(format!("Container terminated: {reason}")),
                ));
            }
        }
    }

    if current.terminated_reason.as_deref() == Some("Completed") && current.exit_code == Some(0) {
        let prev_reason = prev.and_then(|p| p.terminated_reason.as_deref());
        if prev_reason != Some("Completed") {
            events.push(("sandbox_completed".to_string(), None));
        }
    }

    if current.waiting_reason.as_deref() == Some("CrashLoopBackOff") {
        let prev_reason = prev.and_then(|p| p.waiting_reason.as_deref());
        if prev_reason != Some("CrashLoopBackOff") {
            events.push((
                "container_crash_loop".to_string(),
                Some("Container is in CrashLoopBackOff".to_string()),
            ));
        }
    }

    if let Some(prev) = prev {
        if current.restart_count > prev.restart_count {
            events.push((
                "container_restarted".to_string(),
                Some(format!(
                    "Container restarted (count: {} -> {})",
                    prev.restart_count, current.restart_count
                )),
            ));
        }
    }

    let prev_phase = prev.map_or("Unknown", |p| p.phase.as_str());
    if current.phase != prev_phase {
        match current.phase.as_str() {
            "Failed" => {
                let clean_exit = current.terminated_reason.as_deref() == Some("Completed")
                    && current.exit_code == Some(0);
                let is_evicted = current.pod_status_reason.as_deref() == Some("Evicted")
                    || current
                        .terminated_reason
                        .as_deref()
                        .is_some_and(|r| r.contains("Evict"));
                let resource_pressure_eviction = is_evicted
                    && !eviction_is_node_lifecycle(current.pod_status_message.as_deref());
                // Keep the reason: `NodeShutdown`/`Terminated` means the
                // kubelet drained the pod; `DeletionByPodGC`/`NodeLost` means
                // the node went with it, no SIGTERM. Check `disruption_reason`
                // first, matching `extract_pod_observed_state`'s precedence.
                let node_removal_reason = current
                    .disruption_reason
                    .as_deref()
                    .filter(|reason| is_node_removal_reason(reason))
                    .or_else(|| {
                        current
                            .pod_status_reason
                            .as_deref()
                            .filter(|reason| is_node_removal_reason(reason))
                    });

                let evicted_message = || {
                    current.pod_status_message.as_deref().map_or_else(
                        || "Pod was evicted".to_string(),
                        |m| format!("Pod evicted: {}", m.trim()),
                    )
                };

                if resource_pressure_eviction {
                    events.push(("pod_evicted".to_string(), Some(evicted_message())));
                } else if let Some(reason) = node_removal_reason {
                    events.push(("node_reclaimed".to_string(), Some(reason.to_string())));
                } else if clean_exit {
                } else if is_evicted {
                    events.push(("pod_evicted".to_string(), Some(evicted_message())));
                } else {
                    events.push((
                        "pod_failed".to_string(),
                        Some("Pod entered Failed phase".to_string()),
                    ));
                }
            }
            "Succeeded" => {
                events.push(("pod_succeeded".to_string(), None));
            }
            _ => {}
        }
    }

    if current.ready {
        let was_ready = prev.is_some_and(|p| p.ready);
        if !was_ready {
            events.push(("pod_ready".to_string(), None));
        }
    }

    if let Some(prev) = prev {
        if prev.node_name.is_none() && current.node_name.is_some() {
            events.push(("pod_scheduled".to_string(), None));
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use moatless_types::sandbox::SandboxStatus;

    use super::*;
    use crate::pod_status::test_helpers::{
        pod_with_phase_and_containers, running_container, terminated_container, waiting_container,
    };
    use moatless_sandbox_runtime::runtime::OPERATION_GENERATION_LABEL;

    fn plan_for(current: &PodObservedState) -> ObservationPlan {
        PodObservationPolicy::evaluate(None, current, ObservationContext::default())
    }

    fn transition_sub_types(plan: &ObservationPlan) -> Vec<&str> {
        plan.sandbox_transitions
            .iter()
            .map(|transition| transition.sub_type.as_str())
            .collect()
    }

    fn transition_error<'a>(plan: &'a ObservationPlan, sub_type: &str) -> Option<&'a str> {
        plan.sandbox_transitions
            .iter()
            .find(|transition| transition.sub_type == sub_type)
            .and_then(|transition| transition.error.as_deref())
    }

    #[test]
    fn failed_phase_with_clean_exit_is_not_fatal() {
        let pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Completed", 0)],
        );
        let state = super::super::pod_watcher::extract_pod_observed_state(&pod, "pod-1", "task-1");
        let plan = plan_for(&state);
        let subs = transition_sub_types(&plan);

        assert!(
            !subs.contains(&"pod_failed") && !subs.contains(&"pod_evicted"),
            "clean-exit Failed pod must not raise a fatal event: {subs:?}"
        );
        assert!(
            subs.contains(&"sandbox_completed"),
            "clean stop should still be reported via sandbox_completed: {subs:?}"
        );
    }

    #[test]
    fn failed_phase_with_crash_still_fatal() {
        let pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Error", 1)],
        );
        let state = super::super::pod_watcher::extract_pod_observed_state(&pod, "pod-1", "task-1");
        let plan = plan_for(&state);
        let subs = transition_sub_types(&plan);

        assert!(
            subs.contains(&"pod_failed"),
            "a real crash must raise pod_failed: {subs:?}"
        );
        assert!(plan.has_fatal_transition);
    }

    #[test]
    fn kubelet_eviction_surfaces_even_on_clean_exit() {
        let mut pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Completed", 0)],
        );
        let status = pod.status.as_mut().expect("test pod has status");
        status.reason = Some("Evicted".to_string());
        status.message =
            Some("Usage of EmptyDir volume \"docker-storage\" exceeds the limit \"10Gi\". ".into());

        let state = super::super::pod_watcher::extract_pod_observed_state(&pod, "pod-1", "task-1");
        let plan = plan_for(&state);
        let evicted = plan
            .sandbox_transitions
            .iter()
            .find(|transition| transition.sub_type == "pod_evicted")
            .expect("eviction must raise pod_evicted");
        let message = evicted.error.as_deref().unwrap_or_default();
        assert!(
            message.contains("docker-storage") && message.contains("10Gi"),
            "pod_evicted must carry the kubelet's eviction cause, got: {message}"
        );
    }

    #[test]
    fn node_shutdown_eviction_on_clean_exit_stays_silent() {
        let mut pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Completed", 0)],
        );
        let status = pod.status.as_mut().expect("test pod has status");
        status.reason = Some("Evicted".to_string());
        status.message = Some("The node was low on resource: node is shutting down.".into());

        let state = super::super::pod_watcher::extract_pod_observed_state(&pod, "pod-1", "task-1");
        let plan = plan_for(&state);
        let subs = transition_sub_types(&plan);

        assert!(
            !subs.contains(&"pod_evicted") && !subs.contains(&"pod_failed"),
            "a clean-exit node-shutdown eviction must stay silent: {subs:?}"
        );
        assert!(
            subs.contains(&"sandbox_completed"),
            "the clean stop is still reported via sandbox_completed: {subs:?}"
        );
    }

    #[test]
    fn deletion_by_pod_gc_is_node_reclaimed_not_fatal() {
        let mut pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Error", 137)],
        );
        let status = pod.status.as_mut().expect("test pod has status");
        status.reason = Some("DeletionByPodGC".to_string());
        status.message = Some("PodGC: node no longer exists".into());

        let state = super::super::pod_watcher::extract_pod_observed_state(&pod, "pod-1", "task-1");
        let plan = plan_for(&state);
        let subs = transition_sub_types(&plan);

        assert!(
            !subs.contains(&"pod_failed") && !subs.contains(&"pod_evicted"),
            "a node-removal reschedule must not raise a fatal event: {subs:?}"
        );
        assert!(
            subs.contains(&"node_reclaimed"),
            "node removal must surface node_reclaimed: {subs:?}"
        );
        assert_eq!(
            lifecycle_sub_type_to_sandbox_status("node_reclaimed"),
            SandboxStatus::Initializing
        );
        assert_eq!(
            transition_error(&plan, "node_reclaimed"),
            Some("DeletionByPodGC"),
            "the reason must survive: it is what separates a drained pod from a lost one"
        );
    }

    #[test]
    fn node_removal_via_disruption_condition_is_not_fatal() {
        use k8s_openapi::api::core::v1::PodCondition;

        let mut pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Error", 137)],
        );
        let status = pod.status.as_mut().expect("test pod has status");
        status.conditions = Some(vec![PodCondition {
            type_: "DisruptionTarget".to_string(),
            reason: Some("DeletionByTaintManager".to_string()),
            ..Default::default()
        }]);

        let state = super::super::pod_watcher::extract_pod_observed_state(&pod, "pod-1", "task-1");
        assert!(
            state.pod_status_reason.is_none(),
            "precondition: status.reason must be empty so only the condition carries the cause"
        );

        let plan = plan_for(&state);
        let subs = transition_sub_types(&plan);

        assert!(
            !subs.contains(&"pod_failed") && !subs.contains(&"pod_evicted"),
            "a condition-sourced node removal must not raise a fatal event: {subs:?}"
        );
        assert!(
            subs.contains(&"node_reclaimed"),
            "condition-sourced node removal must surface node_reclaimed: {subs:?}"
        );
        assert_eq!(
            transition_error(&plan, "node_reclaimed"),
            Some("DeletionByTaintManager"),
            "the disruption condition is the only carrier of the reason here"
        );
    }

    /// With both carriers populated the structured `DisruptionTarget` condition wins,
    /// matching `extract_pod_observed_state`. Reversing it would publish one reason in
    /// this event and another in the payload's `disruption_reason`.
    #[test]
    fn the_disruption_condition_outranks_the_pod_status_reason() {
        use k8s_openapi::api::core::v1::PodCondition;

        let mut pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Error", 137)],
        );
        let status = pod.status.as_mut().expect("test pod has status");
        status.reason = Some("NodeLost".to_string());
        status.conditions = Some(vec![PodCondition {
            type_: "DisruptionTarget".to_string(),
            reason: Some("DeletionByTaintManager".to_string()),
            ..Default::default()
        }]);

        let state = super::super::pod_watcher::extract_pod_observed_state(&pod, "pod-1", "task-1");
        let plan = plan_for(&state);

        assert_eq!(
            transition_error(&plan, "node_reclaimed"),
            Some("DeletionByTaintManager"),
            "the coarser status.reason must not outrank the structured condition"
        );
    }

    #[test]
    fn ready_tracks_sandbox_container_not_pod_condition() {
        let pod = pod_with_phase_and_containers(
            "Running",
            vec![
                running_container("sandbox", true, true),
                waiting_container("dind", "CrashLoopBackOff"),
            ],
        );

        let state = super::super::pod_watcher::extract_pod_observed_state(&pod, "pod-1", "task-1");
        assert!(
            state.ready,
            "sandbox ready should make observed state ready despite a crash-looping sidecar"
        );

        let plan = plan_for(&state);
        assert!(
            plan.sandbox_transitions
                .iter()
                .any(|transition| transition.sub_type == "pod_ready"),
            "expected pod_ready transition, got: {:?}",
            plan.sandbox_transitions
        );
        assert!(plan.should_record_startup);
    }

    #[test]
    fn detect_emits_pod_scheduled_on_binding() {
        let mut pending = super::super::pod_watcher::extract_pod_observed_state(
            &pod_with_phase_and_containers("Pending", vec![]),
            "pod-1",
            "task-1",
        );
        pending.node_name = None;

        let mut scheduled = pending.clone();
        scheduled.node_name = Some("ip-10-0-1-42".to_string());

        let plan = PodObservationPolicy::evaluate(
            Some(&pending),
            &scheduled,
            ObservationContext::default(),
        );
        assert!(
            plan.sandbox_transitions
                .iter()
                .any(|transition| transition.sub_type == "pod_scheduled"),
            "expected pod_scheduled, got: {:?}",
            plan.sandbox_transitions
        );

        let first_seen = plan_for(&scheduled);
        assert!(
            !first_seen
                .sandbox_transitions
                .iter()
                .any(|transition| transition.sub_type == "pod_scheduled"),
            "pod_scheduled should not fire retroactively, got: {:?}",
            first_seen.sandbox_transitions
        );
    }

    #[test]
    fn intentional_stop_downgrades_error_transitions() {
        let pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Error", 1)],
        );
        let state = super::super::pod_watcher::extract_pod_observed_state(&pod, "pod-1", "task-1");
        let plan = PodObservationPolicy::evaluate(
            None,
            &state,
            ObservationContext {
                intentional_stop: true,
            },
        );

        assert!(plan.has_error_transition);
        let failed = plan
            .sandbox_transitions
            .iter()
            .find(|transition| transition.sub_type == "pod_failed")
            .expect("pod_failed transition");
        assert_eq!(failed.status, SandboxStatus::Stopped);
        assert!(!failed.is_error_event);
    }

    #[test]
    fn lifecycle_sub_type_projects_to_sandbox_status() {
        let cases = [
            ("pod_deleted", SandboxStatus::NotCreated),
            ("pod_evicted", SandboxStatus::Error),
            ("pod_failed", SandboxStatus::Error),
            ("container_oom_killed", SandboxStatus::Initializing),
            ("container_terminated", SandboxStatus::Initializing),
            ("container_crash_loop", SandboxStatus::Initializing),
            ("pod_ready", SandboxStatus::Ready),
            ("pod_succeeded", SandboxStatus::Stopped),
            ("sandbox_completed", SandboxStatus::Stopped),
            ("container_restarted", SandboxStatus::Initializing),
            ("pod_recovering", SandboxStatus::Initializing),
            ("pod_scheduled", SandboxStatus::Initializing),
            ("node_reclaimed", SandboxStatus::Initializing),
        ];

        for (sub_type, expected) in cases {
            assert_eq!(
                lifecycle_sub_type_to_sandbox_status(sub_type),
                expected,
                "{sub_type}"
            );
        }
    }

    #[test]
    fn unknown_lifecycle_sub_type_projects_to_error() {
        assert_eq!(
            lifecycle_sub_type_to_sandbox_status("unknown"),
            SandboxStatus::Error
        );
    }

    #[test]
    fn watcher_maps_runtime_failures_to_lifecycle_observations() {
        assert_eq!(
            lifecycle_observed_transition(SandboxStatus::Stopped, "pod_deleted"),
            Some(ObservedTransition::CleanStop)
        );
        assert_eq!(
            lifecycle_observed_transition(SandboxStatus::Removed, "pod_deleted"),
            Some(ObservedTransition::Deleted)
        );
        assert_eq!(
            lifecycle_observed_transition(SandboxStatus::Error, CONTAINER_OOM_KILLED),
            Some(ObservedTransition::OomKilled)
        );
        assert_eq!(
            lifecycle_observed_transition(SandboxStatus::Error, POD_EVICTED),
            Some(ObservedTransition::Evicted)
        );
        assert_eq!(
            lifecycle_observed_transition(SandboxStatus::Error, "container_crashed"),
            Some(ObservedTransition::Crash)
        );
    }

    #[test]
    fn duplicate_publish_suppression_is_in_observation_plan() {
        let mut current = super::super::pod_watcher::extract_pod_observed_state(
            &pod_with_phase_and_containers(
                "Running",
                vec![running_container("sandbox", true, true)],
            ),
            "pod-1",
            "task-1",
        );
        current.restart_count = 2;
        current.last_published_sandbox_status = Some(SandboxStatus::Initializing);
        let mut previous = current.clone();
        previous.restart_count = 1;

        let plan = PodObservationPolicy::evaluate(
            Some(&previous),
            &current,
            ObservationContext::default(),
        );
        let restarted = plan
            .sandbox_transitions
            .iter()
            .find(|transition| transition.sub_type == "container_restarted")
            .expect("container_restarted transition");
        assert!(restarted.skip_duplicate_publish);

        current.last_published_sandbox_status = Some(SandboxStatus::Ready);
        let plan = PodObservationPolicy::evaluate(
            Some(&previous),
            &current,
            ObservationContext::default(),
        );
        let restarted = plan
            .sandbox_transitions
            .iter()
            .find(|transition| transition.sub_type == "container_restarted")
            .expect("container_restarted transition");
        assert!(!restarted.skip_duplicate_publish);
    }

    #[test]
    fn scheduling_blocked_predicate() {
        let mut s = super::super::pod_watcher::extract_pod_observed_state(
            &pod_with_phase_and_containers("Pending", vec![]),
            "pod-1",
            "task-1",
        );
        s.node_name = None;

        assert!(!should_emit_scheduling_blocked(
            &s,
            super::super::pod_watcher::SCHEDULING_BLOCKED_THRESHOLD_SECS - 1
        ));
        assert!(should_emit_scheduling_blocked(
            &s,
            super::super::pod_watcher::SCHEDULING_BLOCKED_THRESHOLD_SECS
        ));

        let mut scheduled = s.clone();
        scheduled.node_name = Some("ip-10-0-1-42".to_string());
        assert!(!should_emit_scheduling_blocked(
            &scheduled,
            super::super::pod_watcher::SCHEDULING_BLOCKED_THRESHOLD_SECS + 300
        ));

        s.scheduling_blocked_published = true;
        assert!(!should_emit_scheduling_blocked(
            &s,
            super::super::pod_watcher::SCHEDULING_BLOCKED_THRESHOLD_SECS + 300
        ));
    }

    #[test]
    fn storage_fallback_predicate_is_node_agnostic() {
        // A Multi-Attach stall happens after the pod binds to a node, so unlike
        // scheduling_blocked the predicate must fire with a node assigned.
        let mut s = super::super::pod_watcher::extract_pod_observed_state(
            &pod_with_phase_and_containers("Pending", vec![]),
            "pod-1",
            "task-1",
        );
        s.node_name = Some("ip-10-0-1-42".to_string());
        s.ready = false;

        assert!(!should_attempt_storage_fallback(
            &s,
            super::super::pod_watcher::SCHEDULING_BLOCKED_THRESHOLD_SECS - 1
        ));
        assert!(should_attempt_storage_fallback(
            &s,
            super::super::pod_watcher::SCHEDULING_BLOCKED_THRESHOLD_SECS
        ));

        // A ready or running pod is past the stall.
        let mut running = s.clone();
        running.phase = "Running".to_string();
        assert!(!should_attempt_storage_fallback(
            &running,
            super::super::pod_watcher::SCHEDULING_BLOCKED_THRESHOLD_SECS + 300
        ));

        // Fire-once guard.
        s.storage_fallback_requested = true;
        assert!(!should_attempt_storage_fallback(
            &s,
            super::super::pod_watcher::SCHEDULING_BLOCKED_THRESHOLD_SECS + 300
        ));
    }

    #[test]
    fn attach_failure_cause_matches_the_kubelet_and_controller_wording() {
        assert!(cause_is_attach_failure(
            "FailedAttachVolume: AttachVolume.Attach failed for volume \"pvc-abc\""
        ));
        assert!(cause_is_attach_failure(
            "Multi-Attach error for volume \"pvc-abc\": Volume is already used by pod(s) foo"
        ));
        assert!(cause_is_attach_failure("multi-attach error"));
        assert!(!cause_is_attach_failure(
            "FailedScheduling: 0/3 nodes are available: insufficient cpu"
        ));
        assert!(!cause_is_attach_failure("ImagePullBackOff"));
    }

    #[test]
    fn startup_slow_predicate() {
        let mut s = super::super::pod_watcher::extract_pod_observed_state(
            &pod_with_phase_and_containers("Running", vec![]),
            "pod-1",
            "task-1",
        );
        s.node_name = Some("ip-10-0-1-42".to_string());
        s.ready = false;

        assert!(!should_emit_startup_slow(
            &s,
            super::super::pod_watcher::STARTUP_SLOW_THRESHOLD_SECS - 1
        ));
        assert!(should_emit_startup_slow(
            &s,
            super::super::pod_watcher::STARTUP_SLOW_THRESHOLD_SECS
        ));

        let mut ready = s.clone();
        ready.ready = true;
        assert!(!should_emit_startup_slow(
            &ready,
            super::super::pod_watcher::STARTUP_SLOW_THRESHOLD_SECS + 300
        ));

        let mut failed = s.clone();
        failed.phase = "Failed".to_string();
        assert!(!should_emit_startup_slow(
            &failed,
            super::super::pod_watcher::STARTUP_SLOW_THRESHOLD_SECS + 300
        ));

        let mut unscheduled = s.clone();
        unscheduled.node_name = None;
        assert!(!should_emit_startup_slow(
            &unscheduled,
            super::super::pod_watcher::STARTUP_SLOW_THRESHOLD_SECS + 300
        ));

        s.startup_slow_published = true;
        assert!(!should_emit_startup_slow(
            &s,
            super::super::pod_watcher::STARTUP_SLOW_THRESHOLD_SECS + 300
        ));
    }

    #[test]
    fn gc_failed_pod_policy_requires_terminal_phase_grace_and_no_prior_gc() {
        let pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Error", 137)],
        );
        let mut state =
            super::super::pod_watcher::extract_pod_observed_state(&pod, "pod-1", "task-1");
        let now_ms = 1_000_000 + super::super::pod_watcher::FAILED_POD_GC_GRACE_MS;

        assert!(!should_gc_failed_pod(&state, now_ms));

        state.failed_since_ms = Some(1_000_000);
        assert!(should_gc_failed_pod(&state, now_ms));
        assert!(!should_gc_failed_pod(&state, now_ms - 1));

        state.gc_requested = true;
        assert!(!should_gc_failed_pod(&state, now_ms));

        state.gc_requested = false;
        state.phase = "Running".to_string();
        assert!(!should_gc_failed_pod(&state, now_ms));
    }

    #[test]
    fn pod_observation_carries_operation_generation() {
        let mut pod = k8s_openapi::api::core::v1::Pod::default();
        pod.metadata.labels = Some(std::collections::BTreeMap::from([(
            OPERATION_GENERATION_LABEL.to_string(),
            "12".to_string(),
        )]));
        let state = super::super::pod_watcher::extract_pod_observed_state(&pod, "pod-1", "task-1");
        assert_eq!(
            state.operation_generation,
            Some(moatless_sandbox_runtime::operation::OperationGeneration(12))
        );
    }
}
