//! Persistent pod lifecycle watcher.
//!
//! Watches all sandbox pods (label `app.kubernetes.io/managed-by=moatless`) and
//! detects runtime events: OOM, crash loops, evictions, deletions, restarts,
//! and server container status changes.
//!
//! Publishes lifecycle events to Redis Streams for SSE and event handler consumption.
//!
//! Only the leader replica runs the watcher (via K8s Lease election).
//!
//! Pod states are keyed by **pod_name** (not task_id) so that during rolling
//! updates the old terminating pod and the new starting pod are tracked
//! independently. Delete events for old pods check for surviving siblings
//! before publishing `pod_deleted` / setting `NotCreated`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::TryStreamExt;
use k8s_openapi::api::core::v1::{Event as CoreEvent, Pod};
use kube::api::{Api, DeleteParams, ListParams, LogParams};
use kube::runtime::watcher::{self, Config as WatcherConfig};
use moatless_events::{
    SandboxEventPayload, SandboxStatus, ServerEventPayload, ServerSubType, TaskTurnSubType,
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::deployment;
use super::leader_election::LeaderElection;
use super::pod_status::{
    extract_container_observed_statuses, is_sandbox_container_ready,
    DEFAULT_SERVER_READINESS_TIMEOUT_SECS,
};
use crate::events::EventPublisher;
use crate::sandbox::lifecycle::Lifecycle;
use crate::task::dao::{TaskPatch, TaskRepository};
use crate::task::models::SandboxStopReason;
use crate::task_event::dao::TaskEventRepository;
use crate::workspace::dao::WorkspaceRepository;

/// Dependencies the watcher needs to remediate a crash-looping sandbox. The
/// lifecycle owns plan construction and intent, so the watcher only carries the
/// handle plus the task/workspace repos its auto-disable decision reads. All are
/// optional because the watcher also runs in setups without a database (where
/// remediation degrades to publishing a `Failed` event / raw scale-to-0).
#[derive(Clone)]
struct WatcherDeps {
    /// Shared durable lifecycle handle — the sole writer of Sandbox intent. The
    /// watcher drives crash auto-stop (`stop`) and auto-disable redeploy
    /// (`redeploy`) through it. `None` in DB-less setups, where crash mitigation
    /// degrades to a raw scale-to-0 and auto-disable is skipped.
    lifecycle: Option<Arc<Lifecycle>>,
    tasks: Option<Arc<dyn TaskRepository>>,
    workspaces: Option<Arc<dyn WorkspaceRepository>>,
    /// Task-event repo, used to look up the active turn when a pod terminates
    /// involuntarily so `turn_interrupted` carries the right turn context (and
    /// is suppressed when no turn is in flight). `None` in DB-less setups.
    task_events: Option<Arc<dyn TaskEventRepository>>,
}

/// Pod-Pending duration after which the watcher emits `scheduling_blocked`,
/// surfacing an otherwise-silent unschedulable window (no node, quota, etc.).
const SCHEDULING_BLOCKED_THRESHOLD_SECS: i64 = 60;

/// Time a scheduled pod may sit with its sandbox container not yet running
/// before the watcher emits `startup_slow` — makes a long volume-attach or
/// image-pull window (e.g. EBS RWO Multi-Attach) a first-class event.
const STARTUP_SLOW_THRESHOLD_SECS: i64 = 120;

/// Exit code used by the sandbox to signal a permanent init failure (sysexits EX_CONFIG).
const PERMANENT_FAILURE_EXIT_CODE: i32 = 78;

/// Observed state of a server container, used for transition detection.
#[derive(Debug, Clone, PartialEq)]
struct ServerContainerState {
    name: String,
    status: String,
    error: Option<String>,
    detail: Option<String>,
    terminated_reason: Option<String>,
    exit_code: Option<i32>,
    memory_limit: Option<String>,
    running_since: Option<chrono::DateTime<chrono::Utc>>,
    /// Restart count of this server container (used for crash-loop auto-disable).
    restart_count: i32,
}

/// Observed state for a single pod, used for transition detection.
#[derive(Debug, Clone)]
struct PodObservedState {
    task_id: String,
    phase: String,
    /// Restart count of the **sandbox** container specifically.
    restart_count: i32,
    terminated_reason: Option<String>,
    waiting_reason: Option<String>,
    ready: bool,
    pod_name: String,
    /// Exit code from the sandbox container's *current* terminated state.
    exit_code: Option<i32>,
    /// Exit code from the sandbox container's *last* terminated state — survives a
    /// K8s restart so a crash that's already been restarted into Running isn't lost.
    last_exit_code: Option<i32>,
    /// Termination reason from the sandbox container's *last* terminated state.
    last_terminated_reason: Option<String>,
    /// Termination message or waiting message from the sandbox container.
    failure_detail: Option<String>,
    /// Server container snapshots (excludes sandbox container).
    server_containers: Vec<ServerContainerState>,
    /// Whether a fatal event has already been published for this pod.
    /// Suppresses repeated fatal events during CrashLoopBackOff.
    fatal_published: bool,
    /// Server names already auto-disabled during this pod's lifecycle. Fire-once
    /// guard so the watcher doesn't repeatedly redeploy while the old pod is
    /// still reporting the crash-looping container. Cleared naturally when a new
    /// pod replaces this map entry.
    auto_disabled_servers: HashSet<String>,
    /// Whether the `moatless_sandbox_startup_seconds` histogram has already been
    /// recorded for this pod. Carried over so a pod that goes Ready→NotReady→
    /// Ready (e.g. after a container restart) is only timed on its *first* ready,
    /// keeping startup latency from being inflated by re-readies.
    startup_recorded: bool,
    /// Node the pod is scheduled on (`pod.spec.node_name`), once bound. `None`
    /// while the pod is still `Pending` and unscheduled.
    node_name: Option<String>,
    /// Why the pod was disrupted, derived from the `DisruptionTarget` pod
    /// condition reason, falling back to `pod.status.reason`.
    disruption_reason: Option<String>,
    /// Raw pod-level `status.reason`. Kubelet resource evictions (emptyDir
    /// sizeLimit, node pressure) set exactly `"Evicted"` here — the container
    /// states never carry it, so this is the only reliable eviction signal.
    pod_status_reason: Option<String>,
    /// Pod-level `status.message` — for evictions this holds the actionable
    /// cause (e.g. `Usage of EmptyDir volume "docker-storage" exceeds the
    /// limit "10Gi"`).
    pod_status_message: Option<String>,
    /// Pod `creationTimestamp` as epoch milliseconds, used to derive
    /// `pending_seconds` on the Pending→scheduled edge.
    created_at_ms: Option<i64>,
    /// Fire-once guard: a `scheduling_blocked` event has been emitted for this
    /// pod's stuck-Pending window. Carried over so the threshold sweep emits at
    /// most one blocked event per pod.
    scheduling_blocked_published: bool,
    /// Fire-once guard: a `startup_slow` event has been emitted for this pod's
    /// slow-container-start window (e.g. long volume attach).
    startup_slow_published: bool,
    /// Epoch ms when this pod was first observed in the terminal `Failed`
    /// phase. K8s never garbage-collects Failed pods (e.g. evicted for
    /// exceeding an emptyDir sizeLimit), so the sweep deletes them once this
    /// is older than the GC grace period.
    failed_since_ms: Option<i64>,
    /// Fire-once guard: a GC delete has been issued for this Failed pod.
    gc_requested: bool,
    /// An auto-stop (scale Deployment to 0) has been decided for this pod — via
    /// the crash-restart threshold crossing or the exit-78 fast-fail — but not
    /// yet confirmed by a successful `scale_deployment` call. Set when the
    /// decision is made, cleared only on a successful scale-to-0, and retried on
    /// every subsequent watcher event and in the periodic sweep. Without this, a
    /// failed scale is lost: `restart_count` stays `>= threshold` so the "new
    /// crossing" edge never re-fires and the pod crash-loops forever.
    auto_stop_pending: bool,
    /// Sandbox status most recently published from this pod's lifecycle
    /// transitions, carried across Apply events. Used to suppress consecutive
    /// identical publishes from the high-frequency container sub_types
    /// (restart/crash-loop/terminated), which otherwise fire once per K8s
    /// watch Apply while a container flaps. Watcher restart resets this —
    /// acceptable (one extra event per pod after failover).
    last_published_sandbox_status: Option<SandboxStatus>,
}

impl PodObservedState {
    /// Best-known sandbox-container exit code: the current terminated state if
    /// present, otherwise the last terminated state (which survives a restart).
    fn effective_exit_code(&self) -> Option<i32> {
        self.exit_code.or(self.last_exit_code)
    }

    /// Best-known sandbox-container termination reason (current, else last).
    fn effective_terminated_reason(&self) -> Option<String> {
        self.terminated_reason
            .clone()
            .or_else(|| self.last_terminated_reason.clone())
    }
}

/// Spawn the pod lifecycle watcher as a background tokio task.
///
/// The watcher runs in a loop:
/// 1. Try to acquire leadership via K8s Lease.
/// 2. If leader, start watching pods.
/// 3. If not leader, sleep and retry.
/// 4. On watch stream error, re-enter the leadership loop.
#[allow(clippy::too_many_arguments)]
pub fn spawn_pod_lifecycle_watcher(
    k8s: K8sClient,
    event_publisher: Arc<dyn EventPublisher>,
    cancel: CancellationToken,
    max_crash_restart_count: i32,
    lifecycle: Option<Arc<Lifecycle>>,
    tasks: Option<Arc<dyn TaskRepository>>,
    workspaces: Option<Arc<dyn WorkspaceRepository>>,
    task_events: Option<Arc<dyn TaskEventRepository>>,
) -> JoinHandle<()> {
    let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
    metrics::describe_counter!(
        "sandbox_container_terminations_total",
        "Non-intentional sandbox container terminations, labelled by lifecycle_event, K8s reason, and exit_code"
    );
    metrics::describe_histogram!(
        "moatless_sandbox_startup_seconds",
        "Sandbox startup latency in seconds, measured from pod creation to the sandbox container's first Ready"
    );
    let deps = WatcherDeps {
        lifecycle,
        tasks,
        workspaces,
        task_events,
    };

    tokio::spawn(async move {
        let leader_election = LeaderElection::new(&k8s, identity.clone());

        loop {
            if cancel.is_cancelled() {
                info!("Pod watcher shutting down (cancellation)");
                return;
            }

            // Try to acquire or renew leadership
            match leader_election.try_acquire_or_renew().await {
                Ok(true) => {
                    info!(identity = %identity, "Leader acquired, starting pod watch");
                    run_pod_watch(
                        &k8s,
                        event_publisher.as_ref(),
                        &leader_election,
                        &cancel,
                        max_crash_restart_count,
                        &deps,
                    )
                    .await;
                    // Watch ended (stream error or leadership lost) — re-enter loop
                    info!("Pod watch ended, re-entering leadership loop");
                }
                Ok(false) => {
                    debug!(identity = %identity, "Not leader, waiting...");
                }
                Err(e) => {
                    warn!(error = %e, "Leader election error");
                }
            }

            // Wait before retrying (also serves as renew interval for non-leaders)
            tokio::select! {
                () = cancel.cancelled() => {
                    info!("Pod watcher shutting down (cancellation)");
                    return;
                }
                () = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
            }
        }
    })
}

/// Run the pod watch stream until it ends or leadership is lost.
async fn run_pod_watch(
    k8s: &K8sClient,
    event_publisher: &dyn EventPublisher,
    leader_election: &LeaderElection,
    cancel: &CancellationToken,
    max_crash_restart_count: i32,
    deps: &WatcherDeps,
) {
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let wc = WatcherConfig::default().labels("app.kubernetes.io/managed-by=moatless");

    let stream = watcher::watcher(pods_api, wc);
    tokio::pin!(stream);

    let mut pod_states: HashMap<String, PodObservedState> = HashMap::new();
    let mut renew_interval = tokio::time::interval(std::time::Duration::from_secs(5));

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                return;
            }
            _ = renew_interval.tick() => {
                // Renew leadership periodically
                match leader_election.try_acquire_or_renew().await {
                    Ok(true) => {} // Still leader
                    Ok(false) => {
                        warn!("Lost leadership, stopping pod watch");
                        return;
                    }
                    Err(e) => {
                        warn!(error = %e, "Leader renewal failed, stopping pod watch");
                        return;
                    }
                }
                // Time-based sweep: the watch stream only fires on pod changes, so
                // a pod that sits Pending or ContainerCreating for minutes produces
                // no events. This tick turns those silent windows into
                // scheduling_blocked / startup_slow events (fire-once per pod).
                sweep_pending_pods(k8s, event_publisher, &mut pod_states).await;
                gc_failed_pods(k8s, &mut pod_states).await;
                retry_pending_auto_stops(k8s, &mut pod_states, deps).await;
            }
            event = stream.try_next() => {
                match event {
                    Ok(Some(watcher_event)) => {
                        // Boxed: the handler embeds the (large) auto-disable +
                        // redeploy path; keep this loop's future small.
                        Box::pin(process_watcher_event(
                            k8s,
                            &watcher_event,
                            &mut pod_states,
                            event_publisher,
                            max_crash_restart_count,
                            deps,
                        )).await;
                    }
                    Ok(None) => {
                        info!("Pod watch stream ended");
                        return;
                    }
                    Err(e) => {
                        warn!(error = %e, "Pod watch stream error");
                        return;
                    }
                }
            }
        }
    }
}

