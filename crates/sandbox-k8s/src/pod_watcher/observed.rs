//! The observed state of a pod, read off one `Pod` object.

use super::*;

/// Observed state for a single pod, used for transition detection.
#[derive(Debug, Clone)]
pub(crate) struct PodObservedState {
    pub(crate) task_id: String,
    pub(crate) operation_generation: Option<OperationGeneration>,
    pub(crate) phase: String,
    /// Restart count of the **sandbox** container specifically.
    pub(crate) restart_count: i32,
    pub(crate) terminated_reason: Option<String>,
    pub(crate) waiting_reason: Option<String>,
    pub(crate) ready: bool,
    pub(crate) pod_name: String,
    /// Exit code from the sandbox container's *current* terminated state.
    pub(crate) exit_code: Option<i32>,
    /// Exit code from the sandbox container's *last* terminated state —
    /// survives a K8s restart so a crash that's already been restarted into
    /// Running isn't lost.
    pub(crate) last_exit_code: Option<i32>,
    /// Termination reason from the sandbox container's *last* terminated state.
    pub(crate) last_terminated_reason: Option<String>,
    /// Termination message or waiting message from the sandbox container.
    pub(crate) failure_detail: Option<String>,
    /// Whether a fatal event has already been published for this pod.
    /// Suppresses repeated fatal events during `CrashLoopBackOff`.
    pub(super) fatal_published: bool,
    /// Whether the `moatless_sandbox_startup_seconds` histogram has already
    /// been recorded for this pod.
    ///
    /// Carried over so a pod that goes Ready→NotReady→Ready (e.g. after a
    /// container restart) is only timed on its first ready, keeping startup
    /// latency from being inflated by re-readies.
    pub(crate) startup_recorded: bool,
    /// Node the pod is scheduled on (`pod.spec.node_name`), once bound. `None`
    /// while the pod is still `Pending` and unscheduled.
    pub(crate) node_name: Option<String>,
    /// Why the pod was disrupted, derived from the `DisruptionTarget` pod
    /// condition reason, falling back to `pod.status.reason`.
    pub(crate) disruption_reason: Option<String>,
    /// Raw pod-level `status.reason`. Kubelet resource evictions (emptyDir
    /// sizeLimit, node pressure) set exactly `"Evicted"` here — the container
    /// states never carry it, so this is the only reliable eviction signal.
    pub(crate) pod_status_reason: Option<String>,
    /// Pod-level `status.message` — for evictions this holds the actionable
    /// cause (e.g. `Usage of EmptyDir volume "docker-storage" exceeds the limit
    /// "10Gi"`).
    pub(crate) pod_status_message: Option<String>,
    /// Pod `creationTimestamp` as epoch milliseconds, used to derive
    /// `pending_seconds` on the Pending→scheduled edge.
    pub(super) created_at_ms: Option<i64>,
    /// Fire-once guard: a `scheduling_blocked` event has been emitted for this
    /// pod's stuck-Pending window. Carried over so the threshold sweep emits at
    /// most one blocked event per pod.
    pub(crate) scheduling_blocked_published: bool,
    /// Fire-once guard: a `startup_slow` event has been emitted for this pod's
    /// slow-container-start window (e.g. long volume attach).
    pub(crate) startup_slow_published: bool,
    /// Fire-once guard: an attach-failure storage fallback has been requested
    /// for this pod.
    ///
    /// Keyed per pod, so it's effectively once per generation — a rebuild
    /// replaces the pod, and the fresh pod starts the guard clear. Set
    /// regardless of outcome, so a failed rebuild waits for the next clean
    /// start rather than retrying every sweep.
    pub(crate) storage_fallback_requested: bool,
    /// Epoch ms when this pod was first observed in the terminal `Failed`
    /// phase.
    ///
    /// K8s never garbage-collects Failed pods (e.g. evicted for exceeding an
    /// emptyDir sizeLimit), so the sweep deletes them once this is older than
    /// the GC grace period.
    pub(crate) failed_since_ms: Option<i64>,
    /// Fire-once guard: a GC delete has been issued for this Failed pod.
    pub(crate) gc_requested: bool,
    /// An auto-stop (scale Deployment to 0) has been decided for this pod — via
    /// the crash-restart threshold crossing or the exit-78 fast-fail — but not
    /// yet confirmed by a successful `scale_deployment` call.
    ///
    /// Cleared only on a successful scale-to-0, and retried on every watcher
    /// event and the periodic sweep; without this, a failed scale is lost and
    /// `restart_count` staying `>= threshold` means the "new crossing" edge
    /// never re-fires, so the pod crash-loops forever.
    pub(super) auto_stop_pending: bool,
    /// Sandbox status most recently published from this pod's lifecycle
    /// transitions, carried across Apply events.
    ///
    /// Suppresses consecutive identical publishes from the high-frequency
    /// container `sub_types` (restart/crash-loop/terminated), which otherwise
    /// fire once per K8s watch Apply while a container flaps; watcher restart
    /// resets this (acceptable — one extra event per pod after failover).
    pub(crate) last_published_sandbox_status: Option<SandboxStatus>,
}

