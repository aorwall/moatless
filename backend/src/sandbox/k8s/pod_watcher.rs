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

use std::collections::HashMap;

use futures::TryStreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::Api;
use kube::runtime::watcher::{self, Config as WatcherConfig};
use moatless_events::{
    AgentSubType, SandboxSubType, ServerEventPayload, ServerSubType,
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::deployment;
use super::leader_election::LeaderElection;
use super::pod_status::extract_container_observed_statuses;
use crate::db::DbPool;
use crate::events::RedisEventPublisher;
use crate::sandbox::types::EnvironmentStatus;

/// Observed state of a server container, used for transition detection.
#[derive(Debug, Clone, PartialEq)]
struct ServerContainerState {
    name: String,
    status: String,
    error: Option<String>,
    detail: Option<String>,
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
    /// Exit code from the sandbox container's terminated state.
    exit_code: Option<i32>,
    /// Termination message or waiting message from the sandbox container.
    failure_detail: Option<String>,
    /// Server container snapshots (excludes sandbox container).
    server_containers: Vec<ServerContainerState>,
}

/// Spawn the pod lifecycle watcher as a background tokio task.
///
/// The watcher runs in a loop:
/// 1. Try to acquire leadership via K8s Lease.
/// 2. If leader, start watching pods.
/// 3. If not leader, sleep and retry.
/// 4. On watch stream error, re-enter the leadership loop.
pub fn spawn_pod_lifecycle_watcher(
    k8s: K8sClient,
    event_publisher: RedisEventPublisher,
    db_pool: Option<DbPool>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());

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
                        &event_publisher,
                        &db_pool,
                        &leader_election,
                        &cancel,
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
                _ = cancel.cancelled() => {
                    info!("Pod watcher shutting down (cancellation)");
                    return;
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
            }
        }
    })
}

