//! Pod data readers: fetch container statuses and stream logs from running pods.
//!
//! These functions perform K8s API reads against sandbox pods — listing pods
//! by task label, extracting container statuses, and streaming container logs.

use axum::response::sse::Event;
use futures::stream::BoxStream;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams, LogParams};
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{debug, warn};

use super::client::K8sClient;
use super::pod_status::{extract_all_container_statuses, extract_container_observed_statuses};
use crate::sandbox::access::LogOptions;
use crate::sandbox::models::ContainerObservedStatus;
use crate::sandbox::schemas::RuntimeEvent;

/// Errors from log streaming, distinguishing "not ready yet" from other failures.
#[derive(Debug)]
pub enum LogStreamError {
    /// No pod found for the given task — sandbox not running.
    NoPod(String),
    /// Container exists but has not started yet (e.g. PodInitializing) — no logs available.
    NotStarted(String),
    /// No previous instance of the container to read logs from.
    NoPreviousLogs(String),
    /// Other streaming error.
    Other(String),
}

/// K8s rejects log reads with 400 while a container is waiting to start
/// (PodInitializing, ContainerCreating, ...). Returns the API message if so.
fn container_waiting_message(e: &kube::Error) -> Option<&str> {
    match e {
        kube::Error::Api(api) if api.code == 400 && api.message.contains("waiting to start") => {
            Some(&api.message)
        }
        _ => None,
    }
}

/// K8s rejects `previous: true` log reads with 400 when the container has no
/// terminated predecessor.
fn no_previous_container(e: &kube::Error) -> bool {
    matches!(e, kube::Error::Api(api) if api.code == 400 && api.message.contains("not found"))
}

/// Fetch server container statuses from K8s API.
///
/// Reads containerStatuses from the pod matching the task ID,
/// excludes the "sandbox" container, and returns observed statuses.
pub async fn fetch_server_container_statuses(
    k8s: &K8sClient,
    task_id: &str,
) -> Vec<ContainerObservedStatus> {
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    match pods_api.list(&lp).await {
        Ok(list) => {
            if let Some(pod) = list.items.first() {
                extract_container_observed_statuses(pod)
            } else {
                vec![]
            }
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to fetch pod for container statuses");
            vec![]
        }
    }
}

/// Fetch observed statuses for **all** containers (including sandbox) from K8s API.
///
/// Used for diagnostics/troubleshooting — includes the sandbox container.
pub async fn fetch_all_container_statuses(
    k8s: &K8sClient,
    task_id: &str,
) -> Vec<ContainerObservedStatus> {
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    match pods_api.list(&lp).await {
        Ok(list) => {
            if let Some(pod) = list.items.first() {
                extract_all_container_statuses(pod)
            } else {
                vec![]
            }
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to fetch pod for container diagnostics");
            vec![]
        }
    }
}

/// Fetch runtime events associated with a sandbox pod.
///
/// Uses the K8s Event API with a field selector on the pod name,
/// mapping K8s events into provider-agnostic `RuntimeEvent` structs.
/// Returns an empty vec if no pod or events are found.
pub async fn fetch_pod_runtime_events(k8s: &K8sClient, task_id: &str) -> Vec<RuntimeEvent> {
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    let pod_name = match pods_api.list(&lp).await {
        Ok(list) => match list.items.first().and_then(|p| p.metadata.name.clone()) {
            Some(name) => name,
            None => return vec![],
        },
        Err(_) => return vec![],
    };

    let events_api: Api<k8s_openapi::api::core::v1::Event> =
        Api::namespaced(k8s.client.clone(), k8s.namespace());
    let field_selector = format!("involvedObject.name={pod_name},involvedObject.kind=Pod");
    let ep = ListParams::default().fields(&field_selector);

    match events_api.list(&ep).await {
        Ok(event_list) => event_list
            .items
            .into_iter()
            .map(|e| {
                let last_ts = e.last_timestamp.as_ref().map(|t| t.0.to_string());

                RuntimeEvent {
                    reason: e.reason.unwrap_or_default(),
                    message: e.message.unwrap_or_default(),
                    severity: e.type_.map(|t| t.to_lowercase()),
                    count: e.count,
                    last_timestamp: last_ts,
                }
            })
            .collect(),
        Err(e) => {
            warn!(task_id, error = %e, "Failed to fetch K8s events for pod");
            vec![]
        }
    }
}

