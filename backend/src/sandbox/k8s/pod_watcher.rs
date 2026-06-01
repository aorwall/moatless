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
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, LogParams};
use kube::runtime::watcher::{self, Config as WatcherConfig};
use moatless_events::{SandboxEventPayload, SandboxStatus, ServerEventPayload, ServerSubType};
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
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::config::Config;
use crate::events::EventPublisher;
use crate::plugin_repo::dao::PluginRepoRepository;
use crate::repository::dao::RepoRepository;
use crate::sandbox::jwt;
use crate::task::dao::{TaskPatch, TaskRepository};

/// Optional DB + config dependencies the watcher needs to auto-disable a
/// crash-looping server container. All repositories are optional because the
/// watcher also runs in setups without a database (where remediation degrades
/// to publishing a `Failed` event only).
#[derive(Clone)]
struct WatcherDeps {
    tasks: Option<Arc<dyn TaskRepository>>,
    repos: Option<Arc<dyn RepoRepository>>,
    provider_tokens: Option<Arc<dyn ProviderTokenRepository>>,
    plugin_repos: Option<Arc<dyn PluginRepoRepository>>,
    config: Arc<Config>,
}

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
    /// Exit code from the sandbox container's terminated state.
    exit_code: Option<i32>,
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
    tasks: Option<Arc<dyn TaskRepository>>,
    repos: Option<Arc<dyn RepoRepository>>,
    provider_tokens: Option<Arc<dyn ProviderTokenRepository>>,
    plugin_repos: Option<Arc<dyn PluginRepoRepository>>,
    config: Arc<Config>,
) -> JoinHandle<()> {
    let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
    let deps = WatcherDeps {
        tasks,
        repos,
        provider_tokens,
        plugin_repos,
        config,
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
                        },
                    )
                    .await;
            }

            // Carry over fatal_published from previous state, then reset on recovery
            if let Some(prev) = prev_state {
                new_state.fatal_published = prev.fatal_published;
                new_state.auto_disabled_servers = prev.auto_disabled_servers.clone();
            }
            if transitions.iter().any(|(sub, _)| sub == "pod_ready") {
                new_state.fatal_published = false;
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

                event_publisher
                    .publish_sandbox_event_with_payload(
                        &task_id,
                        SandboxStatus::Error,
                        &SandboxEventPayload {
                            message: format!("Sandbox terminated: {error_msg}"),
                            lifecycle_event: Some("fatal".to_string()),
                            error: Some(error_msg),
                        },
                    )
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
                        },
                    )
                    .await;

                if let Err(e) = deployment::scale_deployment(k8s, &task_id, 0).await {
                    warn!(task_id = %task_id, error = %e, "Failed to auto-stop permanently failing sandbox");
                }
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
                            },
                        )
                        .await;

                    // Scale deployment to 0
                    if let Err(e) = deployment::scale_deployment(k8s, &task_id, 0).await {
                        warn!(
                            task_id = %task_id,
                            error = %e,
                            "Failed to auto-stop crash-looping sandbox"
                        );
                    }
                }
            }

            // Auto-scale down when sandbox container exits cleanly (Completed).
            // This prevents K8s from restarting the container into CrashLoopBackOff.
            let has_completed = transitions
                .iter()
                .any(|(sub, _)| sub == "sandbox_completed");
            if has_completed && !is_intentional_stop {
                info!(
                    task_id = %task_id,
                    pod_name = %pod_name,
                    "Sandbox completed cleanly, scaling deployment to 0"
                );
                if let Err(e) = deployment::scale_deployment(k8s, &task_id, 0).await {
                    warn!(
                        task_id = %task_id,
                        error = %e,
                        "Failed to scale down completed sandbox"
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
                    let server_name =
                        sc.name.strip_suffix("-server").unwrap_or(&sc.name).to_string();
                    auto_disable_server(
                        k8s,
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
                // Distinguish Stopped vs Removed by checking if Deployment still exists
                let sandbox_status = match deployment::find_deployment(k8s, &task_id).await {
                    Ok(Some(_)) => {
                        // Deployment still exists (replicas=0 after stop) → Stopped
                        info!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            "Pod deleted (Deployment exists) — publishing stopped"
                        );
                        SandboxStatus::Stopped
                    }
                    Ok(None) => {
                        // No Deployment → fully cleaned up
                        info!(
                            task_id = %task_id,
                            pod_name = %pod_name,
                            "Pod deleted (no Deployment) — publishing removed"
                        );
                        SandboxStatus::Removed
                    }
                    Err(e) => {
                        warn!(
                            task_id = %task_id,
                            error = %e,
                            "Failed to check Deployment on pod delete, defaulting to Removed"
                        );
                        SandboxStatus::Removed
                    }
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
    k8s: &K8sClient,
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

    // Need full DB access to determine default-ness, persist, and redeploy.
    let (Some(tasks), Some(repos), Some(provider_tokens), Some(plugin_repos)) = (
        deps.tasks.as_ref(),
        deps.repos.as_ref(),
        deps.provider_tokens.as_ref(),
        deps.plugin_repos.as_ref(),
    ) else {
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
        repos.as_ref(),
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

    // Rebuild the provision request (now filters the disabled server) and
    // redeploy from the new pod spec.
    let request = match crate::sandbox::auto_provision::build_provision_request(
        tasks.as_ref(),
        repos.as_ref(),
        provider_tokens.as_ref(),
        plugin_repos.as_ref(),
        task_id,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to build provision request for redeploy");
            return;
        }
    };

    let Some(secret) = deps.config.sandbox.task_token_secret.as_ref() else {
        warn!(task_id = %task_id, "TASK_TOKEN_SECRET missing — cannot redeploy after auto-disable");
        return;
    };
    let token = match jwt::generate_user_auth_token(&request.user_id, &request.user_id, secret) {
        Ok(t) => t,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to generate auth token for redeploy");
            return;
        }
    };

    if let Err(e) =
        deployment::redeploy_with_plan(k8s, &deps.config, task_id, &request, &token).await
    {
        warn!(task_id = %task_id, error = %e, "Failed to redeploy after auto-disabling server");
    } else {
        info!(
            task_id = %task_id,
            server = %server_name,
            "Auto-disabled crash-looping server and redeployed"
        );
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
        failure_detail,
        server_containers,
        fatal_published: false,
        auto_disabled_servers: HashSet::new(),
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
                // Check if evicted
                let is_evicted = current
                    .terminated_reason
                    .as_deref()
                    .is_some_and(|r| r.contains("Evict"));

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
        "container_restarted" | "pod_recovering" => SandboxStatus::Initializing,
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
        "pod_succeeded" | "sandbox_completed" => "Completed".to_string(),
        "pod_deleted" => "Pod deleted".to_string(),
        other => other.replace('_', " "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::k8s::pod_status::test_helpers::{
        pod_with_phase_and_containers, running_container, waiting_container,
    };

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
}