impl PodObservedState {
    /// Best-known sandbox-container exit code: the current terminated state if
    /// present, otherwise the last terminated state (which survives a restart).
    pub(super) fn effective_exit_code(&self) -> Option<i32> {
        self.exit_code.or(self.last_exit_code)
    }

    /// Best-known sandbox-container termination reason (current, else last).
    pub(super) fn effective_terminated_reason(&self) -> Option<String> {
        self.terminated_reason
            .clone()
            .or_else(|| self.last_terminated_reason.clone())
    }
}

/// Extract the task ID from a pod's labels.
pub(super) fn extract_task_id(pod: &Pod) -> Option<String> {
    pod.metadata
        .labels
        .as_ref()
        .and_then(|labels| labels.get("moatless.io/task-id"))
        .cloned()
}

/// Record sandbox startup latency (pod creation → first sandbox-container
/// Ready) into the `moatless_sandbox_startup_seconds` histogram.
///
/// Uses the pod's `metadata.creationTimestamp` as the start; skips pods without
/// a creation timestamp or with a negative (clock-skew) elapsed time.
pub(super) fn record_sandbox_startup(pod: &Pod) {
    // k8s-openapi represents `creationTimestamp` as a jiff `Timestamp`, not
    // chrono. Compare epoch-millis directly to avoid a cross-crate Sub impl.
    let Some(created) = pod.metadata.creation_timestamp.as_ref().map(|t| t.0) else {
        return;
    };
    let elapsed_ms =
        k8s_openapi::jiff::Timestamp::now().as_millisecond() - created.as_millisecond();
    if elapsed_ms < 0 {
        return;
    }
    metrics::histogram!("moatless_sandbox_startup_seconds").record(elapsed_ms as f64 / 1000.0);
}