/// Run the pod watch stream until it ends or leadership is lost.
async fn run_pod_watch(
    k8s: &K8sClient,
    event_publisher: &RedisEventPublisher,
    db_pool: &Option<DbPool>,
    leader_election: &LeaderElection,
    cancel: &CancellationToken,
) {
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let wc = WatcherConfig::default().labels("app.kubernetes.io/managed-by=moatless");

    let stream = watcher::watcher(pods_api, wc);
    tokio::pin!(stream);

    let mut pod_states: HashMap<String, PodObservedState> = HashMap::new();
    let mut renew_interval = tokio::time::interval(std::time::Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
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
            }
            event = stream.try_next() => {
                match event {
                    Ok(Some(watcher_event)) => {
                        process_watcher_event(
                            k8s,
                            &watcher_event,
                            &mut pod_states,
                            event_publisher,
                            db_pool,
                        ).await;
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
async fn process_watcher_event(
    k8s: &K8sClient,
    event: &watcher::Event<Pod>,
    pod_states: &mut HashMap<String, PodObservedState>,
    event_publisher: &RedisEventPublisher,
    _db_pool: &Option<DbPool>,
) {
    match event {
        watcher::Event::Apply(pod) | watcher::Event::InitApply(pod) => {
            let is_init = matches!(event, watcher::Event::InitApply(_));
            let task_id = match extract_task_id(pod) {
                Some(id) => id,
                None => {
                    let name = pod.metadata.name.as_deref().unwrap_or("unknown");
                    warn!(pod_name = name, "Pod has no moatless.io/task-id label — skipping");
                    return;
                }
            };
            let pod_name = pod
                .metadata
                .name
                .as_deref()
                .unwrap_or("unknown")
                .to_string();

            let new_state = extract_pod_observed_state(pod, &pod_name, &task_id);

            // During InitApply, populate the map silently without publishing events
            if is_init {
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
                    let payload = serde_json::json!({
                        "lifecycleEvent": "pod_recovering",
                    });
                    event_publisher
                        .publish_sandbox_event_with_payload(
                            &task_id,
                            SandboxSubType::Initializing,
                            payload,
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
                            error: sc.and_then(|s| s.error.clone()),
                            detail: sc.and_then(|s| s.detail.clone()),
                        },
                    )
                    .await;
            }

            // If sandbox entered an error state, force all servers to Failed.
            // When the pod crashes, server containers terminate too — semantically they failed.
            let has_error_transition = transitions.iter().any(|(sub_type, _)| {
                matches!(
                    lifecycle_sub_type_to_env_status(sub_type),
                    EnvironmentStatus::Error
                )
            });

            if has_error_transition {
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
                                error: sc
                                    .error
                                    .clone()
                                    .or_else(|| Some("Pod terminated".to_string())),
                                detail: None,
                            },
                        )
                        .await;
                }
            }

            for (sub_type, error_msg) in &transitions {
                let env_status = lifecycle_sub_type_to_env_status(sub_type);
                let sandbox_sub = env_status_to_sandbox_sub_type(&env_status);

                info!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    lifecycle_event = %sub_type,
                    env_status = %env_status,
                    error = ?error_msg,
                    exit_code = ?new_state.exit_code,
                    failure_detail = ?new_state.failure_detail,
                    "Pod lifecycle event detected"
                );

                let payload = serde_json::json!({
                    "lifecycleEvent": sub_type,
                    "error": error_msg,
                });
                event_publisher
                    .publish_sandbox_event_with_payload(
                        &task_id,
                        sandbox_sub,
                        payload,
                    )
                    .await;
            }

            // If any transition is fatal, also publish agent.error
            // so the event handler creates an error result message
            let has_fatal = transitions.iter().any(|(sub_type, _)| {
                matches!(
                    sub_type.as_str(),
                    "container_oom_killed"
                        | "container_crash_loop"
                        | "pod_failed"
                        | "pod_evicted"
                        | "container_terminated"
                )
            });

            if has_fatal {
                let error_msg = transitions
                    .iter()
                    .filter_map(|(_, err)| err.as_ref())
                    .next()
                    .cloned()
                    .unwrap_or_else(|| "Sandbox terminated unexpectedly".to_string());

                event_publisher
                    .publish_agent_event(
                        &task_id,
                        AgentSubType::Error,
                        None,
                        serde_json::json!({
                            "error": format!("Sandbox terminated: {}", error_msg),
                            "source": "pod_watcher",
                        }),
                    )
                    .await;
            }

            pod_states.insert(pod_name, new_state);
        }
        watcher::Event::Delete(pod) => {
            let task_id = match extract_task_id(pod) {
                Some(id) => id,
                None => {
                    let name = pod.metadata.name.as_deref().unwrap_or("unknown");
                    warn!(pod_name = name, "Deleted pod has no moatless.io/task-id label — skipping");
                    return;
                }
            };
            let pod_name = pod
                .metadata
                .name
                .as_deref()
                .unwrap_or("unknown");

            // Capture server state BEFORE removing from pod_states
            let deleted_pod_servers: Vec<ServerContainerState> = pod_states
                .get(pod_name)
                .map(|s| s.server_containers.clone())
                .unwrap_or_default();

            // Remove the specific pod entry
            pod_states.remove(pod_name);

            // Check if sibling pods for this task still exist
            let siblings = sibling_pods_for_task(pod_states, &task_id, pod_name);

            if !siblings.is_empty() {
                info!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    sibling_count = siblings.len(),
                    "Old pod deleted but siblings exist — skipping NotCreated"
                );
            } else {
                // Distinguish Stopped vs NotCreated by checking if Deployment still exists
                let (env_status, sandbox_sub) = match deployment::find_deployment(k8s, &task_id).await {
                    Ok(Some(_)) => {
                        // Deployment still exists (replicas=0 after stop) → Stopped
                        info!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            "Pod deleted (Deployment exists) — publishing stopped"
                        );
                        (EnvironmentStatus::Stopped, SandboxSubType::Stopped)
                    }
                    Ok(None) => {
                        // No Deployment → fully cleaned up
                        info!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            "Pod deleted (no Deployment) — publishing pod_deleted"
                        );
                        (EnvironmentStatus::NotCreated, SandboxSubType::NotCreated)
                    }
                    Err(e) => {
                        warn!(
                            task_id = %task_id,
                            error = %e,
                            "Failed to check Deployment on pod delete, defaulting to NotCreated"
                        );
                        (EnvironmentStatus::NotCreated, SandboxSubType::NotCreated)
                    }
                };

                // Publish server events for each server in the deleted pod
                let server_sub = if env_status == EnvironmentStatus::NotCreated {
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
                                error: if env_status == EnvironmentStatus::NotCreated {
                                    Some("Pod deleted".to_string())
                                } else {
                                    None
                                },
                                detail: None,
                            },
                        )
                        .await;
                }

                let payload = serde_json::json!({
                    "lifecycleEvent": "pod_deleted",
                });
                event_publisher
                    .publish_sandbox_event_with_payload(
                        &task_id,
                        sandbox_sub,
                        payload,
                    )
                    .await;

                // For unexpected deletions (no Deployment), also publish agent.error
                // so the event handler creates an error result message
                if env_status == EnvironmentStatus::NotCreated {
                    event_publisher
                        .publish_agent_event(
                            &task_id,
                            AgentSubType::Error,
                            None,
                            serde_json::json!({
                                "error": "Sandbox terminated: Pod deleted",
                                "source": "pod_watcher",
                            }),
                        )
                        .await;
                }
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

