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

use moatless_backend_core::scope::ScopeContext;

use super::client::K8sClient;
use super::deployment;
use super::leader_election::LeaderElection;
use super::pod_observation_policy::{
    ObservationContext, PodObservationPolicy, should_emit_scheduling_blocked,
    should_emit_startup_slow, should_gc_failed_pod,
};
use super::pod_status::{extract_container_observed_statuses, is_sandbox_container_ready};
use crate::sandbox::lifecycle::Lifecycle;
use crate::sandbox::lifecycle::OperationGeneration;
use crate::sandbox::lifecycle::classify::ObservedTransition;
use crate::sandbox::lifecycle::runtime::{OPERATION_GENERATION_LABEL, generation_from_label};
use crate::sandbox::lifecycle::watcher::{classify_observation, publish_sandbox_observation};
use crate::sandbox::models::DEFAULT_SERVER_READINESS_TIMEOUT_SECS;
use crate::task::dao::{TaskDao, TaskPatch};
use crate::task::models::{SandboxDesiredState, SandboxStopReason};
use crate::task_event::dao::TaskEventDao;
use crate::workspace::dao::WorkspaceDao;
use moatless_core::events::EventPublisher;

/// Dependencies the watcher needs to remediate a crash-looping sandbox. The
/// lifecycle owns plan construction and intent, so the watcher only carries the
/// handle plus the task/workspace repos its auto-disable decision reads. All are
/// optional because the watcher also runs in setups without a database (where
/// remediation degrades to publishing a `Failed` event / raw scale-to-0).
#[derive(Clone)]
struct WatcherDeps {
    /// Enables ConfigMap-backed node archive tracking for node-local workspace
    /// and Docker volumes. When false, the watcher performs no archive writes.
    node_archive_enabled: bool,
    /// Shared durable lifecycle handle — the sole writer of Sandbox intent. The
    /// watcher drives crash auto-stop (`stop`) and auto-disable redeploy
    /// (`redeploy`) through it. `None` in DB-less setups, where crash mitigation
    /// degrades to a raw scale-to-0 and auto-disable is skipped.
    lifecycle: Option<Arc<Lifecycle>>,
    tasks: Option<Arc<dyn TaskDao>>,
    workspaces: Option<Arc<dyn WorkspaceDao>>,
    /// Task-event repo, used to look up the active turn when a pod terminates
    /// involuntarily so `turn_interrupted` carries the right turn context (and
    /// is suppressed when no turn is in flight). `None` in DB-less setups.
    task_events: Option<Arc<dyn TaskEventDao>>,
}

/// Pod-Pending duration after which the watcher emits `scheduling_blocked`,
/// surfacing an otherwise-silent unschedulable window (no node, quota, etc.).
pub(super) const SCHEDULING_BLOCKED_THRESHOLD_SECS: i64 = 60;

/// Time a scheduled pod may sit with its sandbox container not yet running
/// before the watcher emits `startup_slow` — makes a long volume-attach or
/// image-pull window (e.g. EBS RWO Multi-Attach) a first-class event.
pub(super) const STARTUP_SLOW_THRESHOLD_SECS: i64 = 120;

/// Exit code used by the sandbox to signal a permanent init failure (sysexits EX_CONFIG).
const PERMANENT_FAILURE_EXIT_CODE: i32 = 78;

/// Observed state of a server container, used for transition detection.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ServerContainerState {
    pub(super) name: String,
    pub(super) status: String,
    pub(super) error: Option<String>,
    pub(super) detail: Option<String>,
    pub(super) terminated_reason: Option<String>,
    pub(super) exit_code: Option<i32>,
    pub(super) memory_limit: Option<String>,
    pub(super) running_since: Option<chrono::DateTime<chrono::Utc>>,
    /// Restart count of this server container (used for crash-loop auto-disable).
    pub(super) restart_count: i32,
}