pub(crate) fn extract_pod_observed_state(
    pod: &Pod,
    pod_name: &str,
    task_id: &str,
) -> PodObservedState {
    let status = pod.status.as_ref();

    let phase = status
        .and_then(|s| s.phase.as_deref())
        .unwrap_or("Unknown")
        .to_string();

    // Fix: find the "sandbox" container specifically instead of using first()
    let sandbox_container_status = status
        .and_then(|s| s.container_statuses.as_ref())
        .and_then(|cs| cs.iter().find(|c| c.name == "sandbox"));

    if sandbox_container_status.is_none() && phase == "Running" {
        warn!(
            task_id,
            pod_name, "No sandbox container found in running pod"
        );
    }

    let restart_count = sandbox_container_status.map_or(0, |cs| cs.restart_count);

    let terminated_reason = sandbox_container_status
        .and_then(|cs| cs.state.as_ref())
        .and_then(|s| s.terminated.as_ref())
        .and_then(|t| t.reason.clone());

    let waiting_reason = sandbox_container_status
        .and_then(|cs| cs.state.as_ref())
        .and_then(|s| s.waiting.as_ref())
        .and_then(|w| w.reason.clone());

    let exit_code = sandbox_container_status
        .and_then(|cs| cs.state.as_ref())
        .and_then(|s| s.terminated.as_ref())
        .map(|t| t.exit_code);

    // Last-terminated state survives a K8s restart (current state becomes
    // Running/Waiting after restart). Kept separate from the current-state fields
    // above so transition detection keeps comparing current-vs-previous, while the
    // diagnostics payloads can still recover the exit code/reason after a restart.
    let last_terminated = sandbox_container_status
        .and_then(|cs| cs.last_state.as_ref())
        .and_then(|s| s.terminated.as_ref());
    let last_terminated_reason = last_terminated.and_then(|t| t.reason.clone());
    let last_exit_code = last_terminated.map(|t| t.exit_code);

    let failure_detail = sandbox_container_status
        .and_then(|cs| cs.state.as_ref())
        .and_then(|s| s.terminated.as_ref())
        .and_then(|t| t.message.clone())
        .or_else(|| {
            sandbox_container_status
                .and_then(|cs| cs.last_state.as_ref())
                .and_then(|s| s.terminated.as_ref())
                .and_then(|t| t.message.clone())
        })
        .or_else(|| {
            sandbox_container_status
                .and_then(|cs| cs.state.as_ref())
                .and_then(|s| s.waiting.as_ref())
                .and_then(|w| w.message.clone())
        });

    // Track readiness from the **sandbox** container specifically, not the
    // pod-level `Ready` condition, mirroring `status_resolver`'s use of
    // `is_sandbox_container_ready`.
    let ready = is_sandbox_container_ready(pod);

    // Node the pod is bound to (empty/absent while still Pending+unscheduled).
    let node_name = pod
        .spec
        .as_ref()
        .and_then(|s| s.node_name.clone())
        .filter(|n| !n.is_empty());

    // Why the pod was disrupted: prefer the structured `DisruptionTarget`
    // condition reason (spot reclaim, drain, preemption, kubelet eviction, …),
    // falling back to the coarser pod-level `status.reason` (e.g. "Evicted").
    let disruption_reason = status
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conds| {
            conds
                .iter()
                .find(|c| c.type_ == "DisruptionTarget")
                .and_then(|c| c.reason.clone())
        })
        .or_else(|| status.and_then(|s| s.reason.clone()));

    let created_at_ms = pod
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|t| t.0.as_millisecond());

    PodObservedState {
        task_id: task_id.to_string(),
        operation_generation: pod
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| generation_from_label(labels.get(OPERATION_GENERATION_LABEL))),
        phase,
        restart_count,
        terminated_reason,
        waiting_reason,
        ready,
        pod_name: pod_name.to_string(),
        exit_code,
        last_exit_code,
        last_terminated_reason,
        failure_detail,
        fatal_published: false,
        startup_recorded: false,
        node_name,
        disruption_reason,
        pod_status_reason: status.and_then(|s| s.reason.clone()),
        pod_status_message: status.and_then(|s| s.message.clone()),
        created_at_ms,
        scheduling_blocked_published: false,
        startup_slow_published: false,
        storage_fallback_requested: false,
        failed_since_ms: None,
        gc_requested: false,
        auto_stop_pending: false,
        last_published_sandbox_status: None,
    }
}

/// Generate a human-readable message for a sandbox lifecycle event.
pub(super) fn format_sandbox_lifecycle_message(
    lifecycle_event: &str,
    error: Option<&str>,
) -> String {
    match lifecycle_event {
        "pod_ready" => "Ready".to_string(),
        "pod_recovering" => "Recovering from a failed pod".to_string(),
        "container_oom_killed" => "Out of memory (OOMKilled)".to_string(),
        "container_crash_loop" => "Crash-looping".to_string(),
        "container_restarted" => "Container restarted".to_string(),
        "container_terminated" => match error {
            Some(err) => format!("Terminated: {err}"),
            None => "Terminated unexpectedly".to_string(),
        },
        "pod_failed" => "Pod failed".to_string(),
        "pod_evicted" => "Pod evicted".to_string(),
        "node_reclaimed" => match error {
            Some(reason) => format!("Node reclaimed ({reason}) — rescheduling onto a new node"),
            None => "Node reclaimed — rescheduling onto a new node".to_string(),
        },
        "pod_scheduled" => "Scheduled onto a node".to_string(),
        "pod_succeeded" | "sandbox_completed" => "Completed".to_string(),
        "pod_deleted" => "Pod deleted".to_string(),
        other => other.replace('_', " "),
    }
}

#[cfg(test)]
mod tests {
    use super::format_sandbox_lifecycle_message;

    /// The reason reaches the reader, and the message stays storage-neutral: this
    /// formatter cannot tell a PVC-backed replacement that keeps the live tree from an
    /// S3 restore that lags the loss, or from a sandbox with neither.
    #[test]
    fn node_reclaim_names_the_reason_and_promises_nothing() {
        assert_eq!(
            format_sandbox_lifecycle_message("node_reclaimed", Some("DeletionByPodGC")),
            "Node reclaimed (DeletionByPodGC) — rescheduling onto a new node"
        );
        assert_eq!(
            format_sandbox_lifecycle_message("node_reclaimed", None),
            "Node reclaimed — rescheduling onto a new node"
        );
    }
}