/// Stream server container logs via K8s API, converted to SSE events.
///
/// When live streaming fails (e.g. container terminated), falls back to
/// fetching previous container logs (`previous: true`).
pub async fn stream_container_logs_k8s(
    k8s: &K8sClient,
    task_id: &str,
    container_name: &str,
    options: LogOptions,
) -> Result<BoxStream<'static, Result<Event, crate::sandbox::error::SandboxError>>, LogStreamError>
{
    let LogOptions {
        previous,
        follow,
        tail_lines,
    } = options;
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    let pod_name = pods_api
        .list(&lp)
        .await
        .map_err(|e| LogStreamError::Other(format!("Failed to list pods: {e}")))?
        .items
        .first()
        .and_then(|p| p.metadata.name.clone())
        .ok_or_else(|| LogStreamError::NoPod(format!("No pod found for task {task_id}")))?;

    // If previous logs requested, skip live streaming entirely.
    if previous {
        let prev_params = LogParams {
            container: Some(container_name.to_string()),
            follow: false,
            previous: true,
            tail_lines: Some(tail_lines),
            ..Default::default()
        };

        let prev_stream = pods_api
            .log_stream(&pod_name, &prev_params)
            .await
            .map_err(|e| {
                if no_previous_container(&e) {
                    LogStreamError::NoPreviousLogs(format!(
                        "No previous logs for container {container_name}"
                    ))
                } else {
                    LogStreamError::Other(format!("Failed to fetch previous logs: {e}"))
                }
            })?;

        let event_stream = async_stream::stream! {
            use futures::AsyncBufReadExt;
            let mut lines = prev_stream.lines();
            while let Some(line_result) = futures::StreamExt::next(&mut lines).await {
                match line_result {
                    Ok(line) => yield Ok(Event::default().data(line)),
                    Err(_) => break,
                }
            }
        };
        return Ok(Box::pin(event_stream));
    }

    // Try the live container first. With `follow` the stream stays open; without
    // it K8s returns what is buffered and closes, which is what ends the SSE
    // response for a caller that only wants a snapshot.
    let live_params = LogParams {
        container: Some(container_name.to_string()),
        follow,
        tail_lines: Some(tail_lines),
        ..Default::default()
    };

    match pods_api.log_stream(&pod_name, &live_params).await {
        Ok(log_stream) => {
            let event_stream = async_stream::stream! {
                use futures::AsyncBufReadExt;
                let mut lines = log_stream.lines();
                while let Some(line_result) = futures::StreamExt::next(&mut lines).await {
                    match line_result {
                        Ok(line) => yield Ok(Event::default().data(line)),
                        Err(_) => break,
                    }
                }
            };
            Ok(Box::pin(event_stream))
        }
        Err(live_err) => {
            // Live streaming failed (container may be terminated) — try previous logs.
            debug!(
                task_id,
                container = container_name,
                error = %live_err,
                "Live log stream failed, falling back to previous container logs"
            );

            let prev_params = LogParams {
                container: Some(container_name.to_string()),
                follow: false,
                previous: true,
                tail_lines: Some(tail_lines),
                ..Default::default()
            };

            let prev_stream = pods_api
                .log_stream(&pod_name, &prev_params)
                .await
                .map_err(|prev_err| {
                    // A container that never started has no logs, live or previous.
                    if let Some(msg) = container_waiting_message(&live_err) {
                        LogStreamError::NotStarted(msg.to_string())
                    } else {
                        LogStreamError::Other(format!(
                            "Live log stream failed ({live_err}), previous logs also failed ({prev_err})"
                        ))
                    }
                })?;

            let event_stream = async_stream::stream! {
                use futures::AsyncBufReadExt;
                let mut lines = prev_stream.lines();
                while let Some(line_result) = futures::StreamExt::next(&mut lines).await {
                    match line_result {
                        Ok(line) => yield Ok(Event::default().data(line)),
                        Err(_) => break,
                    }
                }
            };
            Ok(Box::pin(event_stream))
        }
    }
}

// =============================================================================
// Resource metrics from K8s Metrics API
// =============================================================================

/// Per-container resource usage from the K8s Metrics API.
#[derive(Debug, Clone)]
pub struct ContainerResourceUsage {
    pub name: String,
    /// CPU usage in millicores (e.g. 250 = 0.25 cores)
    pub cpu_millicores: u64,
    /// Memory usage in bytes
    pub memory_bytes: u64,
}

/// Response from the K8s Metrics API for a single pod.
#[derive(Debug, Deserialize)]
struct PodMetrics {
    containers: Vec<ContainerMetrics>,
}

#[derive(Debug, Deserialize)]
struct ContainerMetrics {
    name: String,
    usage: ContainerUsage,
}

#[derive(Debug, Deserialize)]
struct ContainerUsage {
    cpu: String,
    memory: String,
}