/// Extract the task ID from a pod's labels.
fn extract_task_id(pod: &Pod) -> Option<String> {
    pod.metadata
        .labels
        .as_ref()
        .and_then(|labels| labels.get("moatless.io/task-id"))
        .cloned()
}

/// Build the observed state from a pod's current status.
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
        warn!(task_id, pod_name, "No sandbox container found in running pod");
    }

    let restart_count = sandbox_container_status
        .map(|cs| cs.restart_count)
        .unwrap_or(0);

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

    let ready = pod
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .map(|conditions| {
            conditions
                .iter()
                .any(|c| c.type_ == "Ready" && c.status == "True")
        })
        .unwrap_or(false);

    // Extract server container states
    let server_containers = extract_container_observed_statuses(pod)
        .into_iter()
        .map(|cs| {
            ServerContainerState {
                name: cs.name.clone(),
                status: cs.to_server_status().to_string(),
                error: cs.error_reason(),
                detail: cs.detail_reason(),
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
        failure_detail,
        server_containers,
    }
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
    let prev_phase = prev.map(|p| p.phase.as_str()).unwrap_or("Unknown");
    if current.phase != prev_phase {
        match current.phase.as_str() {
            "Failed" => {
                // Check if evicted
                let is_evicted = current
                    .terminated_reason
                    .as_deref()
                    .map(|r| r.contains("Evict"))
                    .unwrap_or(false);

                if is_evicted {
                    events.push((
                        "pod_evicted".to_string(),
                        Some("Pod was evicted".to_string()),
                    ));
                } else {
                    events.push((
                        "pod_failed".to_string(),
                        Some("Pod entered Failed phase".to_string()),
                    ));
                }
            }
            "Succeeded" => {
                events.push((
                    "pod_succeeded".to_string(),
                    None,
                ));
            }
            _ => {}
        }
    }

    // Check for pod becoming ready (not-ready → ready)
    if current.ready {
        let was_ready = prev.map(|p| p.ready).unwrap_or(false);
        if !was_ready {
            events.push(("pod_ready".to_string(), None));
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
            events.push((
                sub_type,
                server_name.to_string(),
                sc.status.clone(),
            ));
        }
    }

    events
}

/// Map a lifecycle event sub_type to the corresponding EnvironmentStatus.
fn lifecycle_sub_type_to_env_status(sub_type: &str) -> EnvironmentStatus {
    match sub_type {
        "pod_deleted" => EnvironmentStatus::NotCreated,
        "pod_evicted" | "pod_failed" | "container_oom_killed" | "container_terminated"
        | "container_crash_loop" => EnvironmentStatus::Error,
        "pod_ready" => EnvironmentStatus::Ready,
        "pod_succeeded" => EnvironmentStatus::Stopped,
        "container_restarted" | "pod_recovering" => EnvironmentStatus::Initializing,
        _ => EnvironmentStatus::Error,
    }
}

/// Convert EnvironmentStatus to the corresponding SandboxSubType.
fn env_status_to_sandbox_sub_type(env_status: &EnvironmentStatus) -> SandboxSubType {
    match env_status {
        EnvironmentStatus::NotCreated => SandboxSubType::NotCreated,
        EnvironmentStatus::Initializing => SandboxSubType::Initializing,
        EnvironmentStatus::Ready => SandboxSubType::Ready,
        EnvironmentStatus::Stopped => SandboxSubType::Stopped,
        EnvironmentStatus::Removing => SandboxSubType::Removing,
        EnvironmentStatus::Error => SandboxSubType::Error,
    }
}