/// Return sibling pod states for a given task_id, excluding a specific pod.
fn sibling_pods_for_task<'a>(
    pod_states: &'a HashMap<String, PodObservedState>,
    task_id: &str,
    exclude_pod: &str,
) -> Vec<&'a PodObservedState> {
    pod_states
        .values()
        .filter(|s| s.task_id == task_id && s.pod_name != exclude_pod)
        .collect()
}

/// Process a single watcher event and detect state transitions.
#[allow(clippy::too_many_arguments)]
async fn process_watcher_event(
    k8s: &K8sClient,
    event: &watcher::Event<Pod>,
    pod_states: &mut HashMap<String, PodObservedState>,
    event_publisher: &dyn EventPublisher,
    max_crash_restart_count: i32,
    deps: &WatcherDeps,
) {
    match event {
        watcher::Event::Apply(pod) | watcher::Event::InitApply(pod) => {
            let is_init = matches!(event, watcher::Event::InitApply(_));
            let task_id = if let Some(id) = extract_task_id(pod) {
                id
            } else {
                let name = pod.metadata.name.as_deref().unwrap_or("unknown");
                warn!(
                    pod_name = name,
                    "Pod has no moatless.io/task-id label — skipping"
                );
                return;
            };
            let pod_name = pod
                .metadata
                .name
                .as_deref()
                .unwrap_or("unknown")
                .to_string();

            let mut new_state = extract_pod_observed_state(pod, &pod_name, &task_id);

            // During InitApply, populate the map silently without publishing events
            if is_init {
                if new_state.phase == "Failed" {
                    // Corpse predating this watcher — start its GC clock now.
                    new_state.failed_since_ms =
                        Some(k8s_openapi::jiff::Timestamp::now().as_millisecond());
                }
                pod_states.insert(pod_name, new_state);
                return;
            }

            let prev_state = pod_states.get(&pod_name);
            let is_new_pod = prev_state.is_none();

            // Cross-pod recovery: new pod appearing while a sibling is in Failed/Unknown
            if is_new_pod {
                let siblings = sibling_pods_for_task(pod_states, &task_id, &pod_name);
                let has_failed_sibling = siblings
                    .iter()
                    .any(|s| matches!(s.phase.as_str(), "Failed" | "Unknown"));

                if has_failed_sibling {
                    info!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        "New pod replacing failed sibling — emitting pod_recovering"
                    );
                    event_publisher
                        .publish_sandbox_event_with_payload(
                            &task_id,
                            SandboxStatus::Initializing,
                            &SandboxEventPayload {
                                message: "Recovering from a failed pod".to_string(),
                                lifecycle_event: Some("pod_recovering".to_string()),
                                ..Default::default()
                            },
                        )
                        .await;
                }
            }

            // Detect intra-pod transitions (same pod, state changed)
            let transitions = detect_transitions(prev_state, &new_state);

            // Detect server container transitions
            let server_transitions = detect_server_transitions(prev_state, &new_state);

            // Publish server container events as typed `server` events
            for (server_sub_type, server_name, _status) in &server_transitions {
                let sc = new_state.server_containers.iter().find(|s| {
                    let n = s.name.strip_suffix("-server").unwrap_or(&s.name);
                    n == server_name
                });

                info!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    server = %server_name,
                    event = ?server_sub_type,
                    error = ?sc.and_then(|s| s.error.as_deref()),
                    detail = ?sc.and_then(|s| s.detail.as_deref()),
                    "Server container status changed"
                );

                event_publisher
                    .publish_server_event(
                        &task_id,
                        *server_sub_type,
                        &ServerEventPayload {
                            server_name: server_name.clone(),
                            status: *server_sub_type,
                            message: format_server_message(sc),
                        },
                    )
                    .await;
            }

            // Check if this is an intentional stop (Deployment scaled to 0).
            // When the idle reaper or user explicitly stops a sandbox, the pod
            // terminates with exit code 137 (SIGKILL) which would otherwise be
            // misinterpreted as a crash.
            let has_error_transition = transitions.iter().any(|(sub_type, _)| {
                matches!(
                    lifecycle_sub_type_to_sandbox_status(sub_type),
                    SandboxStatus::Error
                )
            });

            let is_intentional_stop = if has_error_transition {
                match deployment::get_deployment_replicas(k8s, &task_id).await {
                    Ok((desired, _)) => desired == 0,
                    Err(e) => {
                        debug!(task_id = %task_id, error = %e, "Could not check deployment replicas");
                        false
                    }
                }
            } else {
                false
            };

            // If sandbox entered an error state AND this is NOT an intentional stop,
            // force all servers to Failed (pod crash kills server containers too).
            if has_error_transition && !is_intentional_stop {
                for sc in &new_state.server_containers {
                    let server_name = sc.name.strip_suffix("-server").unwrap_or(&sc.name);
                    // Skip servers already reported as failed by detect_server_transitions
                    let already_failed = server_transitions
                        .iter()
                        .any(|(st, name, _)| name == server_name && *st == ServerSubType::Failed);
                    if already_failed {
                        continue;
                    }
                    info!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        server = %server_name,
                        "Forcing server to Failed due to error-level pod transition"
                    );
                    event_publisher
                        .publish_server_event(
                            &task_id,
                            ServerSubType::Failed,
                            &ServerEventPayload {
                                server_name: server_name.to_string(),
                                status: ServerSubType::Failed,
                                message: sc
                                    .error
                                    .clone()
                                    .unwrap_or_else(|| "Pod terminated".to_string()),
                            },
                        )
                        .await;
                }
            }

            // Carried across Apply events so a flapping container (one watch
            // event per restart) publishes each status once, not once per Apply.
            new_state.last_published_sandbox_status =
                prev_state.and_then(|p| p.last_published_sandbox_status);

            for (sub_type, error_msg) in &transitions {
                // For intentional stops, downgrade error-level transitions to Stopped
                let sandbox_status = if is_intentional_stop
                    && matches!(
                        lifecycle_sub_type_to_sandbox_status(sub_type),
                        SandboxStatus::Error
                    ) {
                    SandboxStatus::Stopped
                } else {
                    lifecycle_sub_type_to_sandbox_status(sub_type)
                };

                let is_error_event = matches!(sandbox_status, SandboxStatus::Error);
                if is_error_event && !is_intentional_stop {
                    // A non-intentional error-level termination is a real failure.
                    // Count it (labelled for OOM-vs-crash slicing) and log at warn!
                    // so it surfaces in alerting, not buried in info noise.
                    metrics::counter!(
                        "sandbox_container_terminations_total",
                        "lifecycle_event" => sub_type.clone(),
                        "reason" => new_state
                            .effective_terminated_reason()
                            .unwrap_or_else(|| "unknown".to_string()),
                        "exit_code" => new_state
                            .effective_exit_code()
                            .map_or_else(|| "unknown".to_string(), |c| c.to_string()),
                    )
                    .increment(1);
                    warn!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        lifecycle_event = %sub_type,
                        sandbox_status = %sandbox_status,
                        error = ?error_msg,
                        exit_code = ?new_state.effective_exit_code(),
                        terminated_reason = ?new_state.effective_terminated_reason(),
                        failure_detail = ?new_state.failure_detail,
                        "Sandbox container terminated unexpectedly"
                    );
                } else {
                    info!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        lifecycle_event = %sub_type,
                        sandbox_status = %sandbox_status,
                        error = ?error_msg,
                        exit_code = ?new_state.exit_code,
                        failure_detail = ?new_state.failure_detail,
                        intentional_stop = is_intentional_stop,
                        "Pod lifecycle event detected"
                    );
                }

                // Metrics and logs above still record every detected edge —
                // only the event publish is deduplicated.
                if should_skip_duplicate_publish(
                    sub_type,
                    sandbox_status,
                    new_state.last_published_sandbox_status,
                ) {
                    debug!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        lifecycle_event = %sub_type,
                        sandbox_status = %sandbox_status,
                        "Skipping duplicate sandbox status publish"
                    );
                    continue;
                }

                // Involuntary eviction is inherently node-initiated — tag it so
                // the frontend can render an amber "recovering" chip. Other
                // intentional-stop initiators are set at their explicit publish
                // sites (reaper/handlers/provider), which the watcher can't see.
                let initiator = if sub_type == "pod_evicted" && !is_intentional_stop {
                    Some("eviction".to_string())
                } else {
                    None
                };

                // Time spent Pending before scheduling, surfaced on the edge event.
                let pending_seconds = if sub_type == "pod_scheduled" {
                    new_state.created_at_ms.map(|created| {
                        let now = k8s_openapi::jiff::Timestamp::now().as_millisecond();
                        (now - created).max(0) / 1000
                    })
                } else {
                    None
                };

                event_publisher
                    .publish_sandbox_event_with_payload(
                        &task_id,
                        sandbox_status,
                        &SandboxEventPayload {
                            message: format_sandbox_lifecycle_message(
                                sub_type,
                                error_msg.as_deref(),
                            ),
                            lifecycle_event: Some(sub_type.clone()),
                            error: if is_intentional_stop {
                                None
                            } else {
                                error_msg.clone()
                            },
                            // Carry the exit code / K8s reason so consumers can tell
                            // OOM (137) from a crash (1) without scraping pod state.
                            // Ride these on intentional stops too (Phase 3) — the
                            // exit code is diagnostic, not an error signal.
                            exit_code: new_state.effective_exit_code(),
                            terminated_reason: new_state.effective_terminated_reason(),
                            // Node-level correlation + disruption cause on every event.
                            node_name: new_state.node_name.clone(),
                            disruption_reason: new_state.disruption_reason.clone(),
                            initiator,
                            pending_seconds,
                        },
                    )
                    .await;
                new_state.last_published_sandbox_status = Some(sandbox_status);
            }

            // Carry over fatal_published from previous state, then reset on recovery.
            // startup_recorded is sticky (never reset) so re-readies aren't re-timed.
            if let Some(prev) = prev_state {
                new_state.fatal_published = prev.fatal_published;
                new_state.auto_disabled_servers = prev.auto_disabled_servers.clone();
                new_state.startup_recorded = prev.startup_recorded;
                // Sticky fire-once guards for the threshold sweep, so re-observing
                // a still-stuck pod doesn't re-emit its blocked/slow event.
                new_state.scheduling_blocked_published = prev.scheduling_blocked_published;
                new_state.startup_slow_published = prev.startup_slow_published;
                // Carry a still-pending auto-stop so a scale-to-0 that previously
                // failed is retried on this event (see the auto-stop block below).
                new_state.auto_stop_pending = prev.auto_stop_pending;
            }
            // Start (or carry over) the GC clock for pods in the terminal
            // Failed phase — K8s never removes them on its own.
            if new_state.phase == "Failed" {
                new_state.failed_since_ms = prev_state
                    .and_then(|p| p.failed_since_ms)
                    .or_else(|| Some(k8s_openapi::jiff::Timestamp::now().as_millisecond()));
                new_state.gc_requested = prev_state.is_some_and(|p| p.gc_requested);
            }
            if transitions.iter().any(|(sub, _)| sub == "pod_ready") {
                new_state.fatal_published = false;
                // Time pod-creation → first Ready once per pod.
                if !new_state.startup_recorded {
                    record_sandbox_startup(pod);
                    new_state.startup_recorded = true;
                }
            }

            // Only truly irrecoverable events are fatal — pod-level failures
            // where K8s will NOT restart the container.
            // Container-level crashes (OOM, terminated, crash loop) are not fatal
            // because K8s will restart them. The auto-stop threshold handles
            // persistent crash loops separately.
            let has_fatal = transitions
                .iter()
                .any(|(sub_type, _)| matches!(sub_type.as_str(), "pod_failed" | "pod_evicted"));

            if has_fatal && !is_intentional_stop && !new_state.fatal_published {
                let error_msg = transitions
                    .iter()
                    .find_map(|(_, err)| err.as_ref())
                    .cloned()
                    .unwrap_or_else(|| "Sandbox terminated unexpectedly".to_string());

                let exit_code = new_state.effective_exit_code();
                let terminated_reason = new_state.effective_terminated_reason();

                // Fetch container logs for root cause. Previously logs were only
                // captured for exit-78 and the crash-restart threshold, so a
                // one-off fatal pod failure (e.g. the agent container terminating
                // with reason "Error" mid-turn) left no trace.
                let logs = fetch_sandbox_logs(k8s, &pod_name, 50).await;

                metrics::counter!(
                    "sandbox_container_terminations_total",
                    "lifecycle_event" => "fatal".to_string(),
                    "reason" => terminated_reason
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    "exit_code" => exit_code
                        .map_or_else(|| "unknown".to_string(), |c| c.to_string()),
                )
                .increment(1);
                warn!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    exit_code = ?exit_code,
                    terminated_reason = ?terminated_reason,
                    failure_detail = ?new_state.failure_detail,
                    "Sandbox pod failed irrecoverably"
                );

                let mut parts = vec![format!("Sandbox terminated: {error_msg}")];
                if let Some(code) = exit_code {
                    parts.push(format!("Exit code: `{code}`"));
                }
                if let Some(ref reason) = terminated_reason {
                    parts.push(format!("Reason: `{reason}`"));
                }
                // Name the node-level cause (spot reclaim, drain, preemption) in
                // the message itself, not just the payload chip, so the root cause
                // is legible wherever the message is read.
                if let Some(ref disruption) = new_state.disruption_reason {
                    parts.push(format!("Disruption: `{disruption}`"));
                }
                if let Some(ref logs) = logs {
                    parts.push(format!("Container logs:\n```\n{logs}\n```"));
                }
                let detailed_message = parts.join("\n");

                event_publisher
                    .publish_sandbox_event_with_payload(
                        &task_id,
                        SandboxStatus::Error,
                        &SandboxEventPayload {
                            message: detailed_message,
                            lifecycle_event: Some("fatal".to_string()),
                            error: Some(error_msg),
                            exit_code,
                            terminated_reason,
                            node_name: new_state.node_name.clone(),
                            disruption_reason: new_state.disruption_reason.clone(),
                            ..Default::default()
                        },
                    )
                    .await;

                // If a turn is in flight, surface the interruption immediately as
                // a taskTurn fact (with cause) instead of waiting up to ~5 min for
                // the reaper's delayed `reconciled_orphaned_turn`.
                let cause = new_state
                    .disruption_reason
                    .clone()
                    .or_else(|| new_state.effective_terminated_reason());
                emit_turn_interrupted_if_active(deps, event_publisher, &task_id, cause.as_deref())
                    .await;

                new_state.fatal_published = true;
            }

            // Immediate fail for permanent init errors.
            // Exit code 78 signals the sandbox detected an unrecoverable failure
            // (repo not found, auth error, etc.) — no point retrying.
            let has_terminated = transitions
                .iter()
                .any(|(sub, _)| sub == "container_terminated");
            if has_terminated
                && !is_intentional_stop
                && new_state.exit_code == Some(PERMANENT_FAILURE_EXIT_CODE)
            {
                warn!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    exit_code = ?new_state.exit_code,
                    failure_detail = ?new_state.failure_detail,
                    "Permanent init failure (exit 78), auto-stopping immediately"
                );

                let logs = fetch_sandbox_logs(k8s, &pod_name, 50).await;

                let mut error_parts = vec!["Sandbox stopped: permanent init failure.".to_string()];
                if let Some(ref detail) = new_state.failure_detail {
                    error_parts.push(format!("Error: `{detail}`"));
                }
                if let Some(ref logs) = logs {
                    error_parts.push(format!("Container logs:\n```\n{logs}\n```"));
                }
                let detailed_error = error_parts.join("\n");

                event_publisher
                    .publish_sandbox_event_with_payload(
                        &task_id,
                        SandboxStatus::Error,
                        &SandboxEventPayload {
                            message: detailed_error.clone(),
                            lifecycle_event: Some("auto_stopped".to_string()),
                            error: Some(detailed_error),
                            exit_code: new_state.effective_exit_code(),
                            terminated_reason: new_state.effective_terminated_reason(),
                            initiator: Some("crash_autostop".to_string()),
                            node_name: new_state.node_name.clone(),
                            disruption_reason: new_state.disruption_reason.clone(),
                            ..Default::default()
                        },
                    )
                    .await;

                // Mark the auto-stop as pending; the shared block below performs
                // (and retries) the scale-to-0 so a transient failure isn't lost.
                new_state.auto_stop_pending = true;
            }

            // Auto-stop sandbox if restart count exceeds threshold
            if max_crash_restart_count > 0
                && new_state.restart_count >= max_crash_restart_count
                && !is_intentional_stop
            {
                let prev_below_threshold =
                    prev_state.is_none_or(|p| p.restart_count < max_crash_restart_count);

                if prev_below_threshold {
                    warn!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        restart_count = new_state.restart_count,
                        threshold = max_crash_restart_count,
                        "Restart count exceeded threshold, auto-stopping sandbox"
                    );

                    // Fetch container logs for root cause before stopping
                    let logs = fetch_sandbox_logs(k8s, &pod_name, 50).await;

                    // Build detailed error message with code blocks for technical details
                    let mut error_parts = vec![format!(
                        "Sandbox auto-stopped after {} restarts (threshold: {}).",
                        new_state.restart_count, max_crash_restart_count
                    )];
                    if let Some(exit_code) = new_state.exit_code {
                        error_parts.push(format!("Exit code: `{exit_code}`"));
                    }
                    if let Some(ref detail) = new_state.failure_detail {
                        error_parts.push(format!("Failure: `{detail}`"));
                    }
                    if let Some(ref logs) = logs {
                        error_parts.push(format!("Container logs:\n```\n{logs}\n```"));
                    }
                    let detailed_error = error_parts.join("\n");

                    // Publish detailed sandbox.fatal with root cause
                    event_publisher
                        .publish_sandbox_event_with_payload(
                            &task_id,
                            SandboxStatus::Error,
                            &SandboxEventPayload {
                                message: detailed_error.clone(),
                                lifecycle_event: Some("auto_stopped".to_string()),
                                error: Some(detailed_error),
                                exit_code: new_state.effective_exit_code(),
                                terminated_reason: new_state.effective_terminated_reason(),
                                initiator: Some("crash_autostop".to_string()),
                                node_name: new_state.node_name.clone(),
                                disruption_reason: new_state.disruption_reason.clone(),
                                ..Default::default()
                            },
                        )
                        .await;

                    // Mark the auto-stop as pending; the shared block below does
                    // the scale-to-0 and retries it if it fails.
                    new_state.auto_stop_pending = true;
                }
            }

            // Perform (or retry) a pending auto-stop scale-to-0. Both the exit-78
            // fast-fail and the crash-restart threshold set `auto_stop_pending`;
            // this block is the single point that scales the Deployment to 0 and
            // clears the flag only on success. A failed scale keeps the flag set,
            // so the next watcher event for this pod — or the periodic sweep —
            // retries. Without this retry the crossing edge never re-fires
            // (`restart_count` stays `>= threshold`) and the pod crash-loops
            // forever.
            if new_state.auto_stop_pending
                && !is_intentional_stop
                && crash_autostop(deps, k8s, &task_id).await
            {
                info!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    "Crash auto-stop succeeded"
                );
                new_state.auto_stop_pending = false;
            }

            // Reconcile a clean sandbox exit (Completed, exit 0) against the
            // Deployment's desired replicas.
            //
            // The sandbox's gRPC server never exits on its own during a task —
            // it only returns exit 0 via the SIGTERM graceful-shutdown driver
            // (see sandbox/src/shutdown.rs). So a `sandbox_completed` means "the
            // pod got a SIGTERM", which has two very different causes:
            //   * Intentional stop (idle reaper / user / crash auto-stop) already
            //     scaled the Deployment to 0 — the SIGTERM is the pod draining.
            //     Desired replicas == 0. Scaling to 0 again is a harmless no-op.
            //   * Involuntary node event (spot reclaim, drain, consolidation)
            //     SIGTERMs a pod the Deployment still wants (desired == 1). Here
            //     K8s will reschedule the pod and agent recovery resumes the
            //     in-flight turn — UNLESS we scale to 0 and cancel the task.
            //
            // So we must only "confirm" the stop when the Deployment already
            // desires 0. When it still desires a replica, leave it untouched and
            // let K8s reschedule. A genuine exit-0 restart loop is still caught
            // by the restart-count auto-stop above.
            let has_completed = transitions
                .iter()
                .any(|(sub, _)| sub == "sandbox_completed");
            if has_completed {
                let desired_zero = match deployment::get_deployment_replicas(k8s, &task_id).await {
                    Ok((desired, _)) => desired == 0,
                    Err(e) => {
                        // Fail safe: if we can't tell, do NOT scale down — a
                        // reschedule after eviction is recoverable, a wrongly
                        // cancelled active task is not.
                        warn!(
                            task_id = %task_id,
                            error = %e,
                            "Could not read deployment replicas on clean exit; leaving deployment untouched"
                        );
                        false
                    }
                };
                if desired_zero {
                    info!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        "Sandbox exited cleanly after an intentional stop, confirming scale to 0"
                    );
                    if let Err(e) = deployment::scale_deployment(k8s, &task_id, 0).await {
                        warn!(
                            task_id = %task_id,
                            error = %e,
                            "Failed to scale down completed sandbox"
                        );
                    }
                } else {
                    info!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        "Sandbox exited cleanly while deployment still desires a replica; treating as involuntary eviction and leaving it to K8s to reschedule"
                    );
                }
            }

            // Auto-disable a crash-looping *server* container so a single broken
            // preview server doesn't gate readiness / poison the whole sandbox.
            // The default server is never disabled (only surfaced as Failed).
            if max_crash_restart_count > 0 && !is_intentional_stop {
                // Identify server containers that just crossed the restart
                // threshold and haven't already been auto-disabled this pod.
                let candidates: Vec<ServerContainerState> = new_state
                    .server_containers
                    .iter()
                    .filter(|sc| sc.restart_count >= max_crash_restart_count)
                    .filter(|sc| {
                        let name = sc.name.strip_suffix("-server").unwrap_or(&sc.name);
                        !new_state.auto_disabled_servers.contains(name)
                    })
                    .filter(|sc| {
                        // Crossing guard: same container was below threshold before
                        // (or this is a freshly-observed pod).
                        prev_state
                            .and_then(|p| {
                                p.server_containers.iter().find(|p_sc| p_sc.name == sc.name)
                            })
                            .is_none_or(|p_sc| p_sc.restart_count < max_crash_restart_count)
                    })
                    .cloned()
                    .collect();

                for sc in candidates {
                    let server_name = sc
                        .name
                        .strip_suffix("-server")
                        .unwrap_or(&sc.name)
                        .to_string();
                    auto_disable_server(
                        &task_id,
                        &server_name,
                        &sc,
                        event_publisher,
                        max_crash_restart_count,
                        deps,
                    )
                    .await;
                    // Fire-once for this pod regardless of outcome.
                    new_state.auto_disabled_servers.insert(server_name);
                }
            }

            pod_states.insert(pod_name, new_state);
        }
        watcher::Event::Delete(pod) => {
            let task_id = if let Some(id) = extract_task_id(pod) {
                id
            } else {
                let name = pod.metadata.name.as_deref().unwrap_or("unknown");
                warn!(
                    pod_name = name,
                    "Deleted pod has no moatless.io/task-id label — skipping"
                );
                return;
            };
            let pod_name = pod.metadata.name.as_deref().unwrap_or("unknown");

            // Capture server state BEFORE removing from pod_states
            let deleted_pod_servers: Vec<ServerContainerState> = pod_states
                .get(pod_name)
                .map(|s| s.server_containers.clone())
                .unwrap_or_default();

            // Remove the specific pod entry
            pod_states.remove(pod_name);

            // Check if sibling pods for this task still exist
            let siblings = sibling_pods_for_task(pod_states, &task_id, pod_name);

            if siblings.is_empty() {
                // Distinguish Stopped vs Removed vs Recovering. A Deployment
                // that still desires replicas is replacing this pod after an
                // involuntary disruption, so publishing `stopped` here would
                // incorrectly make a recoverable eviction look intentional.
                let sandbox_status = match deployment::find_deployment(k8s, &task_id).await {
                    Ok(Some(dep)) => {
                        let desired = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
                        if desired == 0 {
                            info!(
                                task_id = %task_id,
                                pod_name = %pod_name,
                                "Pod deleted (Deployment scaled to 0) — publishing stopped"
                            );
                            Some(SandboxStatus::Stopped)
                        } else {
                            info!(
                                task_id = %task_id,
                                pod_name = %pod_name,
                                desired_replicas = desired,
                                "Pod deleted while Deployment still desires replicas — treating as recovering"
                            );
                            None
                        }
                    }
                    Ok(None) => {
                        // No Deployment → fully cleaned up
                        info!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            "Pod deleted (no Deployment) — publishing removed"
                        );
                        Some(SandboxStatus::Removed)
                    }
                    Err(e) => {
                        warn!(
                            task_id = %task_id,
                            error = %e,
                            "Failed to check Deployment on pod delete, defaulting to Removed"
                        );
                        Some(SandboxStatus::Removed)
                    }
                };

                let Some(sandbox_status) = sandbox_status else {
                    return;
                };

                // Publish server events for each server in the deleted pod
                let server_sub = if sandbox_status == SandboxStatus::Removed {
                    ServerSubType::Failed // Unexpected deletion
                } else {
                    ServerSubType::Stopped // Intentional stop
                };

                for sc in &deleted_pod_servers {
                    let server_name = sc.name.strip_suffix("-server").unwrap_or(&sc.name);
                    info!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        server = %server_name,
                        event = ?server_sub,
                        "Publishing server event for deleted pod"
                    );
                    event_publisher
                        .publish_server_event(
                            &task_id,
                            server_sub,
                            &ServerEventPayload {
                                server_name: server_name.to_string(),
                                status: server_sub,
                                message: if sandbox_status == SandboxStatus::Removed {
                                    "Pod deleted".to_string()
                                } else {
                                    String::new()
                                },
                            },
                        )
                        .await;
                }

                let delete_message = if sandbox_status == SandboxStatus::Stopped {
                    "Stopped"
                } else {
                    "Removed"
                };
                event_publisher
                    .publish_sandbox_event_with_payload(
                        &task_id,
                        sandbox_status,
                        &SandboxEventPayload {
                            message: delete_message.to_string(),
                            lifecycle_event: Some("pod_deleted".to_string()),
                            ..Default::default()
                        },
                    )
                    .await;

                // No sandbox.fatal here — "no Deployment + pod deleted" is always
                // intentional cleanup (the cleanup handler deletes the Deployment first).
                // Fatal events only come from the Apply handler for truly irrecoverable
                // transitions (pod_failed, pod_evicted).
            } else {
                info!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    sibling_count = siblings.len(),
                    "Old pod deleted but siblings exist — skipping NotCreated"
                );
            }
        }
        watcher::Event::Init => {
            // Watcher re-syncing — clear stale entries so InitApply repopulates
            pod_states.clear();
        }
        watcher::Event::InitDone => {
            info!(
                pod_count = pod_states.len(),
                "Pod watcher initial sync complete"
            );
        }
    }
}