/// Fetch resource usage for all containers in a task's pod via the K8s Metrics API.
///
/// Requires metrics-server to be installed in the cluster.
/// Returns an empty map on any failure (metrics unavailable, pod not found, etc.).
pub async fn fetch_container_resource_usage(
    k8s: &K8sClient,
    task_id: &str,
) -> HashMap<String, ContainerResourceUsage> {
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    let pod_name = match pods_api.list(&lp).await {
        Ok(list) => match list.items.first().and_then(|p| p.metadata.name.clone()) {
            Some(name) => name,
            None => return HashMap::new(),
        },
        Err(_) => return HashMap::new(),
    };

    // Query: /apis/metrics.k8s.io/v1beta1/namespaces/{ns}/pods/{pod}
    let url = format!(
        "/apis/metrics.k8s.io/v1beta1/namespaces/{}/pods/{pod_name}",
        k8s.namespace()
    );

    let request = match hyper::Request::get(&url).body(Vec::new()) {
        Ok(r) => r,
        Err(e) => {
            debug!(task_id, error = %e, "Failed to build metrics request");
            return HashMap::new();
        }
    };

    let response: PodMetrics = match k8s.client.request(request).await {
        Ok(resp) => resp,
        Err(kube::Error::Api(ref e)) if e.code == 404 => {
            debug!(
                task_id,
                pod_name,
                "Pod metrics not yet available — metrics-server has not scraped this pod yet"
            );
            return HashMap::new();
        }
        Err(kube::Error::Api(ref e)) if e.code == 403 => {
            warn!(
                task_id,
                error = %e,
                "Missing RBAC for metrics.k8s.io/pods — observability Role not bound"
            );
            return HashMap::new();
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to fetch pod metrics");
            return HashMap::new();
        }
    };

    response
        .containers
        .into_iter()
        .map(|c| {
            let cpu = parse_cpu_to_millicores(&c.usage.cpu);
            let mem = parse_memory_to_bytes(&c.usage.memory);
            (
                c.name.clone(),
                ContainerResourceUsage {
                    name: c.name,
                    cpu_millicores: cpu,
                    memory_bytes: mem,
                },
            )
        })
        .collect()
}

/// Parse K8s CPU quantity (e.g. "250m", "1", "100n") to millicores.
pub(crate) fn parse_cpu_to_millicores(s: &str) -> u64 {
    if let Some(nano) = s.strip_suffix('n') {
        // Nanocores → millicores
        nano.parse::<u64>().unwrap_or(0) / 1_000_000
    } else if let Some(micro) = s.strip_suffix('u') {
        micro.parse::<u64>().unwrap_or(0) / 1_000
    } else if let Some(milli) = s.strip_suffix('m') {
        milli.parse::<u64>().unwrap_or(0)
    } else {
        // Whole cores
        s.parse::<f64>().map_or(0, |v| (v * 1000.0) as u64)
    }
}

/// Parse K8s memory quantity (e.g. "128Mi", "1Gi", "256000Ki") to bytes.
pub(crate) fn parse_memory_to_bytes(s: &str) -> u64 {
    if let Some(v) = s.strip_suffix("Ki") {
        v.parse::<u64>().unwrap_or(0) * 1024
    } else if let Some(v) = s.strip_suffix("Mi") {
        v.parse::<u64>().unwrap_or(0) * 1024 * 1024
    } else if let Some(v) = s.strip_suffix("Gi") {
        v.parse::<u64>().unwrap_or(0) * 1024 * 1024 * 1024
    } else if let Some(v) = s.strip_suffix('k') {
        v.parse::<u64>().unwrap_or(0) * 1000
    } else if let Some(v) = s.strip_suffix('M') {
        v.parse::<u64>().unwrap_or(0) * 1_000_000
    } else if let Some(v) = s.strip_suffix('G') {
        v.parse::<u64>().unwrap_or(0) * 1_000_000_000
    } else {
        // Plain bytes
        s.parse::<u64>().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_error(code: u16, message: &str) -> kube::Error {
        kube::Error::Api(Box::new(
            kube::core::Status::failure(message, "BadRequest").with_code(code),
        ))
    }

    #[test]
    fn detects_container_waiting_to_start() {
        let e = api_error(
            400,
            "container \"sandbox\" in pod \"p\" is waiting to start: PodInitializing",
        );
        assert!(container_waiting_message(&e).is_some());
        assert!(!no_previous_container(&e));
    }

    #[test]
    fn detects_missing_previous_container() {
        let e = api_error(
            400,
            "previous terminated container \"sandbox\" in pod \"p\" not found",
        );
        assert!(no_previous_container(&e));
        assert!(container_waiting_message(&e).is_none());
    }

    #[test]
    fn other_errors_are_not_classified() {
        let e = api_error(500, "etcd unavailable");
        assert!(container_waiting_message(&e).is_none());
        assert!(!no_previous_container(&e));
    }
}
