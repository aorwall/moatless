//! Background readiness checker for pod startup monitoring.
//!
//! Runs as a fire-and-forget tokio task after provisioning.
//! Phase 1: Watches K8s pod conditions (event-driven via watcher) until Ready.
//! Phase 2: Polls `:8001/status` for per-server readiness (preview, files, etc.).

use futures::TryStreamExt;
use k8s_openapi::api::core::v1::{Event as K8sEvent, Pod};
use kube::api::Api;
use kube::runtime::watcher::{self, Config as WatcherConfig};
use std::collections::{HashMap, HashSet};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use super::client::K8sClient;
use super::health;
use super::status_cache::StatusCache;
use crate::events::RedisEventPublisher;
use crate::sandbox::types::PodPhase;

/// Watches pod readiness in a background task after provisioning.
/// Publishes environment events to Redis when state transitions occur.
pub(super) struct BackgroundReadinessChecker {
    pub(super) k8s: K8sClient,
    pub(super) task_id: String,
    pub(super) service_name: String,
    pub(super) timeout_secs: u64,
    pub(super) status_cache: StatusCache,
    pub(super) event_publisher: Option<RedisEventPublisher>,
}

/// Tracks observable pod state for change-only logging.
#[derive(Default)]
struct ObservedState {
    phase: String,
    ready: bool,
    waiting_reason: Option<String>,
    pod_ip: Option<String>,
}

impl ObservedState {
    /// Update from current pod data. Returns true if any field changed.
    fn update(
        &mut self,
        phase: &str,
        ready: bool,
        waiting_reason: Option<String>,
        pod_ip: Option<&str>,
    ) -> bool {
        let changed = self.phase != phase
            || self.ready != ready
            || self.waiting_reason != waiting_reason
            || self.pod_ip.as_deref() != pod_ip;

        if changed {
            self.phase = phase.to_string();
            self.ready = ready;
            self.waiting_reason = waiting_reason;
            self.pod_ip = pod_ip.map(|s| s.to_string());
        }
        changed
    }
}

impl BackgroundReadinessChecker {
    pub(super) async fn run(&self) {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(self.timeout_secs);

        // Phase 1: Watch for pod readiness via K8s conditions
        // Also returns a handle for the K8s event watcher (startup timeline).
        let result = tokio::time::timeout(timeout, self.wait_for_pod_ready(start)).await;

        // Extract the event watcher handle regardless of outcome
        let (phase1_result, event_watcher_handle) = match result {
            Ok(Ok((ready, handle))) => (Some(Ok(ready)), handle),
            Ok(Err(e)) => (Some(Err(e)), None),
            Err(_) => (None, None), // timeout
        };

        match phase1_result {
            Some(Ok(true)) => {
                // Pod ready — publish event, enter Phase 2
                self.status_cache.invalidate(&self.task_id).await;
                info!(
                    task_id = %self.task_id,
                    elapsed_secs = start.elapsed().as_secs(),
                    "Pod is ready (background check)"
                );
                self.publish_event("ready", None).await;
                self.monitor_servers(start).await;
            }
            Some(Ok(false)) => {
                // Terminal error — already published inside wait_for_pod_ready
            }
            Some(Err(e)) => {
                let msg = format!("Watch error: {e}");
                warn!(task_id = %self.task_id, error = %e, "Pod watch stream failed");
                self.publish_event("error", Some(&msg)).await;
            }
            None => {
                let msg = format!("Pod startup timed out after {}s", self.timeout_secs);
                warn!(
                    task_id = %self.task_id,
                    timeout_secs = self.timeout_secs,
                    "Background readiness check timed out"
                );
                self.publish_event("error", Some(&msg)).await;
            }
        }

        // Stop the K8s event watcher — it's purely informational
        if let Some(handle) = event_watcher_handle {
            handle.abort();
        }
    }