/// Observed state for a single pod, used for transition detection.
#[derive(Debug, Clone)]
pub(super) struct PodObservedState {
    pub(super) task_id: String,
    pub(super) operation_generation: Option<OperationGeneration>,
    pub(super) phase: String,
    /// Restart count of the **sandbox** container specifically.
    pub(super) restart_count: i32,
    pub(super) terminated_reason: Option<String>,
    pub(super) waiting_reason: Option<String>,
    pub(super) ready: bool,
    pub(super) pod_name: String,
    /// Exit code from the sandbox container's *current* terminated state.
    pub(super) exit_code: Option<i32>,
    /// Exit code from the sandbox container's *last* terminated state — survives a
    /// K8s restart so a crash that's already been restarted into Running isn't lost.
    pub(super) last_exit_code: Option<i32>,
    /// Termination reason from the sandbox container's *last* terminated state.
    pub(super) last_terminated_reason: Option<String>,
    /// Termination message or waiting message from the sandbox container.
    pub(super) failure_detail: Option<String>,
    /// Server container snapshots (excludes sandbox container).
    pub(super) server_containers: Vec<ServerContainerState>,
    /// Whether a fatal event has already been published for this pod.
    /// Suppresses repeated fatal events during CrashLoopBackOff.
    pub(super) fatal_published: bool,
    /// Server names already auto-disabled during this pod's lifecycle. Fire-once
    /// guard so the watcher doesn't re-publish the disable on every watch event
    /// while the container keeps crash-looping in this pod. Cleared naturally
    /// when a new pod replaces this map entry.
    pub(super) auto_disabled_servers: HashSet<String>,
    /// Whether the `moatless_sandbox_startup_seconds` histogram has already been
    /// recorded for this pod. Carried over so a pod that goes Ready→NotReady→
    /// Ready (e.g. after a container restart) is only timed on its *first* ready,
    /// keeping startup latency from being inflated by re-readies.
    pub(super) startup_recorded: bool,
    /// Node the pod is scheduled on (`pod.spec.node_name`), once bound. `None`
    /// while the pod is still `Pending` and unscheduled.
    pub(super) node_name: Option<String>,
    /// Why the pod was disrupted, derived from the `DisruptionTarget` pod
    /// condition reason, falling back to `pod.status.reason`.
    pub(super) disruption_reason: Option<String>,
    /// Raw pod-level `status.reason`. Kubelet resource evictions (emptyDir
    /// sizeLimit, node pressure) set exactly `"Evicted"` here — the container
    /// states never carry it, so this is the only reliable eviction signal.
    pub(super) pod_status_reason: Option<String>,
    /// Pod-level `status.message` — for evictions this holds the actionable
    /// cause (e.g. `Usage of EmptyDir volume "docker-storage" exceeds the
    /// limit "10Gi"`).
    pub(super) pod_status_message: Option<String>,
    /// Pod `creationTimestamp` as epoch milliseconds, used to derive
    /// `pending_seconds` on the Pending→scheduled edge.
    pub(super) created_at_ms: Option<i64>,
    /// Fire-once guard: a `scheduling_blocked` event has been emitted for this
    /// pod's stuck-Pending window. Carried over so the threshold sweep emits at
    /// most one blocked event per pod.
    pub(super) scheduling_blocked_published: bool,
    /// Fire-once guard: a `startup_slow` event has been emitted for this pod's
    /// slow-container-start window (e.g. long volume attach).
    pub(super) startup_slow_published: bool,
    /// Epoch ms when this pod was first observed in the terminal `Failed`
    /// phase. K8s never garbage-collects Failed pods (e.g. evicted for
    /// exceeding an emptyDir sizeLimit), so the sweep deletes them once this
    /// is older than the GC grace period.
    pub(super) failed_since_ms: Option<i64>,
    /// Fire-once guard: a GC delete has been issued for this Failed pod.
    pub(super) gc_requested: bool,
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
    /// identical publishes from the high-frequency container sub_types
    /// (restart/crash-loop/terminated), which otherwise fire once per K8s
    /// watch Apply while a container flaps. Watcher restart resets this —
    /// acceptable (one extra event per pod after failover).
    pub(super) last_published_sandbox_status: Option<SandboxStatus>,
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

/// Spawn the pod lifecycle watcher as a background tokio task.
///
/// The watcher runs in a loop:
/// 1. Try to acquire leadership via K8s Lease.
/// 2. If leader, start watching pods.
/// 3. If not leader, sleep and retry.
/// 4. On watch stream error, re-enter the leadership loop.
// reason: the watcher wires nine collaborators (client, publisher, cancel, the
// crash threshold, archive feature gate, and four optional DAOs) into
// one spawned task.
#[allow(clippy::too_many_arguments)]
pub fn spawn_pod_lifecycle_watcher(
    k8s: K8sClient,
    event_publisher: Arc<dyn EventPublisher>,
    cancel: CancellationToken,
    max_crash_restart_count: i32,
    node_archive_enabled: bool,
    lifecycle: Option<Arc<Lifecycle>>,
    tasks: Option<Arc<dyn TaskDao>>,
    workspaces: Option<Arc<dyn WorkspaceDao>>,
    task_events: Option<Arc<dyn TaskEventDao>>,
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
        node_archive_enabled,
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
                retry_pending_auto_stops(&mut pod_states, deps).await;
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
#[expect(
    clippy::cognitive_complexity,
    reason = "Kubernetes watch adapter still coordinates Sandbox observation, publication, and cleanup intent until the Sandbox observation module lands"
)]
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

            // Node-backed workspace/Docker archives outlive the pod. Record a
            // scheduled location on first observation and whenever the node or
            // operation generation changes. Watcher persistence is best-effort;
            // start/remove also synchronously merge the current pod before a
            // load-bearing placement or deletion decision.
            let should_record_node = deps.node_archive_enabled
                && new_state.node_name.is_some()
                && pod_states.get(&pod_name).is_none_or(|previous| {
                    previous.node_name != new_state.node_name
                        || previous.operation_generation != new_state.operation_generation
                });
            if should_record_node {
                if let Err(error) =
                    super::node_archive::record_observed_pod(k8s, &task_id, pod).await
                {
                    warn!(
                        task_id,
                        pod_name,
                        error = %error,
                        "Failed to record node-local sandbox archive location"
                    );
                }
            }

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

            // Carried across Apply events so a flapping container (one watch
            // event per restart) publishes each status once, not once per Apply.
            new_state.last_published_sandbox_status =
                prev_state.and_then(|p| p.last_published_sandbox_status);

            let initial_plan = PodObservationPolicy::evaluate(
                prev_state,
                &new_state,
                ObservationContext::default(),
            );

            // Publish server container events as typed `server` events
            for transition in &initial_plan.server_transitions {
                let server_sub_type = transition.sub_type;
                let server_name = &transition.server_name;
                let sc = new_state.server_containers.iter().find(|s| {
                    let n = s.name.strip_suffix("-server").unwrap_or(&s.name);
                    n == server_name
                });

                info!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    server = %server_name,
                    event = ?transition.sub_type,
                    status = %transition.status,
                    error = ?sc.and_then(|s| s.error.as_deref()),
                    detail = ?sc.and_then(|s| s.detail.as_deref()),
                    "Server container status changed"
                );

                event_publisher
                    .publish_server_event(
                        &task_id,
                        server_sub_type,
                        &ServerEventPayload {
                            server_name: server_name.clone(),
                            status: server_sub_type,
                            message: format_server_message(sc),
                        },
                    )
                    .await;
            }

            // Check if this is an intentional stop (Deployment scaled to 0).
            // When the idle reaper or user explicitly stops a sandbox, the pod
            // terminates with exit code 137 (SIGKILL) which would otherwise be
            // misinterpreted as a crash.
            let is_intentional_stop = if initial_plan.has_error_transition {
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
            let plan = if is_intentional_stop {
                PodObservationPolicy::evaluate(
                    prev_state,
                    &new_state,
                    ObservationContext {
                        intentional_stop: true,
                    },
                )
            } else {
                initial_plan
            };

            // If sandbox entered an error state AND this is NOT an intentional stop,
            // force all servers to Failed (pod crash kills server containers too).
            if plan.has_error_transition && !is_intentional_stop {
                for sc in &new_state.server_containers {
                    let server_name = sc.name.strip_suffix("-server").unwrap_or(&sc.name);
                    // Skip servers already reported as failed by the policy plan.
                    let already_failed = plan.server_transitions.iter().any(|t| {
                        t.server_name == server_name && t.sub_type == ServerSubType::Failed
                    });
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

            for transition in &plan.sandbox_transitions {
                let sub_type = &transition.sub_type;
                let error_msg = &transition.error;
                let sandbox_status = transition.status;

                if transition.is_error_event && !is_intentional_stop {
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
                if transition.skip_duplicate_publish {
                    debug!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        lifecycle_event = %sub_type,
                        sandbox_status = %sandbox_status,
                        "Skipping duplicate sandbox status publish"
                    );
                    continue;
                }

                let mut event_generation = new_state.operation_generation;
                if let Some(observed_transition) = transition.lifecycle_observation {
                    let decision = classify_observation(
                        deps.lifecycle.as_ref(),
                        &task_id,
                        observed_transition,
                        new_state.operation_generation,
                    )
                    .await;
                    if !decision.publishes {
                        debug!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            lifecycle_event = %sub_type,
                            "Suppressing lifecycle-correlated pod observation"
                        );
                        continue;
                    }
                    event_generation = decision.event_generation;
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

                publish_sandbox_observation(
                    event_publisher,
                    &task_id,
                    sandbox_status,
                    &SandboxEventPayload {
                        message: format_sandbox_lifecycle_message(sub_type, error_msg.as_deref()),
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
                    event_generation,
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
                // Carry a still-pending auto-stop so a failed scale-to-0 is
                // retried on this event (see the auto-stop block below).
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
            if plan
                .sandbox_transitions
                .iter()
                .any(|transition| transition.sub_type == "pod_ready")
            {
                new_state.fatal_published = false;
                // Time pod-creation → first Ready once per pod.
                if plan.should_record_startup {
                    record_sandbox_startup(pod);
                    new_state.startup_recorded = true;
                }
            }

            // Only truly irrecoverable events are fatal — pod-level failures
            // where K8s will NOT restart the container.
            // Container-level crashes (OOM, terminated, crash loop) are not fatal
            // because K8s will restart them. The auto-stop threshold handles
            // persistent crash loops separately.
            if plan.has_fatal_transition && !is_intentional_stop && !new_state.fatal_published {
                let error_msg = plan
                    .sandbox_transitions
                    .iter()
                    .find_map(|transition| transition.error.as_ref())
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
                if let Some(note) = workspace_preservation_note(deps, &task_id).await {
                    parts.push(note);
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
                            message: detailed_message.clone(),
                            lifecycle_event: Some("fatal".to_string()),
                            error: Some(detailed_message),
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
            let has_terminated = plan
                .sandbox_transitions
                .iter()
                .any(|transition| transition.sub_type == "container_terminated");
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
                && crash_autostop(deps, &task_id).await
            {
                info!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    "Crash auto-stop succeeded"
                );
                new_state.auto_stop_pending = false;
            }

            // Reconcile a clean sandbox exit (Completed, exit 0) against durable
            // Sandbox intent.
            //
            // The sandbox's gRPC server never exits on its own during a task —
            // it only returns exit 0 via the SIGTERM graceful-shutdown driver
            // (see sandbox/src/shutdown.rs). So a `sandbox_completed` means "the
            // pod got a SIGTERM", which has three very different causes:
            //   * Stop (idle reaper / user / crash auto-stop) already scaled the
            //     Deployment to 0 — the SIGTERM is the pod draining. Scaling to
            //     0 again is a harmless no-op that confirms the stop.
            //   * Remove (task close / delete) already deleted the Deployment.
            //     There is nothing to scale.
            //   * Involuntary node event (spot reclaim, drain, consolidation)
            //     SIGTERMs a pod the Deployment still wants. Here K8s will
            //     reschedule the pod and agent recovery resumes the in-flight
            //     turn — UNLESS we scale to 0 and cancel the task.
            //
            // Only intent separates these: the K8s API reports a deleted and a
            // scaled-to-0 Deployment identically. A genuine exit-0 restart loop
            // is still caught by the restart-count auto-stop above.
            if plan.has_completed_transition {
                match clean_exit_action(deps, k8s, &task_id).await {
                    CleanExitAction::AlreadyRemoved => {
                        info!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            "Sandbox exited cleanly after a remove; deployment is already deleted"
                        );
                    }
                    CleanExitAction::ConfirmStop => {
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
                    }
                    CleanExitAction::LeaveForReschedule => {
                        info!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            "Sandbox exited cleanly while a replica is still desired; treating as involuntary eviction and leaving it to K8s to reschedule"
                        );
                    }
                }
            }

            // Auto-disable a crash-looping *server* container so a single broken
            // preview server doesn't come back on the sandbox's next start. This
            // never touches the running pod: the sandbox container and every
            // healthy sidecar keep serving, and the broken container is left to
            // the kubelet's own restart backoff. The default server is never
            // disabled (only surfaced as Failed).
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
            let operation_generation =
                pod.metadata.labels.as_ref().and_then(|labels| {
                    generation_from_label(labels.get(OPERATION_GENERATION_LABEL))
                });

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

                let decision = classify_observation(
                    deps.lifecycle.as_ref(),
                    &task_id,
                    if sandbox_status == SandboxStatus::Stopped {
                        ObservedTransition::CleanStop
                    } else {
                        ObservedTransition::Deleted
                    },
                    operation_generation,
                )
                .await
                .publishes;
                if !decision {
                    debug!(
                        task_id = %task_id,
                        pod_name = %pod_name,
                        "Suppressing lifecycle-correlated pod deletion"
                    );
                }

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
                if decision {
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
                }

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
/// - Otherwise the server name is persisted into `tasks.disabled_servers`, so
///   the next provisioned pod is built without the container.
///
/// **The running pod is deliberately left alone.** Removing a container from a
/// pod means replacing the pod, which kills the sandbox container and the
/// in-flight agent turn with it — a broken preview server taking down the whole
/// sandbox is exactly what this function exists to prevent. The crash-looping
/// container stays in `CrashLoopBackOff` (kubelet backoff caps at 5 minutes)
/// until the sandbox is next stopped/started, which also means a server the
/// agent repairs in the meantime recovers on its own.
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
        "Server disabled after {} restarts (threshold: {}); it will not be started again with the sandbox",
        sc.restart_count, max_crash_restart_count
    );

    // Need DB access to determine default-ness and persist the disabled set.
    let (Some(tasks), Some(workspaces)) = (deps.tasks.as_ref(), deps.workspaces.as_ref()) else {
        debug!(
            task_id = %task_id,
            "No DB access in watcher — surfacing server Failed without auto-disable"
        );
        publish_server_failed(
            event_publisher,
            task_id,
            server_name,
            format!("Server crash-looping after {} restarts", sc.restart_count),
        )
        .await;
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
    // reason: pod watcher auto-disable reads task across all owners (system context)
    #[allow(clippy::disallowed_methods)]
    let sys = ScopeContext::system();
    let mut disabled = match tasks.find_by_id(task_id, &sys).await {
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
    if let Err(e) = tasks.update_task(task_id, &patch, &sys).await {
        warn!(task_id = %task_id, error = %e, "Failed to persist disabled_servers");
        return;
    }

    // Server-scoped fact only: the sandbox itself is unaffected, so publishing a
    // `sandbox` event here would report a healthy sandbox as re-initializing.
    publish_server_failed(event_publisher, task_id, server_name, failed_message).await;
    info!(
        task_id = %task_id,
        server = %server_name,
        "Auto-disabled crash-looping server; it will be dropped from the next pod"
    );
}

/// Stop a crash-looping sandbox through the lifecycle (Stopped/Error intent +
/// graceful stop + scale-to-0). Returns whether the stop landed, so callers can
/// clear their pending-retry flag. The watcher performs no direct K8s mutation:
/// without a lifecycle handle (DB-less) there is no intent owner, so the crash
/// loop is left for the reconciler rather than scaled down here.
async fn crash_autostop(deps: &WatcherDeps, task_id: &str) -> bool {
    let Some(lifecycle) = deps.lifecycle.as_ref() else {
        debug!(task_id = %task_id, "No lifecycle handle — leaving crash-looping sandbox for the reconciler");
        return false;
    };
    match lifecycle.stop(task_id, SandboxStopReason::Error).await {
        Ok(_) => true,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Crash auto-stop via lifecycle failed; will retry");
            false
        }
    }
}

/// What to do with the Deployment when a sandbox pod exits cleanly (exit 0
/// after SIGTERM). See the call site for why a clean exit is ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanExitAction {
    /// Intent is `Removed` — lifecycle already deleted the Deployment as part of
    /// the remove, so there is nothing left to scale (a patch would 404).
    AlreadyRemoved,
    /// Intent is `Stopped` — the SIGTERM is the pod draining after a deliberate
    /// stop. Confirm the scale-to-0.
    ConfirmStop,
    /// Intent is `Running`, or unknown — an involuntary node event SIGTERMed a
    /// pod the Deployment still wants. Leave it for K8s to reschedule.
    LeaveForReschedule,
}

/// Map durable Sandbox intent onto the clean-exit action.
///
/// `None` means intent could not be read (no lifecycle handle, missing Task
/// row, or a store error) and fails safe: a reschedule after an eviction is
/// recoverable, a wrongly cancelled active task is not.
fn clean_exit_action_for_intent(desired: Option<SandboxDesiredState>) -> CleanExitAction {
    match desired {
        Some(SandboxDesiredState::Removed) => CleanExitAction::AlreadyRemoved,
        Some(SandboxDesiredState::Stopped) => CleanExitAction::ConfirmStop,
        Some(SandboxDesiredState::Running) | None => CleanExitAction::LeaveForReschedule,
    }
}

/// Decide what a clean sandbox exit means for the Deployment.
///
/// Intent is the authority: lifecycle is the sole writer of Sandbox intent, and
/// only it can distinguish "Deployment deleted by a remove" from "Deployment
/// scaled to 0 by a stop" — the K8s API reports both as zero desired replicas
/// (`get_deployment_replicas` maps a 404 to `(0, 0)`).
///
/// Without a lifecycle handle (DB-less setups) there is no intent to read, so
/// this falls back to the replica count, which at least separates a stop from a
/// still-wanted replica.
async fn clean_exit_action(deps: &WatcherDeps, k8s: &K8sClient, task_id: &str) -> CleanExitAction {
    let Some(lifecycle) = deps.lifecycle.as_ref() else {
        return match deployment::get_deployment_replicas(k8s, task_id).await {
            Ok((0, _)) => CleanExitAction::ConfirmStop,
            Ok(_) => CleanExitAction::LeaveForReschedule,
            Err(e) => {
                warn!(
                    task_id = %task_id,
                    error = %e,
                    "Could not read deployment replicas on clean exit; leaving deployment untouched"
                );
                CleanExitAction::LeaveForReschedule
            }
        };
    };
    let desired = match lifecycle.desired_state(task_id).await {
        Ok(desired) => Some(desired),
        Err(e) => {
            debug!(
                task_id = %task_id,
                error = %e,
                "Could not read sandbox intent on clean exit; leaving deployment untouched"
            );
            None
        }
    };
    clean_exit_action_for_intent(desired)
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

pub(super) fn extract_pod_observed_state(
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

/// Best-effort lookup of the most relevant K8s Event reason+message for a pod.
/// Names the cause of a stuck-Pending / slow-start window (e.g.
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

    let mut message = match cause {
        Some(c) => format!(
            "Turn interrupted — sandbox pod terminated involuntarily ({c}). Resuming when it reschedules."
        ),
        None => {
            "Turn interrupted — sandbox pod terminated involuntarily. Resuming when it reschedules."
                .to_string()
        }
    };
    if let Some(note) = workspace_preservation_note(deps, task_id).await {
        message.push('\n');
        message.push_str(&note);
    }
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

async fn workspace_preservation_note(deps: &WatcherDeps, task_id: &str) -> Option<String> {
    let task_events = deps.task_events.as_ref()?;
    match task_events
        .latest_successful_workspace_upload(task_id)
        .await
    {
        Ok(Some(upload)) => {
            let synced_at = upload
                .created_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            Some(format!(
                "Workspace data was synced to S3 before termination; the latest preserved archive was uploaded at `{synced_at}` ({}).",
                format_bytes(upload.bytes.max(0) as u64)
            ))
        }
        Ok(None) => None,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to read workspace sync marker for termination message");
            None
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
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
pub(super) const FAILED_POD_GC_GRACE_MS: i64 = 15 * 60 * 1000;

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

/// Retry the crash auto-stop for any pod with a still-pending auto-stop.
///
/// The watch stream only fires on pod changes, so if the auto-stop for a
/// crash-looping / exit-78 pod failed and the pod then produces no further
/// watch events, nothing would retry it. This sweep (on the renew tick) closes
/// that gap: it re-issues the lifecycle stop for every pod whose
/// `auto_stop_pending` is still set and clears the flag on success.
async fn retry_pending_auto_stops(
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
        pod_with_phase_and_containers, running_container, terminated_container,
    };

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
    /// "confirming" a scale-to-0 here would patch a Deployment that is already
    /// gone and log a spurious NotFound warning.
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

    #[test]
    fn format_bytes_uses_binary_units() {
        assert_eq!(format_bytes(42), "42 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.0 MiB");
    }
}