/// Publish a `server` `Failed` event for a named server.
async fn publish_server_failed(
    event_publisher: &dyn EventPublisher,
    task_id: &str,
    server_name: &str,
    message: String,
) {
    event_publisher
        .publish_server_event(
            task_id,
            ServerSubType::Failed,
            &ServerEventPayload {
                server_name: server_name.to_string(),
                status: ServerSubType::Failed,
                message,
            },
        )
        .await;
}

/// Auto-disable a crash-looping server container for a task.
///
/// - The **default** server is never disabled — only surfaced as `Failed` —
///   because removing it would leave the sandbox without a primary preview.
/// - Without DB access (e.g. Docker mode), this degrades to publishing a
///   `Failed` event so the UI reflects the broken server.
/// - Otherwise the server name is persisted into `tasks.disabled_servers`
///   (so it stays gone across stop/start) and the Deployment is rebuilt
///   without the container.
#[allow(clippy::too_many_arguments)]
async fn auto_disable_server(
    task_id: &str,
    server_name: &str,
    sc: &ServerContainerState,
    event_publisher: &dyn EventPublisher,
    max_crash_restart_count: i32,
    deps: &WatcherDeps,
) {
    warn!(
        task_id = %task_id,
        server = %server_name,
        restart_count = sc.restart_count,
        threshold = max_crash_restart_count,
        "Server container exceeded crash-loop threshold"
    );

    let failed_message = format!(
        "Server auto-disabled after {} restarts (threshold: {})",
        sc.restart_count, max_crash_restart_count
    );

    // Need DB access to determine default-ness and persist the disabled set; the
    // lifecycle owns the redeploy plan repositories.
    let (Some(tasks), Some(workspaces)) = (deps.tasks.as_ref(), deps.workspaces.as_ref()) else {
        debug!(
            task_id = %task_id,
            "No DB access in watcher — surfacing server Failed without auto-disable"
        );
        publish_server_failed(event_publisher, task_id, server_name, failed_message).await;
        return;
    };

    // Never disable the default server — surface Failed and stop.
    let is_default = crate::sandbox::auto_provision::get_server_configs_for_task(
        tasks.as_ref(),
        workspaces.as_ref(),
        task_id,
    )
    .await
    .iter()
    .find(|s| s.name == server_name)
    .is_some_and(|s| s.default);

    if is_default {
        info!(
            task_id = %task_id,
            server = %server_name,
            "Crash-looping server is the default — not disabling, surfacing Failed only"
        );
        publish_server_failed(
            event_publisher,
            task_id,
            server_name,
            format!(
                "Default server crash-looping after {} restarts",
                sc.restart_count
            ),
        )
        .await;
        return;
    }

    // Merge the server into the per-task disabled set and persist.
    let mut disabled = match tasks.find_by_id_internal(task_id).await {
        Ok(Some(task)) => crate::sandbox::auto_provision::parse_disabled_servers(&task),
        Ok(None) => {
            warn!(task_id = %task_id, "Task not found while auto-disabling server");
            return;
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "DB error reading task for auto-disable");
            return;
        }
    };
    if !disabled.iter().any(|s| s == server_name) {
        disabled.push(server_name.to_string());
    }

    let patch = TaskPatch {
        disabled_servers: Some(disabled),
        ..Default::default()
    };
    if let Err(e) = tasks.update_task(task_id, &patch).await {
        warn!(task_id = %task_id, error = %e, "Failed to persist disabled_servers");
        return;
    }

    // Notify the UI: the server is dead and the sandbox is being rebuilt.
    publish_server_failed(event_publisher, task_id, server_name, failed_message).await;
    event_publisher
        .publish_sandbox_event_with_payload(
            task_id,
            SandboxStatus::Initializing,
            &SandboxEventPayload {
                message: format!("Disabled crash-looping server '{server_name}' and redeploying"),
                lifecycle_event: Some("server_auto_disabled".to_string()),
                ..Default::default()
            },
        )
        .await;

    // Redeploy through the lifecycle: it rebuilds the plan (now filtering the
    // disabled server), threads the Running intent + generation, and applies the
    // fresh pod spec. `disabled_servers` is already persisted above, so the
    // rebuilt plan drops the crash-looping server. The lifecycle handle is always
    // present when DB access is (the branch above already required `tasks`).
    let Some(lifecycle) = deps.lifecycle.as_ref() else {
        warn!(task_id = %task_id, "No lifecycle handle — skipping redeploy after auto-disable");
        return;
    };
    if let Err(e) = lifecycle.redeploy(task_id).await {
        warn!(task_id = %task_id, error = %e, "Failed to redeploy after auto-disabling server");
    } else {
        info!(
            task_id = %task_id,
            server = %server_name,
            "Auto-disabled crash-looping server and redeployed"
        );
    }
}