    /// Watch pod events until Ready condition is True, or a terminal failure.
    /// Returns `Ok((true, handle))` when ready, `Ok((false, handle))` on terminal error.
    /// The optional `JoinHandle` is the K8s event watcher (startup timeline).
    async fn wait_for_pod_ready(
        &self,
        start: std::time::Instant,
    ) -> Result<(bool, Option<JoinHandle<()>>), watcher::Error> {
        let pods_api: Api<Pod> =
            Api::namespaced(self.k8s.client.clone(), self.k8s.namespace());
        let wc = WatcherConfig::default()
            .labels(&format!("moatless.io/task-id={}", self.task_id));

        let stream = watcher::watcher(pods_api, wc);
        tokio::pin!(stream);
        let mut state = ObservedState::default();
        let mut event_watcher_handle: Option<JoinHandle<()>> = None;

        while let Some(event) = stream.try_next().await? {
            let pod = match &event {
                watcher::Event::Apply(pod) | watcher::Event::InitApply(pod) => pod,
                watcher::Event::Delete(_) => {
                    warn!(task_id = %self.task_id, "Pod was deleted during startup");
                    self.publish_event("error", Some("Pod deleted during startup"))
                        .await;
                    return Ok((false, event_watcher_handle));
                }
                watcher::Event::Init | watcher::Event::InitDone => continue,
            };

            // Spawn event watcher on first pod event (once we know the pod name)
            if event_watcher_handle.is_none() {
                if let Some(pod_name) = pod.metadata.name.as_deref() {
                    event_watcher_handle =
                        Some(self.spawn_k8s_event_watcher(pod_name));
                }
            }

            let elapsed_secs = start.elapsed().as_secs();

            // Extract pod state
            let raw_phase = pod
                .status
                .as_ref()
                .and_then(|s| s.phase.as_deref())
                .unwrap_or("Unknown");
            let phase = PodPhase::from(raw_phase);
            let waiting_reason = extract_waiting_reason(pod);
            let pod_ip = pod.status.as_ref().and_then(|s| s.pod_ip.as_deref());
            let ready = is_pod_condition_ready(pod);

            // Log on state change
            if state.update(raw_phase, ready, waiting_reason.clone(), pod_ip) {
                info!(
                    task_id = %self.task_id,
                    phase = %raw_phase,
                    ready,
                    pod_ip = ?pod_ip,
                    waiting_reason = ?waiting_reason,
                    elapsed_secs,
                    "Pod state changed"
                );
            }

            // Check terminal phases
            if matches!(phase, PodPhase::Failed | PodPhase::Succeeded) {
                let msg = format!("Pod entered {} phase", phase.as_str());
                warn!(task_id = %self.task_id, phase = phase.as_str(), "Pod entered terminal phase");
                self.publish_event("error", Some(&msg)).await;
                return Ok((false, event_watcher_handle));
            }

            // Check permanent container failures
            if let Some(msg) = check_permanent_failure(pod) {
                warn!(task_id = %self.task_id, error = %msg, "Container hit permanent failure");
                self.publish_event("error", Some(&msg)).await;
                return Ok((false, event_watcher_handle));
            }

            // Check Ready condition (set by kubelet when readinessProbe passes)
            if ready {
                return Ok((true, event_watcher_handle));
            }
        }

        // Stream ended unexpectedly
        warn!(task_id = %self.task_id, "Pod watch stream ended unexpectedly");
        self.publish_event("error", Some("Watch stream ended unexpectedly"))
            .await;
        Ok((false, event_watcher_handle))
    }

    /// Spawn a background task that watches K8s Event objects for the pod
    /// and publishes them as `k8s_event` environment events to Redis.
    /// This provides a real-time startup timeline to the frontend.
    fn spawn_k8s_event_watcher(&self, pod_name: &str) -> JoinHandle<()> {
        let k8s = self.k8s.clone();
        let task_id = self.task_id.clone();
        let pod_name = pod_name.to_string();
        let publisher = self.event_publisher.clone();
        // Give the event watcher enough time to cover Phase 1 + Phase 2
        let timeout_secs = self.timeout_secs + 120;

        tokio::spawn(async move {
            let deadline = std::time::Duration::from_secs(timeout_secs);
            let result = tokio::time::timeout(deadline, async {
                if let Some(ref publisher) = publisher {
                    forward_k8s_events(&k8s, &task_id, &pod_name, publisher).await;
                }
            })
            .await;

            if result.is_err() {
                info!(
                    task_id = %task_id,
                    "K8s event watcher timed out (self-terminating)"
                );
            }
        })
    }

