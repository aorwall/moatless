//! The observed state of a pod and its server containers, read off one
//! `Pod` object.

use super::*;

/// Observed state of a server container, used for transition detection.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ServerContainerState {
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) error: Option<String>,
    pub(crate) detail: Option<String>,
    pub(crate) terminated_reason: Option<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) memory_limit: Option<String>,
    pub(crate) running_since: Option<chrono::DateTime<chrono::Utc>>,
    /// Restart count of this server container (used for crash-loop auto-disable).
    pub(crate) restart_count: i32,
}

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
    /// Exit code from the sandbox container's *last* terminated state — survives a
    /// K8s restart so a crash that's already been restarted into Running isn't lost.
    pub(crate) last_exit_code: Option<i32>,
    /// Termination reason from the sandbox container's *last* terminated state.
    pub(crate) last_terminated_reason: Option<String>,
    /// Termination message or waiting message from the sandbox container.
    pub(crate) failure_detail: Option<String>,
    /// Server container snapshots (excludes sandbox container).
    pub(crate) server_containers: Vec<ServerContainerState>,
    /// Whether a fatal event has already been published for this pod.
    /// Suppresses repeated fatal events during `CrashLoopBackOff`.
    pub(super) fatal_published: bool,
    /// Server names already reported as crash-looping during this pod's
    /// lifecycle. Fire-once guard so the watcher publishes one `Failed` event
    /// per container instead of one per watch event while it keeps crashing in
    /// this pod. Cleared naturally when a new pod replaces this map entry.
    pub(super) crash_reported_servers: HashSet<String>,
    /// Whether the `moatless_sandbox_startup_seconds` histogram has already been
    /// recorded for this pod. Carried over so a pod that goes Ready→NotReady→
    /// Ready (e.g. after a container restart) is only timed on its *first* ready,
    /// keeping startup latency from being inflated by re-readies.
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
    /// cause (e.g. `Usage of EmptyDir volume "docker-storage" exceeds the
    /// limit "10Gi"`).
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
    /// Fire-once guard: an attach-failure storage fallback has been requested for
    /// this pod. Keyed per pod, so it is effectively once per generation — a
    /// rebuild replaces the pod, and the fresh pod starts the guard clear. Set
    /// whatever the outcome, so a failed rebuild waits for the next clean start
    /// rather than retrying every sweep.
    pub(crate) storage_fallback_requested: bool,
    /// Epoch ms when this pod was first observed in the terminal `Failed`
    /// phase. K8s never garbage-collects Failed pods (e.g. evicted for
    /// exceeding an emptyDir sizeLimit), so the sweep deletes them once this
    /// is older than the GC grace period.
    pub(crate) failed_since_ms: Option<i64>,
    /// Fire-once guard: a GC delete has been issued for this Failed pod.
    pub(crate) gc_requested: bool,
    /// An auto-stop (scale Deployment to 0) has been decided for this pod — via
    /// the crash-restart threshold crossing or the exit-78 fast-fail — but not
    /// yet confirmed by a successful `scale_deployment` call. Set when the
    /// decision is made, cleared only on a successful scale-to-0, and retried on
    /// every subsequent watcher event and in the periodic sweep. Without this, a
    /// failed scale is lost: `restart_count` stays `>= threshold` so the "new
    /// crossing" edge never re-fires and the pod crash-loops forever.
    pub(super) auto_stop_pending: bool,
    /// Sandbox status most recently published from this pod's lifecycle
    /// transitions, carried across Apply events. Used to suppress consecutive
    /// identical publishes from the high-frequency container `sub_types`
    /// (restart/crash-loop/terminated), which otherwise fire once per K8s
    /// watch Apply while a container flaps. Watcher restart resets this —
    /// acceptable (one extra event per pod after failover).
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

/// Build the observed state from a pod's current status.
/// Record sandbox startup latency (pod creation → first sandbox-container Ready)
/// into the `moatless_sandbox_startup_seconds` histogram.
///
/// Uses the pod's `metadata.creationTimestamp` as the start. Skips pods without
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
    // pod-level `Ready` condition. A crash-looping non-default *server*
    // container keeps the pod-level condition false forever, which would
    // suppress the `pod_ready` → `Ready` transition even though the agent is
    // fine. This mirrors `status_resolver`'s use of `is_sandbox_container_ready`.
    let ready = is_sandbox_container_ready(pod);

    // Build a map of container name → memory limit from pod spec
    let memory_limits: HashMap<String, String> = pod
        .spec
        .as_ref()
        .map(|spec| {
            spec.containers
                .iter()
                .filter_map(|c| {
                    let limit = c.resources.as_ref()?.limits.as_ref()?.get("memory")?;
                    Some((c.name.clone(), limit.0.clone()))
                })
                .collect()
        })
        .unwrap_or_default();

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

    // Extract server container states
    let server_containers = extract_container_observed_statuses(pod)
        .into_iter()
        .map(|cs| {
            let memory_limit = memory_limits.get(&cs.name).cloned();
            ServerContainerState {
                name: cs.name.clone(),
                status: cs
                    .to_server_status_with_timeout(DEFAULT_SERVER_READINESS_TIMEOUT_SECS)
                    .to_string(),
                error: cs.error_reason_with_timeout(DEFAULT_SERVER_READINESS_TIMEOUT_SECS),
                detail: cs.detail_reason(),
                terminated_reason: cs.terminated_reason.clone(),
                exit_code: cs.exit_code,
                memory_limit,
                running_since: cs.running_since,
                restart_count: cs.restart_count,
            }
        })
        .collect();

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
        server_containers,
        fatal_published: false,
        crash_reported_servers: HashSet::new(),
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

/// Generate a human-readable message for a server container event.
///
/// Since `server_name` and `status` are separate fields, this returns only
/// the contextual detail (e.g. "`ContainerCreating`", "Out of memory (limit: 1536Mi)").
pub(super) fn format_server_message(sc: Option<&ServerContainerState>) -> String {
    let sc = match sc {
        Some(s) => s,
        None => return String::new(),
    };

    // Termination reasons take priority
    if let Some(reason) = &sc.terminated_reason {
        return match reason.as_str() {
            "OOMKilled" => match &sc.memory_limit {
                Some(limit) => format!("Out of memory (limit: {limit})"),
                None => "Out of memory (OOMKilled)".to_string(),
            },
            "Completed" => String::new(),
            _ => match sc.exit_code {
                Some(code) => format!("Terminated: {reason} (exit code: {code})"),
                None => format!("Terminated: {reason}"),
            },
        };
    }

    // Fall back to detail, then error
    sc.detail
        .as_deref()
        .or(sc.error.as_deref())
        .unwrap_or("")
        .to_string()
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
        "node_reclaimed" => {
            "Node reclaimed — rescheduling onto a new node (work preserved)".to_string()
        }
        "pod_scheduled" => "Scheduled onto a node".to_string(),
        "pod_succeeded" | "sandbox_completed" => "Completed".to_string(),
        "pod_deleted" => "Pod deleted".to_string(),
        other => other.replace('_', " "),
    }
}