/// Stop a crash-looping sandbox. Through the lifecycle (Stopped/Error intent +
/// graceful stop + scale-to-0) when a DB-backed handle is available; otherwise a
/// raw scale-to-0 so the crash loop still halts in DB-less setups. Returns
/// whether the stop landed, so callers can clear their pending-retry flag.
async fn crash_autostop(deps: &WatcherDeps, k8s: &K8sClient, task_id: &str) -> bool {
    match deps.lifecycle.as_ref() {
        Some(lifecycle) => match lifecycle.stop(task_id, SandboxStopReason::Error).await {
            Ok(_) => true,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Crash auto-stop via lifecycle failed; will retry");
                false
            }
        },
        None => match deployment::scale_deployment(k8s, task_id, 0).await {
            Ok(()) => true,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Crash auto-stop scale-to-0 failed; will retry");
                false
            }
        },
    }
}

/// Extract the task ID from a pod's labels.
fn extract_task_id(pod: &Pod) -> Option<String> {
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
fn record_sandbox_startup(pod: &Pod) {
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

fn extract_pod_observed_state(pod: &Pod, pod_name: &str, task_id: &str) -> PodObservedState {
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
        auto_disabled_servers: HashSet::new(),
        startup_recorded: false,
        node_name,
        disruption_reason,
        pod_status_reason: status.and_then(|s| s.reason.clone()),
        pod_status_message: status.and_then(|s| s.message.clone()),
        created_at_ms,
        scheduling_blocked_published: false,
        startup_slow_published: false,
        failed_since_ms: None,
        gc_requested: false,
        auto_stop_pending: false,
        last_published_sandbox_status: None,
    }
}

/// Whether an eviction's kubelet `status.message` names a graceful
/// node-shutdown or drain cause rather than resource pressure.
///
/// Resource-pressure evictions (emptyDir sizeLimit exceeded, node disk/memory
/// pressure) recur until the cause is fixed and wipe ephemeral data, so they
/// surface as a fatal `pod_evicted` even when the container drained cleanly.
/// Graceful node-shutdown / drain evictions instead name a shutdown/drain
/// cause; K8s reschedules them and #268 keeps them silent on clean exit.
///
/// An absent message is treated as resource pressure (returns `false`): a bare
/// pod-level `status.reason == "Evicted"` is only ever set by the kubelet under
/// resource pressure, so it should be surfaced.
/// Whether a disruption reason — from either the pod-level `status.reason` or
/// the `DisruptionTarget` condition — means the pod's *node* went away rather
/// than the container itself failing.
///
/// Covers the involuntary node-removal family: pod-GC of a pod orphaned by a
/// deleted node (`DeletionByPodGC`), the node controller losing/shutting down a
/// node (`NodeLost` / `NodeShutdown` / `Terminated`), and a NoExecute taint
/// draining the node during a spot reclaim or autoscaler consolidation
/// (`DeletionByTaintManager`). K8s reschedules all of these onto a new node, so
/// they are a routine restart (`node_reclaimed`), never a fatal error.
///
/// Deliberately excludes `PreemptionByScheduler` (a higher-priority pod bumping
/// this one — a scheduling decision, not a node loss) and `TerminationByKubelet`
/// (node resource pressure — handled by the eviction path so recurring evictions
/// still surface).
fn is_node_removal_reason(reason: &str) -> bool {
    matches!(
        reason,
        "DeletionByPodGC" | "NodeLost" | "NodeShutdown" | "Terminated" | "DeletionByTaintManager"
    )
}

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

/// Detect lifecycle transitions between previous and current pod state.
///
/// Returns a list of `(sub_type, optional_error_message)` pairs.
fn detect_transitions(
    prev: Option<&PodObservedState>,
    current: &PodObservedState,
) -> Vec<(String, Option<String>)> {
    let mut events = Vec::new();

    // Check for OOMKilled
    if current.terminated_reason.as_deref() == Some("OOMKilled") {
        let prev_reason = prev.and_then(|p| p.terminated_reason.as_deref());
        if prev_reason != Some("OOMKilled") {
            events.push((
                "container_oom_killed".to_string(),
                Some("Container was OOMKilled".to_string()),
            ));
        }
    }

    // Check for non-zero exit (non-OOM termination)
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

    // Detect clean sandbox exit (Completed with exit code 0).
    // Instead of silently ignoring it, emit a transition so we can
    // scale the deployment to 0 and prevent CrashLoopBackOff.
    if current.terminated_reason.as_deref() == Some("Completed") && current.exit_code == Some(0) {
        let prev_reason = prev.and_then(|p| p.terminated_reason.as_deref());
        if prev_reason != Some("Completed") {
            events.push(("sandbox_completed".to_string(), None));
        }
    }

    // Check for CrashLoopBackOff
    if current.waiting_reason.as_deref() == Some("CrashLoopBackOff") {
        let prev_reason = prev.and_then(|p| p.waiting_reason.as_deref());
        if prev_reason != Some("CrashLoopBackOff") {
            events.push((
                "container_crash_loop".to_string(),
                Some("Container is in CrashLoopBackOff".to_string()),
            ));
        }
    }

    // Check for restart count increase
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

    // Check for pod phase transitions to terminal states
    let prev_phase = prev.map_or("Unknown", |p| p.phase.as_str());
    if current.phase != prev_phase {
        match current.phase.as_str() {
            "Failed" => {
                // A pod reaches Failed phase three very different ways:
                //   * Real failure — the container crashed, was OOM-killed, or
                //     exited non-zero. Worth a fatal error.
                //   * Graceful node shutdown / drain — the kubelet fails the pod
                //     as its node is drained/reclaimed, but the sandbox caught
                //     SIGTERM, drained, and the container exited 0 ("Completed").
                //     This is an involuntary eviction K8s will reschedule, NOT a
                //     sandbox error. The clean stop is already reported via
                //     `sandbox_completed`, so raising a fatal error here would be
                //     a scary false alarm for a routine reschedule (see #268).
                //   * Kubelet *resource* eviction — the pod exceeded an emptyDir
                //     sizeLimit (e.g. the dind `docker-storage` cap) or the node
                //     hit disk/memory pressure. The kubelet sets the pod-level
                //     `status.reason` to exactly "Evicted" and carries the cause
                //     in `status.message`. These wipe the pod's ephemeral data
                //     and recur until the cause is fixed, so they must surface
                //     even when the sandbox drained cleanly — which previously
                //     made them silently look like an idle stop.
                let clean_exit = current.terminated_reason.as_deref() == Some("Completed")
                    && current.exit_code == Some(0);
                let is_evicted = current.pod_status_reason.as_deref() == Some("Evicted")
                    || current
                        .terminated_reason
                        .as_deref()
                        .is_some_and(|r| r.contains("Evict"));
                // A resource-pressure eviction outranks the clean-exit branch; a
                // node-lifecycle (shutdown/drain) eviction keeps #268's silence
                // on clean exit. Distinguish by the kubelet cause in
                // `status.message`.
                let resource_pressure_eviction = is_evicted
                    && !eviction_is_node_lifecycle(current.pod_status_message.as_deref());

                // Involuntary node removal — the node object was deleted (spot
                // reclaim, autoscaler consolidation) and the pod-GC controller
                // failed the pod with `DeletionByPodGC`, or the node controller
                // marked it `NodeLost`/`NodeShutdown`/`Terminated`, or a NoExecute
                // taint drained it (`DeletionByTaintManager`). The Deployment
                // reschedules onto a new node, so a fatal `pod_failed` here is a
                // false alarm for a routine reschedule — the same class as #268's
                // graceful drain, but on the hard node-removal path where the pod
                // never drained cleanly (so the `clean_exit` branch above misses
                // it).
                //
                // Match on BOTH the pod-level `status.reason` and the structured
                // `DisruptionTarget` condition (carried in `disruption_reason`):
                // the reason often lands on the condition first while
                // `status.reason` is still empty on the watch event that observes
                // the Failed edge, which previously slipped through here as a
                // scary fatal `pod_failed`.
                let is_node_removal = current
                    .pod_status_reason
                    .as_deref()
                    .is_some_and(is_node_removal_reason)
                    || current
                        .disruption_reason
                        .as_deref()
                        .is_some_and(is_node_removal_reason);

                let evicted_message = || {
                    current.pod_status_message.as_deref().map_or_else(
                        || "Pod was evicted".to_string(),
                        |m| format!("Pod evicted: {}", m.trim()),
                    )
                };

                if resource_pressure_eviction {
                    // Surface even on clean exit — the cause recurs until fixed.
                    events.push(("pod_evicted".to_string(), Some(evicted_message())));
                } else if is_node_removal {
                    // Involuntary node removal that K8s auto-reschedules. This is
                    // NOT a fatal failure, but it IS the root cause of the restart
                    // the user sees — so surface it explicitly as `node_reclaimed`
                    // (a recovering-tone event, not an error) instead of staying
                    // silent. The node name and disruption reason ride on every
                    // sandbox event payload, so the UI can name the exact cause.
                    events.push(("node_reclaimed".to_string(), None));
                } else if clean_exit {
                    // Graceful node-shutdown/drain eviction, or a clean idle
                    // stop — emit nothing. `sandbox_completed` covers the stop
                    // and the Deployment reschedules the pod; no fatal event.
                } else if is_evicted {
                    // A node-lifecycle eviction that did NOT drain cleanly —
                    // still surface it so the user sees the pod went away.
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

    // Check for pod becoming ready (not-ready → ready)
    if current.ready {
        let was_ready = prev.is_some_and(|p| p.ready);
        if !was_ready {
            events.push(("pod_ready".to_string(), None));
        }
    }

    // Pending → scheduled edge: the pod just got bound to a node. Only emit on a
    // real transition (a previously-observed pod that had no node), so we don't
    // retroactively fire for pods first seen already-scheduled.
    if let Some(prev) = prev {
        if prev.node_name.is_none() && current.node_name.is_some() {
            events.push(("pod_scheduled".to_string(), None));
        }
    }

    events
}

/// Detect server container status transitions.
///
/// Returns a list of `(ServerSubType, server_name, status)` tuples for changed servers.
fn detect_server_transitions(
    prev: Option<&PodObservedState>,
    current: &PodObservedState,
) -> Vec<(ServerSubType, String, String)> {
    let mut events = Vec::new();

    let prev_servers: HashMap<&str, &str> = prev
        .map(|p| {
            p.server_containers
                .iter()
                .map(|sc| (sc.name.as_str(), sc.status.as_str()))
                .collect()
        })
        .unwrap_or_default();

    for sc in &current.server_containers {
        let server_name = sc.name.strip_suffix("-server").unwrap_or(&sc.name);
        let prev_status = prev_servers.get(sc.name.as_str()).copied();

        if prev_status != Some(sc.status.as_str()) {
            let sub_type = match sc.status.as_str() {
                "starting" => ServerSubType::Starting,
                "started" => ServerSubType::Started,
                "failed" => ServerSubType::Failed,
                "stopped" => ServerSubType::Stopped,
                _ => continue,
            };
            events.push((sub_type, server_name.to_string(), sc.status.clone()));
        }
    }

    events
}

/// Fetch the last N lines of the sandbox container's logs.
/// Returns `None` if logs can't be fetched (best-effort, never fails).
async fn fetch_sandbox_logs(k8s: &K8sClient, pod_name: &str, tail_lines: i64) -> Option<String> {
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let params = LogParams {
        container: Some("sandbox".to_string()),
        tail_lines: Some(tail_lines),
        ..Default::default()
    };
    match pods_api.logs(pod_name, &params).await {
        Ok(logs) if !logs.is_empty() => Some(logs),
        Ok(_) => None,
        Err(e) => {
            debug!(pod_name = %pod_name, error = %e, "Failed to fetch sandbox logs for root cause");
            None
        }
    }
}

/// Best-effort lookup of the most relevant K8s Event reason+message for a pod,
/// used to name the cause of a stuck-Pending / slow-start window (e.g.
/// `FailedScheduling`, `FailedAttachVolume`, `Multi-Attach error for volume …`).
///
/// Lists core/v1 Events with a field selector on `involvedObject.name`, prefers
/// the last `Warning`-type event, and falls back to the last event of any type.
/// Returns `None` when nothing is found or the API call fails (e.g. missing RBAC
/// for events) — the caller degrades to emitting the event without a cause chip.
async fn fetch_pod_event_cause(k8s: &K8sClient, pod_name: &str) -> Option<String> {
    let events_api: Api<CoreEvent> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().fields(&format!("involvedObject.name={pod_name}"));
    let list = match events_api.list(&lp).await {
        Ok(list) => list,
        Err(e) => {
            debug!(pod_name = %pod_name, error = %e, "Failed to list pod events for cause");
            return None;
        }
    };

    // Prefer the newest Warning; fall back to the newest event of any type. K8s
    // orders list results arbitrarily, so pick by `last_timestamp` explicitly.
    let newest_warning = list
        .items
        .iter()
        .filter(|e| e.type_.as_deref() == Some("Warning"))
        .max_by_key(|e| e.last_timestamp.as_ref().map(|t| t.0));
    let event = match newest_warning {
        Some(e) => e,
        None => list
            .items
            .iter()
            .max_by_key(|e| e.last_timestamp.as_ref().map(|t| t.0))?,
    };

    let reason = event.reason.as_deref().unwrap_or("").trim();
    let message = event.message.as_deref().unwrap_or("").trim();
    match (reason.is_empty(), message.is_empty()) {
        (true, true) => None,
        (false, true) => Some(reason.to_string()),
        (true, false) => Some(message.to_string()),
        (false, false) => Some(format!("{reason}: {message}")),
    }
}

/// Emit a `taskTurn` `turn_interrupted` event when the task has a turn in
/// flight. No-op (with a debug log) when there's no task-event repo, no active
/// turn, or the lookup fails — the eventual terminal event / reaper still closes
/// the turn, so a missed interruption marker is a cosmetic loss, not correctness.
async fn emit_turn_interrupted_if_active(
    deps: &WatcherDeps,
    event_publisher: &dyn EventPublisher,
    task_id: &str,
    cause: Option<&str>,
) {
    let Some(task_events) = deps.task_events.as_ref() else {
        return;
    };
    let turn = match task_events.active_turn_for_task(task_id).await {
        Ok(Some(turn)) => turn,
        Ok(None) => {
            debug!(task_id = %task_id, "Pod interrupted but no active turn — skipping turn_interrupted");
            return;
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to look up active turn for turn_interrupted");
            return;
        }
    };

    let message = match cause {
        Some(c) => format!(
            "Turn interrupted — sandbox pod terminated involuntarily ({c}). Resuming when it reschedules."
        ),
        None => {
            "Turn interrupted — sandbox pod terminated involuntarily. Resuming when it reschedules."
                .to_string()
        }
    };
    let mut payload = serde_json::json!({ "message": message });
    if let Some(c) = cause {
        payload["reason"] = serde_json::Value::String(c.to_string());
    }

    info!(
        task_id = %task_id,
        turn_number = ?turn.turn_number,
        cause = ?cause,
        "Emitting turn_interrupted for active turn"
    );
    event_publisher
        .publish_task_turn_event(
            task_id,
            TaskTurnSubType::Interrupted,
            turn.agent_type.as_deref(),
            turn.turn_number,
            payload,
        )
        .await;
}

/// Whether a pod qualifies for a `scheduling_blocked` event: still Pending and
/// unscheduled (no node) past the threshold, and not already reported. Pure so
/// the threshold logic is unit-testable without a K8s client.
fn should_emit_scheduling_blocked(state: &PodObservedState, age_secs: i64) -> bool {
    !state.scheduling_blocked_published
        && state.phase == "Pending"
        && state.node_name.is_none()
        && age_secs >= SCHEDULING_BLOCKED_THRESHOLD_SECS
}

/// Whether a pod qualifies for a `startup_slow` event: scheduled onto a node but
/// its sandbox container still isn't ready long after creation, and the pod
/// hasn't reached a terminal phase. Pure for unit-testing.
fn should_emit_startup_slow(state: &PodObservedState, age_secs: i64) -> bool {
    !state.startup_slow_published
        && state.node_name.is_some()
        && !state.ready
        && state.phase != "Failed"
        && state.phase != "Succeeded"
        && age_secs >= STARTUP_SLOW_THRESHOLD_SECS
}

/// Periodic sweep over tracked pods that emits the threshold-driven visibility
/// events — `scheduling_blocked` (pod stuck Pending) and `startup_slow` (pod
/// scheduled but its sandbox container not yet running). Runs on the watcher's
/// renew tick so multi-minute silent windows become first-class events. Each
/// event fires at most once per pod via the sticky `*_published` guards.
async fn sweep_pending_pods(
    k8s: &K8sClient,
    event_publisher: &dyn EventPublisher,
    pod_states: &mut HashMap<String, PodObservedState>,
) {
    let now_ms = k8s_openapi::jiff::Timestamp::now().as_millisecond();

    for state in pod_states.values_mut() {
        let Some(created) = state.created_at_ms else {
            continue;
        };
        let age_secs = (now_ms - created).max(0) / 1000;

        // scheduling_blocked: still Pending and unscheduled past the threshold.
        if should_emit_scheduling_blocked(state, age_secs) {
            let cause = fetch_pod_event_cause(k8s, &state.pod_name).await;
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
                        disruption_reason: cause,
                        pending_seconds: Some(age_secs),
                        ..Default::default()
                    },
                )
                .await;
            state.scheduling_blocked_published = true;
        }

        // startup_slow: scheduled onto a node but the sandbox container still
        // isn't ready long after creation (e.g. a long EBS attach / image pull).
        if should_emit_startup_slow(state, age_secs) {
            let cause = fetch_pod_event_cause(k8s, &state.pod_name).await;
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
/// Long enough for post-mortem inspection (`kubectl logs --previous`, pod
/// describe), short enough that corpses don't accumulate — K8s itself never
/// removes Failed pods until the cluster-wide terminated-pod GC threshold.
const FAILED_POD_GC_GRACE_MS: i64 = 15 * 60 * 1000;

/// True when a tracked pod is a Failed corpse past the GC grace period that
/// hasn't been deleted yet.
fn should_gc_failed_pod(state: &PodObservedState, now_ms: i64) -> bool {
    state.phase == "Failed"
        && !state.gc_requested
        && state
            .failed_since_ms
            .is_some_and(|since| now_ms - since >= FAILED_POD_GC_GRACE_MS)
}

/// Delete pods stuck in the terminal `Failed` phase past the grace period.
///
/// Evicted pods (emptyDir sizeLimit exceeded, node pressure) stay `Failed`
/// forever unless something deletes them. The ReplicaSet has already replaced
/// them, so they are pure corpses — but they still match the task's label
/// selector, clutter the sandbox view, and pin logs on the node. Runs on the
/// watcher's renew tick (leader-only). The resulting `Delete` watch event
/// removes the entry from `pod_states`.
async fn gc_failed_pods(k8s: &K8sClient, pod_states: &mut HashMap<String, PodObservedState>) {
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

/// Retry the scale-to-0 for any pod with a still-pending auto-stop.
///
/// The watch stream only fires on pod changes, so if the scale-to-0 for a
/// crash-looping / exit-78 pod failed and the pod then produces no further
/// watch events, nothing would retry it. This sweep (on the renew tick) closes
/// that gap: it re-issues the scale for every pod whose `auto_stop_pending` is
/// still set and clears the flag on success.
async fn retry_pending_auto_stops(
    k8s: &K8sClient,
    pod_states: &mut HashMap<String, PodObservedState>,
    deps: &WatcherDeps,
) {
    for state in pod_states.values_mut() {
        if !state.auto_stop_pending {
            continue;
        }
        if crash_autostop(deps, k8s, &state.task_id).await {
            info!(
                task_id = %state.task_id,
                pod_name = %state.pod_name,
                "Crash auto-stop succeeded on periodic retry"
            );
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

/// Lifecycle sub_types that can fire on every K8s watch Apply while a
/// container flaps (CrashLoopBackOff): one publish per restart cycle otherwise
/// floods history with identical `initializing` events. Genuine transitions
/// (`pod_ready`, `pod_failed`, `pod_evicted`, `pod_recovering`,
/// `pod_scheduled`, `container_oom_killed`, ...) always publish.
const HIGH_FREQUENCY_LIFECYCLE_SUB_TYPES: &[&str] = &[
    "container_restarted",
    "container_crash_loop",
    "container_terminated",
];

/// Whether a lifecycle publish should be suppressed: high-frequency container
/// sub_type AND the sandbox status it maps to matches the last one published
/// for this pod.
fn should_skip_duplicate_publish(
    sub_type: &str,
    status: SandboxStatus,
    last_published: Option<SandboxStatus>,
) -> bool {
    HIGH_FREQUENCY_LIFECYCLE_SUB_TYPES.contains(&sub_type) && last_published == Some(status)
}

/// Map a lifecycle event sub_type to the corresponding SandboxStatus.
fn lifecycle_sub_type_to_sandbox_status(sub_type: &str) -> SandboxStatus {
    match sub_type {
        "pod_deleted" => SandboxStatus::NotCreated,
        "pod_evicted" | "pod_failed" => SandboxStatus::Error,
        // Container-level crashes stay Initializing — K8s will restart them.
        // Only pod-level failures (evicted, failed) are truly dead.
        "container_oom_killed" | "container_terminated" | "container_crash_loop" => {
            SandboxStatus::Initializing
        }
        "pod_ready" => SandboxStatus::Ready,
        "pod_succeeded" | "sandbox_completed" => SandboxStatus::Stopped,
        // Node reclaim is an involuntary reschedule, not a failure — treat it as
        // recovering (Initializing), never Error, so it doesn't read as a crash.
        "container_restarted" | "pod_recovering" | "pod_scheduled" | "node_reclaimed" => {
            SandboxStatus::Initializing
        }
        _ => SandboxStatus::Error,
    }
}

/// Generate a human-readable message for a server container event.
///
/// Since `server_name` and `status` are separate fields, this returns only
/// the contextual detail (e.g. "ContainerCreating", "Out of memory (limit: 1536Mi)").
fn format_server_message(sc: Option<&ServerContainerState>) -> String {
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
fn format_sandbox_lifecycle_message(lifecycle_event: &str, error: Option<&str>) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::k8s::pod_status::test_helpers::{
        pod_with_phase_and_containers, running_container, terminated_container, waiting_container,
    };

    /// A Failed corpse past the grace period is GC'd exactly once; live pods,
    /// fresh failures, and already-GC'd entries are left alone.
    #[test]
    fn should_gc_failed_pod_requires_terminal_phase_grace_and_no_prior_gc() {
        let pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Error", 137)],
        );
        let mut state = extract_pod_observed_state(&pod, "pod-1", "task-1");
        let now_ms = 1_000_000 + FAILED_POD_GC_GRACE_MS;

        // No failed_since recorded yet — not eligible.
        assert!(!should_gc_failed_pod(&state, now_ms));

        // Failed long enough — eligible.
        state.failed_since_ms = Some(1_000_000);
        assert!(should_gc_failed_pod(&state, now_ms));

        // Within the grace period — not yet.
        assert!(!should_gc_failed_pod(&state, now_ms - 1));

        // Delete already issued — never again.
        state.gc_requested = true;
        assert!(!should_gc_failed_pod(&state, now_ms));

        // Non-terminal phase — never, regardless of timestamps.
        state.gc_requested = false;
        state.phase = "Running".to_string();
        assert!(!should_gc_failed_pod(&state, now_ms));
    }

    /// Graceful node shutdown fails the pod while the sandbox container exits
    /// 0/Completed. That is a routine eviction K8s will reschedule, so it must
    /// NOT surface a fatal `pod_failed`/`pod_evicted` — the stop is reported via
    /// `sandbox_completed` instead.
    #[test]
    fn failed_phase_with_clean_exit_is_not_fatal() {
        let pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Completed", 0)],
        );
        let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
        let transitions = detect_transitions(None, &state);
        let subs: Vec<&str> = transitions
            .iter()
            .map(|(s, _)| s.as_str())
            .collect::<Vec<_>>();

        assert!(
            !subs.contains(&"pod_failed") && !subs.contains(&"pod_evicted"),
            "clean-exit Failed pod must not raise a fatal event: {subs:?}"
        );
        assert!(
            subs.contains(&"sandbox_completed"),
            "clean stop should still be reported via sandbox_completed: {subs:?}"
        );
    }

    /// A pod that fails with a real crash (non-zero exit) still raises
    /// `pod_failed` so the user sees the fatal error.
    #[test]
    fn failed_phase_with_crash_still_fatal() {
        let pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Error", 1)],
        );
        let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
        let transitions = detect_transitions(None, &state);
        let subs: Vec<&str> = transitions
            .iter()
            .map(|(s, _)| s.as_str())
            .collect::<Vec<_>>();

        assert!(
            subs.contains(&"pod_failed"),
            "a real crash must raise pod_failed: {subs:?}"
        );
    }

    /// A kubelet *resource* eviction (pod `status.reason == "Evicted"`, e.g.
    /// emptyDir sizeLimit exceeded) must surface `pod_evicted` with the
    /// actionable `status.message` — even when the sandbox drained cleanly
    /// (exit 0), which previously (pre-#269) made the eviction silently look
    /// like a stop under #268's clean-exit-wins ordering.
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

        let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
        let transitions = detect_transitions(None, &state);

        let evicted = transitions
            .iter()
            .find(|(s, _)| s == "pod_evicted")
            .expect("eviction must raise pod_evicted");
        let message = evicted.1.as_deref().unwrap_or_default();
        assert!(
            message.contains("docker-storage") && message.contains("10Gi"),
            "pod_evicted must carry the kubelet's eviction cause, got: {message}"
        );
    }

    /// A graceful node-shutdown / drain eviction (pod `status.reason ==
    /// "Evicted"` but a node-lifecycle `status.message`) that drained cleanly
    /// stays SILENT — #268's intent, preserved by classifying the cause. K8s
    /// reschedules it and `sandbox_completed` already reports the clean stop.
    #[test]
    fn node_shutdown_eviction_on_clean_exit_stays_silent() {
        let mut pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Completed", 0)],
        );
        let status = pod.status.as_mut().expect("test pod has status");
        status.reason = Some("Evicted".to_string());
        status.message = Some("The node was low on resource: node is shutting down.".into());

        let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
        let transitions = detect_transitions(None, &state);
        let subs: Vec<&str> = transitions
            .iter()
            .map(|(s, _)| s.as_str())
            .collect::<Vec<_>>();

        assert!(
            !subs.contains(&"pod_evicted") && !subs.contains(&"pod_failed"),
            "a clean-exit node-shutdown eviction must stay silent: {subs:?}"
        );
        assert!(
            subs.contains(&"sandbox_completed"),
            "the clean stop is still reported via sandbox_completed: {subs:?}"
        );
    }

    /// An involuntary node removal — the node object is deleted (spot reclaim /
    /// autoscaler consolidation) and the pod-GC controller fails the pod with
    /// `status.reason == "DeletionByPodGC"`, hard-killing the container (no clean
    /// exit). K8s reschedules onto a new node, so this must NOT raise a fatal
    /// `pod_failed`/`pod_evicted`; instead it surfaces the root cause as a
    /// recovering-tone `node_reclaimed` event.
    #[test]
    fn deletion_by_pod_gc_is_node_reclaimed_not_fatal() {
        let mut pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Error", 137)],
        );
        let status = pod.status.as_mut().expect("test pod has status");
        status.reason = Some("DeletionByPodGC".to_string());
        status.message = Some("PodGC: node no longer exists".into());

        let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
        let transitions = detect_transitions(None, &state);
        let subs: Vec<&str> = transitions
            .iter()
            .map(|(s, _)| s.as_str())
            .collect::<Vec<_>>();

        assert!(
            !subs.contains(&"pod_failed") && !subs.contains(&"pod_evicted"),
            "a node-removal (DeletionByPodGC) reschedule must not raise a fatal event: {subs:?}"
        );
        assert!(
            subs.contains(&"node_reclaimed"),
            "node removal must surface the root cause as node_reclaimed: {subs:?}"
        );
        // node_reclaimed is a recovering reschedule, never an Error status.
        assert!(
            !matches!(
                lifecycle_sub_type_to_sandbox_status("node_reclaimed"),
                SandboxStatus::Error
            ),
            "node_reclaimed must not map to an Error sandbox status"
        );
    }

    /// The real spot-reclaim shape that previously slipped through as a fatal:
    /// the node-removal reason arrives on the structured `DisruptionTarget`
    /// condition while the pod-level `status.reason` is still empty on the watch
    /// event that first observes the Failed edge. Classification must consult the
    /// condition (`disruption_reason`), not just `status.reason`, so this is a
    /// `node_reclaimed` reschedule and NOT a fatal `pod_failed`.
    #[test]
    fn node_removal_via_disruption_condition_is_not_fatal() {
        use k8s_openapi::api::core::v1::PodCondition;

        let mut pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Error", 137)],
        );
        let status = pod.status.as_mut().expect("test pod has status");
        // status.reason intentionally left unset — the cause is only on the
        // DisruptionTarget condition (a NoExecute-taint drain / spot reclaim).
        status.conditions = Some(vec![PodCondition {
            type_: "DisruptionTarget".to_string(),
            reason: Some("DeletionByTaintManager".to_string()),
            ..Default::default()
        }]);

        let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
        assert!(
            state.pod_status_reason.is_none(),
            "precondition: status.reason must be empty so only the condition carries the cause"
        );

        let transitions = detect_transitions(None, &state);
        let subs: Vec<&str> = transitions
            .iter()
            .map(|(s, _)| s.as_str())
            .collect::<Vec<_>>();

        assert!(
            !subs.contains(&"pod_failed") && !subs.contains(&"pod_evicted"),
            "a condition-sourced node removal must not raise a fatal event: {subs:?}"
        );
        assert!(
            subs.contains(&"node_reclaimed"),
            "condition-sourced node removal must surface node_reclaimed: {subs:?}"
        );
    }

    /// A terminated sandbox container exposes its exit code + K8s reason so the
    /// fatal/termination payloads can distinguish OOM (137) from a crash (1).
    #[test]
    fn extract_captures_sandbox_exit_code_and_reason() {
        let pod = pod_with_phase_and_containers(
            "Failed",
            vec![terminated_container("sandbox", "Error", 1)],
        );
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

    /// A crash-looping non-default server must NOT keep the watcher from
    /// observing the sandbox as ready and emitting `pod_ready`.
    #[test]
    fn ready_tracks_sandbox_container_not_pod_condition() {
        let pod = pod_with_phase_and_containers(
            "Running",
            vec![
                running_container("sandbox", true, true),
                waiting_container("storybook-server", "CrashLoopBackOff"),
            ],
        );

        let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
        assert!(
            state.ready,
            "sandbox ready should make observed state ready despite crash-looping server"
        );

        let transitions = detect_transitions(None, &state);
        assert!(
            transitions.iter().any(|(sub, _)| sub == "pod_ready"),
            "expected pod_ready transition, got: {transitions:?}"
        );
    }

    /// When the sandbox container itself is not ready, the state is not ready
    /// even if other containers are healthy.
    #[test]
    fn not_ready_when_sandbox_container_not_ready() {
        let pod = pod_with_phase_and_containers(
            "Running",
            vec![
                running_container("sandbox", true, false),
                running_container("dev-server", true, true),
            ],
        );

        let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
        assert!(!state.ready);
    }

    /// `extract_pod_observed_state` must carry each server container's restart
    /// count so the crash-loop auto-disable crossing logic can fire.
    #[test]
    fn server_container_restart_count_is_populated() {
        use k8s_openapi::api::core::v1::{
            ContainerState as K8sState, ContainerStateWaiting, ContainerStatus, Pod, PodStatus,
        };

        let pod = Pod {
            status: Some(PodStatus {
                phase: Some("Running".to_string()),
                container_statuses: Some(vec![
                    running_container("sandbox", true, true),
                    ContainerStatus {
                        name: "storybook-server".to_string(),
                        ready: false,
                        started: Some(false),
                        restart_count: 7,
                        state: Some(K8sState {
                            waiting: Some(ContainerStateWaiting {
                                reason: Some("CrashLoopBackOff".to_string()),
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

        let state = extract_pod_observed_state(&pod, "pod-1", "task-1");
        let sb = state
            .server_containers
            .iter()
            .find(|s| s.name == "storybook-server")
            .expect("storybook-server tracked");
        assert_eq!(sb.restart_count, 7);
    }

    /// `extract_pod_observed_state` captures the scheduled node and the
    /// structured `DisruptionTarget` reason so node-level correlation and the
    /// disruption cause are readable directly from the event.
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

    /// A pod that binds to a node (Pending→scheduled edge) emits `pod_scheduled`,
    /// but only on a real transition — never for a pod first seen already bound.
    #[test]
    fn detect_emits_pod_scheduled_on_binding() {
        let mut pending = extract_pod_observed_state(
            &pod_with_phase_and_containers("Pending", vec![]),
            "pod-1",
            "task-1",
        );
        pending.node_name = None;

        let mut scheduled = pending.clone();
        scheduled.node_name = Some("ip-10-0-1-42".to_string());

        let transitions = detect_transitions(Some(&pending), &scheduled);
        assert!(
            transitions.iter().any(|(sub, _)| sub == "pod_scheduled"),
            "expected pod_scheduled, got: {transitions:?}"
        );

        // First observation already-scheduled (prev = None) must NOT fire it.
        let first_seen = detect_transitions(None, &scheduled);
        assert!(
            !first_seen.iter().any(|(sub, _)| sub == "pod_scheduled"),
            "pod_scheduled should not fire retroactively, got: {first_seen:?}"
        );
    }

    /// `pod_scheduled` maps to an Initializing (progress) status, not the
    /// default Error arm — otherwise it would trip the intentional-stop check.
    #[test]
    fn pod_scheduled_is_initializing_not_error() {
        assert_eq!(
            lifecycle_sub_type_to_sandbox_status("pod_scheduled"),
            SandboxStatus::Initializing
        );
    }

    fn base_state(phase: &str) -> PodObservedState {
        extract_pod_observed_state(
            &pod_with_phase_and_containers(phase, vec![]),
            "pod-1",
            "task-1",
        )
    }

    /// A pod stuck Pending+unscheduled past the threshold trips
    /// `scheduling_blocked` exactly once — not before the threshold, not once
    /// scheduled, and not after it's already been reported.
    #[test]
    fn scheduling_blocked_predicate() {
        let mut s = base_state("Pending");
        s.node_name = None;

        assert!(!should_emit_scheduling_blocked(
            &s,
            SCHEDULING_BLOCKED_THRESHOLD_SECS - 1
        ));
        assert!(should_emit_scheduling_blocked(
            &s,
            SCHEDULING_BLOCKED_THRESHOLD_SECS
        ));

        // Once scheduled (node assigned) it no longer qualifies.
        let mut scheduled = s.clone();
        scheduled.node_name = Some("ip-10-0-1-42".to_string());
        assert!(!should_emit_scheduling_blocked(
            &scheduled,
            SCHEDULING_BLOCKED_THRESHOLD_SECS + 300
        ));

        // Fire-once guard.
        s.scheduling_blocked_published = true;
        assert!(!should_emit_scheduling_blocked(
            &s,
            SCHEDULING_BLOCKED_THRESHOLD_SECS + 300
        ));
    }

    /// A scheduled-but-not-ready pod trips `startup_slow` past the threshold, but
    /// not while ready, nor once it has reached a terminal phase, nor twice.
    #[test]
    fn startup_slow_predicate() {
        let mut s = base_state("Running");
        s.node_name = Some("ip-10-0-1-42".to_string());
        s.ready = false;

        assert!(!should_emit_startup_slow(
            &s,
            STARTUP_SLOW_THRESHOLD_SECS - 1
        ));
        assert!(should_emit_startup_slow(&s, STARTUP_SLOW_THRESHOLD_SECS));

        // Ready pods don't qualify.
        let mut ready = s.clone();
        ready.ready = true;
        assert!(!should_emit_startup_slow(
            &ready,
            STARTUP_SLOW_THRESHOLD_SECS + 300
        ));

        // Terminal phases don't qualify (the fatal path handles those).
        let mut failed = s.clone();
        failed.phase = "Failed".to_string();
        assert!(!should_emit_startup_slow(
            &failed,
            STARTUP_SLOW_THRESHOLD_SECS + 300
        ));

        // Unscheduled pods are scheduling_blocked's job, not startup_slow's.
        let mut unscheduled = s.clone();
        unscheduled.node_name = None;
        assert!(!should_emit_startup_slow(
            &unscheduled,
            STARTUP_SLOW_THRESHOLD_SECS + 300
        ));

        // Fire-once guard.
        s.startup_slow_published = true;
        assert!(!should_emit_startup_slow(
            &s,
            STARTUP_SLOW_THRESHOLD_SECS + 300
        ));
    }

    /// High-frequency container sub_types are suppressed only when they would
    /// repeat the last-published sandbox status; genuine transitions always
    /// publish regardless of the last-published status.
    #[test]
    fn duplicate_publish_suppression() {
        // A flapping container re-reporting Initializing is suppressed.
        assert!(should_skip_duplicate_publish(
            "container_restarted",
            SandboxStatus::Initializing,
            Some(SandboxStatus::Initializing)
        ));
        assert!(should_skip_duplicate_publish(
            "container_crash_loop",
            SandboxStatus::Initializing,
            Some(SandboxStatus::Initializing)
        ));
        assert!(should_skip_duplicate_publish(
            "container_terminated",
            SandboxStatus::Initializing,
            Some(SandboxStatus::Initializing)
        ));

        // A status change publishes even for high-frequency sub_types.
        assert!(!should_skip_duplicate_publish(
            "container_restarted",
            SandboxStatus::Initializing,
            Some(SandboxStatus::Ready)
        ));

        // First publish for a pod (nothing published yet) goes through.
        assert!(!should_skip_duplicate_publish(
            "container_restarted",
            SandboxStatus::Initializing,
            None
        ));

        // Genuine transitions are never suppressed, even when repeated.
        assert!(!should_skip_duplicate_publish(
            "pod_ready",
            SandboxStatus::Ready,
            Some(SandboxStatus::Ready)
        ));
        assert!(!should_skip_duplicate_publish(
            "pod_evicted",
            SandboxStatus::Error,
            Some(SandboxStatus::Error)
        ));
        // OOM kills stay loud — each one carries a fresh exit code payload.
        assert!(!should_skip_duplicate_publish(
            "container_oom_killed",
            SandboxStatus::Initializing,
            Some(SandboxStatus::Initializing)
        ));
    }
}