    /// Phase 2: Monitor individual server statuses after the pod is ready.
    ///
    /// Polls the sandbox `/status` endpoint to track when servers (preview, files, etc.)
    /// become ready. Logs only on state changes and publishes events for transitions.
    async fn monitor_servers(&self, overall_start: std::time::Instant) {
        const SERVER_MONITOR_TIMEOUT_SECS: u64 = 120;
        const SERVER_POLL_INTERVAL_SECS: u64 = 3;
        const MAX_CONSECUTIVE_FAILURES: u32 = 5;

        let poll_interval = std::time::Duration::from_secs(SERVER_POLL_INTERVAL_SECS);
        let monitor_start = std::time::Instant::now();
        let deadline =
            monitor_start + std::time::Duration::from_secs(SERVER_MONITOR_TIMEOUT_SECS);

        let mut last_statuses: HashMap<String, String> = HashMap::new();
        let mut consecutive_failures: u32 = 0;

        info!(
            task_id = %self.task_id,
            "Starting server monitoring phase"
        );

        while std::time::Instant::now() < deadline {
            let elapsed_secs = overall_start.elapsed().as_secs();

            match health::fetch_sandbox_status(
                &self.service_name,
                self.k8s.namespace(),
                8001,
            )
            .await
            {
                Some(snapshot) => {
                    consecutive_failures = 0;
                    let mut all_terminal = true;

                    for server in &snapshot.servers {
                        let prev = last_statuses.get(&server.name);
                        // "started" is always terminal.
                        // "failed" is only terminal if we previously saw the server
                        // in "starting" state (meaning it actually attempted to start
                        // and failed). If prev is None or "failed", the server hasn't
                        // been launched yet — the entrypoint runs setup commands
                        // (e.g. bun install) before starting servers.
                        let is_terminal = server.status == "started"
                            || (server.status == "failed"
                                && prev.is_some_and(|p| p != "failed"));

                        if prev.map(|p| p.as_str()) != Some(&server.status) {
                            info!(
                                task_id = %self.task_id,
                                server = %server.name,
                                port = server.port,
                                status = %server.status,
                                prev_status = ?prev,
                                elapsed_secs,
                                "Server status changed"
                            );

                            if server.status == "started" {
                                self.publish_server_event(
                                    "server_ready",
                                    &server.name,
                                    server.port,
                                    &server.status,
                                )
                                .await;
                            } else if server.status == "failed"
                                && prev.is_some_and(|p| p != "failed")
                            {
                                // Only publish server_failed if the server was
                                // previously in a non-failed state (i.e., it
                                // actually attempted to start and then failed).
                                self.publish_server_event(
                                    "server_failed",
                                    &server.name,
                                    server.port,
                                    &server.status,
                                )
                                .await;
                            }

                            last_statuses
                                .insert(server.name.clone(), server.status.clone());
                        }

                        if !is_terminal {
                            all_terminal = false;
                        }
                    }

                    if !snapshot.servers.is_empty() && all_terminal {
                        let summary: Vec<String> = last_statuses
                            .iter()
                            .map(|(name, status)| format!("{name}={status}"))
                            .collect();
                        info!(
                            task_id = %self.task_id,
                            servers = %summary.join(", "),
                            elapsed_secs,
                            "All servers reached terminal state"
                        );
                        return;
                    }
                }
                None => {
                    consecutive_failures += 1;
                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        warn!(
                            task_id = %self.task_id,
                            consecutive_failures,
                            "Sandbox status endpoint unreachable, stopping server monitoring"
                        );
                        return;
                    }
                }
            }

            tokio::time::sleep(poll_interval).await;
        }

