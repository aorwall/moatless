//! The periodic pass over tracked pods that turns a silent window — stuck
//! Pending, slow start, wedged volume — into an event.

use super::diagnostics::fetch_pod_event_cause;
use super::remediation::{crash_autostop, maybe_run_storage_fallback};
use super::watch::WatcherDeps;
use super::*;
use moatless_sandbox_runtime::metrics::{Repair, record_repair};

/// Pod-Pending duration after which the watcher emits `scheduling_blocked`,
/// surfacing an otherwise-silent unschedulable window (no node, quota, etc.).
pub(crate) const SCHEDULING_BLOCKED_THRESHOLD_SECS: i64 = 60;

/// Time a scheduled pod may sit with its sandbox container not yet running
/// before the watcher emits `startup_slow` — makes a long volume-attach or
/// image-pull window (e.g. EBS RWO Multi-Attach) a first-class event.
pub(crate) const STARTUP_SLOW_THRESHOLD_SECS: i64 = 120;

/// Periodic sweep over tracked pods that emits the threshold-driven
/// visibility events: `scheduling_blocked` (stuck Pending) and `startup_slow`
/// (scheduled but container not yet running).
///
/// Runs on the watcher's renew tick; each event fires at most once per pod
/// via the sticky `*_published` guards.
pub(super) async fn sweep_pending_pods(
    k8s: &K8sClient,
    event_publisher: &dyn EventPublisher,
    pod_states: &mut HashMap<String, PodObservedState>,
    deps: &WatcherDeps,
) {
    let now_ms = k8s_openapi::jiff::Timestamp::now().as_millisecond();

    for state in pod_states.values_mut() {
        let Some(created) = state.created_at_ms else {
            continue;
        };
        let age_secs = (now_ms - created).max(0) / 1000;

        let emit_scheduling_blocked = should_emit_scheduling_blocked(state, age_secs);
        let emit_startup_slow = should_emit_startup_slow(state, age_secs);
        let fallback_candidate =
            deps.storage_fallback.is_some() && should_attempt_storage_fallback(state, age_secs);

        // Read the pod's newest Warning event at most once per sweep, shared by
        // the two threshold events and the attach-failure fallback below.
        let cause = if emit_scheduling_blocked || emit_startup_slow || fallback_candidate {
            fetch_pod_event_cause(k8s, &state.pod_name).await
        } else {
            None
        };

        // Attach-failure fallback: a pvc+s3 pod wedged on `Multi-Attach` /
        // `FailedAttachVolume` can come back up on an emptyDir restored from S3.
        // Guarded once per pod (≈ once per generation) whatever the outcome.
        if fallback_candidate && cause.as_deref().is_some_and(cause_is_attach_failure) {
            maybe_run_storage_fallback(event_publisher, state, deps).await;
        }

        // scheduling_blocked: still Pending and unscheduled past the threshold.
        if emit_scheduling_blocked {
            warn!(
                task_id = %state.task_id,
                pod_name = %state.pod_name,
                age_secs,
                cause = ?cause,
                "Pod stuck Pending past threshold — emitting scheduling_blocked"
            );
            event_publisher
                .publish_sandbox_event_with_payload(
                    &state.task_id,
                    SandboxStatus::Initializing,
                    &SandboxEventPayload {
                        message: cause.clone().map_or_else(
                            || format!("Scheduling blocked ({age_secs}s Pending)"),
                            |c| format!("Scheduling blocked: {c}"),
                        ),
                        lifecycle_event: Some("scheduling_blocked".to_string()),
                        disruption_reason: cause.clone(),
                        pending_seconds: Some(age_secs),
                        ..Default::default()
                    },
                )
                .await;
            state.scheduling_blocked_published = true;
        }

        // startup_slow: scheduled onto a node but the sandbox container still
        // isn't ready long after creation (e.g. a long EBS attach / image pull).
        if emit_startup_slow {
            warn!(
                task_id = %state.task_id,
                pod_name = %state.pod_name,
                age_secs,
                waiting_reason = ?state.waiting_reason,
                cause = ?cause,
                "Sandbox container slow to start past threshold — emitting startup_slow"
            );
            event_publisher
                .publish_sandbox_event_with_payload(
                    &state.task_id,
                    SandboxStatus::Initializing,
                    &SandboxEventPayload {
                        message: cause.clone().map_or_else(
                            || format!("Slow startup ({age_secs}s, not ready)"),
                            |c| format!("Slow startup: {c}"),
                        ),
                        lifecycle_event: Some("startup_slow".to_string()),
                        node_name: state.node_name.clone(),
                        disruption_reason: cause,
                        pending_seconds: Some(age_secs),
                        ..Default::default()
                    },
                )
                .await;
            state.startup_slow_published = true;
        }
    }
}

/// How long a `Failed` pod corpse is kept before being garbage-collected.
///
/// Long enough for post-mortem inspection (`kubectl logs --previous`, pod
/// describe), short enough that corpses don't accumulate — K8s itself never
/// removes Failed pods until the cluster-wide terminated-pod GC threshold.
pub(crate) const FAILED_POD_GC_GRACE_MS: i64 = 15 * 60 * 1000;

/// Delete pods stuck in the terminal `Failed` phase past the grace period.
///
/// An evicted pod stays `Failed` forever unless deleted; the `ReplicaSet` has
/// already replaced it, so it's a pure corpse. Runs on the watcher's renew
/// tick (leader-only); the `Delete` watch event removes it from
/// `pod_states`.
pub(super) async fn gc_failed_pods(
    k8s: &K8sClient,
    pod_states: &mut HashMap<String, PodObservedState>,
) {
    let now_ms = k8s_openapi::jiff::Timestamp::now().as_millisecond();
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());

    for state in pod_states.values_mut() {
        if !should_gc_failed_pod(state, now_ms) {
            continue;
        }

        match pods_api
            .delete(&state.pod_name, &DeleteParams::default())
            .await
        {
            Ok(_) => {
                info!(
                    task_id = %state.task_id,
                    pod_name = %state.pod_name,
                    "Garbage-collected Failed pod past grace period"
                );
                record_repair(Repair::FailedPodGc);
                state.gc_requested = true;
            }
            Err(kube::Error::Api(ae)) if ae.code == 404 => {
                // Already gone (manual delete, node GC) — nothing to do.
                state.gc_requested = true;
            }
            Err(e) => {
                warn!(
                    task_id = %state.task_id,
                    pod_name = %state.pod_name,
                    error = %e,
                    "Failed to garbage-collect Failed pod; will retry next sweep"
                );
            }
        }
    }
}

/// Retry the crash auto-stop for any pod with a still-pending auto-stop.
///
/// The watch stream only fires on pod changes, so a failed auto-stop with no
/// further events would never retry; this sweep re-issues the lifecycle stop
/// for every pod whose `auto_stop_pending` is still set, clearing it on
/// success.
pub(super) async fn retry_pending_auto_stops(
    pod_states: &mut HashMap<String, PodObservedState>,
    deps: &WatcherDeps,
) {
    for state in pod_states.values_mut() {
        if !state.auto_stop_pending {
            continue;
        }
        if crash_autostop(deps, &state.task_id).await {
            info!(
                task_id = %state.task_id,
                pod_name = %state.pod_name,
                "Crash auto-stop succeeded on periodic retry"
            );
            record_repair(Repair::CrashAutoStopRetry);
            state.auto_stop_pending = false;
        } else {
            warn!(
                task_id = %state.task_id,
                pod_name = %state.pod_name,
                "Crash auto-stop still failing; will retry next sweep"
            );
        }
    }
}