        let summary: Vec<String> = last_statuses
            .iter()
            .map(|(name, status)| format!("{name}={status}"))
            .collect();
        warn!(
            task_id = %self.task_id,
            servers = %summary.join(", "),
            timeout_secs = SERVER_MONITOR_TIMEOUT_SECS,
            "Server monitoring timed out with servers not yet ready"
        );
    }

    /// Helper to publish an environment event (fire-and-forget).
    async fn publish_event(&self, sub_type: &str, error_message: Option<&str>) {
        if let Some(ref publisher) = self.event_publisher {
            publisher
                .publish_environment_event(&self.task_id, sub_type, error_message)
                .await;
        }
    }

    /// Helper to publish a server status event with structured payload.
    async fn publish_server_event(
        &self,
        sub_type: &str,
        server_name: &str,
        port: u16,
        status: &str,
    ) {
        if let Some(ref publisher) = self.event_publisher {
            let payload = serde_json::json!({
                "server": server_name,
                "port": port,
                "status": status,
            });
            publisher
                .publish_environment_event_with_payload(&self.task_id, sub_type, payload)
                .await;
        }
    }
}

/// Check if pod has the Ready condition set to True.
fn is_pod_condition_ready(pod: &Pod) -> bool {
    pod.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .map(|conditions| {
            conditions
                .iter()
                .any(|c| c.type_ == "Ready" && c.status == "True")
        })
        .unwrap_or(false)
}

/// Extract the first container's waiting reason, if any.
fn extract_waiting_reason(pod: &Pod) -> Option<String> {
    pod.status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref())
        .and_then(|cs| cs.first())
        .and_then(|cs| cs.state.as_ref())
        .and_then(|s| s.waiting.as_ref())
        .and_then(|w| w.reason.clone())
}

/// Check for permanent container failures. Returns error message if found.
fn check_permanent_failure(pod: &Pod) -> Option<String> {
    let container_statuses = pod.status.as_ref()?.container_statuses.as_ref()?;
    for cs in container_statuses {
        if let Some(waiting) = cs.state.as_ref().and_then(|s| s.waiting.as_ref()) {
            let reason = waiting.reason.as_deref().unwrap_or("");
            const PERMANENT: &[&str] = &[
                "ImagePullBackOff",
                "ErrImagePull",
                "InvalidImageName",
                "CreateContainerConfigError",
            ];
            if PERMANENT.contains(&reason) {
                return Some(format!("Container error: {reason}"));
            }
        }
    }
    None
}

/// Watch K8s Event objects for a specific pod and publish them as Redis events.
///
/// K8s automatically creates `core/v1/Event` objects for pod lifecycle milestones
/// (scheduling, image pulls, container creation, probe results). This function
/// watches those events and forwards them as `k8s_event` environment events,
/// giving the frontend a detailed startup timeline.
async fn forward_k8s_events(
    k8s: &K8sClient,
    task_id: &str,
    pod_name: &str,
    publisher: &RedisEventPublisher,
) {
    let events_api: Api<K8sEvent> =
        Api::namespaced(k8s.client.clone(), k8s.namespace());
    let wc = WatcherConfig::default().fields(&format!(
        "involvedObject.name={},involvedObject.namespace={},involvedObject.kind=Pod",
        pod_name,
        k8s.namespace()
    ));

    let stream = watcher::watcher(events_api, wc);
    tokio::pin!(stream);
    let mut seen: HashSet<String> = HashSet::new();

    while let Ok(Some(event)) = stream.try_next().await {
        let k8s_event = match &event {
            watcher::Event::Apply(e) | watcher::Event::InitApply(e) => e,
            _ => continue,
        };

        let reason = k8s_event.reason.as_deref().unwrap_or("Unknown");
        let message = k8s_event.message.as_deref().unwrap_or("");
        let event_type = k8s_event.type_.as_deref().unwrap_or("Normal");

        // Dedup by reason+message — K8s Events get updated (count incremented)
        // on repeat, and the watcher replays them.
        let key = format!("{reason}:{message}");
        if !seen.insert(key) {
            continue;
        }

        info!(
            task_id = %task_id,
            reason = %reason,
            event_type = %event_type,
            message = %message,
            "K8s event for pod"
        );

        let payload = serde_json::json!({
            "reason": reason,
            "message": message,
            "eventType": event_type,
        });

        publisher
            .publish_environment_event_with_payload(task_id, "k8s_event", payload)
            .await;
    }
}
