//! Pod data readers: fetch container statuses and stream logs from running pods.
//!
//! These functions perform K8s API reads against sandbox pods — listing pods
//! by task label, extracting container statuses, and streaming container logs.

use futures::stream::BoxStream;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams, LogParams};
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{debug, warn};

use super::client::K8sClient;
use super::pod_status::{extract_all_container_statuses, extract_container_observed_statuses};
use moatless_types::workspace::{Cpu, Memory};
use moatless_backend_core::sandbox::{SandboxDiagnosticsContainer, SandboxDiagnosticsEntry};
use moatless_sandbox_runtime::access::LogOptions;
use moatless_sandbox_runtime::models::{ContainerObservedStatus, ContainerState, PodPhase};
use moatless_sandbox_runtime::schemas::RuntimeEvent;

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

/// Stream server container logs via K8s API, one value per line.
///
/// When live streaming fails (e.g. container terminated), falls back to
/// fetching previous container logs (`previous: true`).
pub async fn stream_container_logs_k8s(
    k8s: &K8sClient,
    task_id: &str,
    container_name: &str,
    options: LogOptions,
) -> Result<
    BoxStream<'static, Result<String, moatless_sandbox_runtime::error::SandboxError>>,
    LogStreamError,
> {
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

        let line_stream = async_stream::stream! {
            use futures::AsyncBufReadExt;
            let mut lines = prev_stream.lines();
            while let Some(line_result) = futures::StreamExt::next(&mut lines).await {
                match line_result {
                    Ok(line) => yield Ok(line),
                    Err(_) => break,
                }
            }
        };
        return Ok(Box::pin(line_stream));
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
            let line_stream = async_stream::stream! {
                use futures::AsyncBufReadExt;
                let mut lines = log_stream.lines();
                while let Some(line_result) = futures::StreamExt::next(&mut lines).await {
                    match line_result {
                        Ok(line) => yield Ok(line),
                        Err(_) => break,
                    }
                }
            };
            Ok(Box::pin(line_stream))
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

            let line_stream = async_stream::stream! {
                use futures::AsyncBufReadExt;
                let mut lines = prev_stream.lines();
                while let Some(line_result) = futures::StreamExt::next(&mut lines).await {
                    match line_result {
                        Ok(line) => yield Ok(line),
                        Err(_) => break,
                    }
                }
            };
            Ok(Box::pin(line_stream))
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
            // Live metrics, not config: a quantity the metrics API spells in a
            // form this parser does not know reads as zero rather than dropping
            // the whole sample.
            let cpu = c.usage.cpu.parse::<Cpu>().map_or(0, |q| q.millicores());
            let mem = c.usage.memory.parse::<Memory>().map_or(0, |q| q.bytes());
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

// =============================================================================
// Fleet-wide diagnostics (every Sandbox at once)
// =============================================================================

/// Response from the K8s Metrics API for every pod in a namespace.
#[derive(Debug, Deserialize)]
struct NamespacePodMetrics {
    items: Vec<NamedPodMetrics>,
}

#[derive(Debug, Deserialize)]
struct NamedPodMetrics {
    metadata: PodMetricsObjectMeta,
    containers: Vec<ContainerMetrics>,
}

#[derive(Debug, Deserialize)]
struct PodMetricsObjectMeta {
    name: String,
}

/// Fetch resource usage for every pod in the namespace via one K8s Metrics API
/// read, keyed by pod name then container name.
///
/// Requires metrics-server. Returns an empty map on any failure (metrics
/// unavailable, RBAC missing, a pod not yet scraped) — the same best-effort
/// contract [`fetch_container_resource_usage`] makes for a single pod.
async fn fetch_namespace_resource_usage(
    k8s: &K8sClient,
) -> HashMap<String, HashMap<String, ContainerResourceUsage>> {
    let url = format!(
        "/apis/metrics.k8s.io/v1beta1/namespaces/{}/pods",
        k8s.namespace()
    );

    let request = match hyper::Request::get(&url).body(Vec::new()) {
        Ok(r) => r,
        Err(e) => {
            debug!(error = %e, "Failed to build namespace-wide metrics request");
            return HashMap::new();
        }
    };

    let response: NamespacePodMetrics = match k8s.client.request(request).await {
        Ok(resp) => resp,
        Err(kube::Error::Api(ref e)) if e.code == 404 => {
            debug!(
                "Namespace-wide pod metrics not yet available — metrics-server has not scraped this namespace yet"
            );
            return HashMap::new();
        }
        Err(kube::Error::Api(ref e)) if e.code == 403 => {
            warn!(
                error = %e,
                "Missing RBAC for a namespace-wide metrics.k8s.io/pods read — observability Role not bound"
            );
            return HashMap::new();
        }
        Err(e) => {
            warn!(error = %e, "Failed to fetch namespace-wide pod metrics");
            return HashMap::new();
        }
    };

    response
        .items
        .into_iter()
        .map(|pod| {
            let containers = pod
                .containers
                .into_iter()
                .map(|c| {
                    let cpu = c.usage.cpu.parse::<Cpu>().map_or(0, |q| q.millicores());
                    let mem = c.usage.memory.parse::<Memory>().map_or(0, |q| q.bytes());
                    (
                        c.name.clone(),
                        ContainerResourceUsage {
                            name: c.name,
                            cpu_millicores: cpu,
                            memory_bytes: mem,
                        },
                    )
                })
                .collect();
            (pod.metadata.name, containers)
        })
        .collect()
}

/// Project one pod, and its resource usage if any was found for it, into the
/// fleet-wide diagnostics entry.
///
/// A pod with no `moatless.io/task-id` label is not a Sandbox and is skipped —
/// callers list by that label already, so this only matters for defense in
/// depth. Pure and synchronous, which is what makes it unit-testable without a
/// K8s API: [`fetch_fleet_diagnostics`] is the thin async wrapper that fetches
/// the two inputs this closes over.
fn map_pod_to_diagnostics_entry(
    pod: &Pod,
    usage: Option<&HashMap<String, ContainerResourceUsage>>,
) -> Option<SandboxDiagnosticsEntry> {
    let task_id = pod
        .metadata
        .labels
        .as_ref()?
        .get("moatless.io/task-id")?
        .clone();
    let node_name = pod.spec.as_ref().and_then(|spec| spec.node_name.clone());
    let raw_phase = pod
        .status
        .as_ref()
        .and_then(|status| status.phase.as_deref())
        .unwrap_or("Unknown");
    let sandbox_status = PodPhase::from(raw_phase).sandbox_status();

    let containers: Vec<SandboxDiagnosticsContainer> = extract_all_container_statuses(pod)
        .into_iter()
        .map(|cs| {
            let container_usage = usage.and_then(|by_name| by_name.get(&cs.name));
            SandboxDiagnosticsContainer {
                name: cs.name,
                state: match cs.state {
                    ContainerState::Running => "running".to_string(),
                    ContainerState::Waiting => "waiting".to_string(),
                    ContainerState::Terminated => "terminated".to_string(),
                    ContainerState::Unknown => "unknown".to_string(),
                },
                restart_count: cs.restart_count,
                cpu_millicores: container_usage.map(|u| u.cpu_millicores),
                memory_mb: container_usage.map(|u| u.memory_bytes / (1024 * 1024)),
            }
        })
        .collect();

    let total_cpu_millicores =
        usage.map(|by_name| by_name.values().map(|u| u.cpu_millicores).sum());
    let total_memory_mb =
        usage.map(|by_name| by_name.values().map(|u| u.memory_bytes).sum::<u64>() / (1024 * 1024));

    Some(SandboxDiagnosticsEntry {
        task_id,
        title: None,
        sandbox_status,
        node_name,
        containers,
        total_cpu_millicores,
        total_memory_mb,
    })
}

/// Every Sandbox in the namespace, with its status and — best-effort — live
/// resource usage: one pod LIST (label key present) plus one namespace-wide
/// metrics read, joined by pod and container name.
///
/// Mirrors [`fetch_all_container_statuses`] + [`fetch_container_resource_usage`]
/// for one Task; this is the same combinator run once for the whole fleet
/// instead of once per Task. A failed pod list is a real error — there is
/// nothing to answer with — but a failed metrics read is not: entries still
/// carry status and restart counts with `None` usage, the pod list is fetched
/// first, and metrics are overlaid on it afterward.
pub async fn fetch_fleet_diagnostics(
    k8s: &K8sClient,
) -> Result<Vec<SandboxDiagnosticsEntry>, kube::Error> {
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels("moatless.io/task-id");
    let pods = pods_api.list(&lp).await?;

    let resource_usage = fetch_namespace_resource_usage(k8s).await;

    Ok(pods
        .items
        .iter()
        .filter_map(|pod| {
            let pod_name = pod.metadata.name.as_deref().unwrap_or_default();
            map_pod_to_diagnostics_entry(pod, resource_usage.get(pod_name))
        })
        .collect())
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

    mod fleet_diagnostics {
        use super::*;
        use crate::pod_status::test_helpers::{
            pod_with_phase_and_containers, running_container, waiting_container,
        };
        use k8s_openapi::api::core::v1::PodSpec;
        use std::collections::BTreeMap;

        fn labeled(mut pod: Pod, task_id: &str, node_name: &str) -> Pod {
            pod.metadata.labels = Some(BTreeMap::from([(
                "moatless.io/task-id".to_string(),
                task_id.to_string(),
            )]));
            pod.spec = Some(PodSpec {
                node_name: Some(node_name.to_string()),
                ..Default::default()
            });
            pod
        }

        fn usage(entries: &[(&str, u64, u64)]) -> HashMap<String, ContainerResourceUsage> {
            entries
                .iter()
                .map(|(name, cpu_millicores, memory_bytes)| {
                    (
                        name.to_string(),
                        ContainerResourceUsage {
                            name: name.to_string(),
                            cpu_millicores: *cpu_millicores,
                            memory_bytes: *memory_bytes,
                        },
                    )
                })
                .collect()
        }

        #[test]
        fn maps_a_running_pod_with_metrics_to_an_entry_with_totals() {
            let pod = labeled(
                pod_with_phase_and_containers(
                    "Running",
                    vec![
                        running_container("sandbox", true, true),
                        running_container("files-server", true, true),
                    ],
                ),
                "task-1",
                "node-a",
            );
            let usage = usage(&[
                ("sandbox", 100, 200 * 1024 * 1024),
                ("files-server", 50, 100 * 1024 * 1024),
            ]);

            let entry = map_pod_to_diagnostics_entry(&pod, Some(&usage)).expect("labeled pod maps");

            assert_eq!(entry.task_id, "task-1");
            assert_eq!(entry.node_name.as_deref(), Some("node-a"));
            assert_eq!(entry.containers.len(), 2);
            let sandbox = entry
                .containers
                .iter()
                .find(|c| c.name == "sandbox")
                .expect("sandbox container present");
            assert_eq!(sandbox.state, "running");
            assert_eq!(sandbox.cpu_millicores, Some(100));
            assert_eq!(sandbox.memory_mb, Some(200));
            assert_eq!(entry.total_cpu_millicores, Some(150));
            assert_eq!(entry.total_memory_mb, Some(300));
        }

        #[test]
        fn a_pod_with_no_metrics_has_none_usage_and_none_totals() {
            let pod = labeled(
                pod_with_phase_and_containers(
                    "Pending",
                    vec![waiting_container("sandbox", "PodInitializing")],
                ),
                "task-2",
                "node-b",
            );

            let entry = map_pod_to_diagnostics_entry(&pod, None).expect("labeled pod maps");

            assert_eq!(entry.containers.len(), 1);
            assert_eq!(entry.containers[0].cpu_millicores, None);
            assert_eq!(entry.containers[0].memory_mb, None);
            assert_eq!(entry.total_cpu_millicores, None);
            assert_eq!(entry.total_memory_mb, None);
        }

        #[test]
        fn a_pod_without_the_task_id_label_is_skipped() {
            let pod = pod_with_phase_and_containers(
                "Running",
                vec![running_container("sandbox", true, true)],
            );

            assert!(map_pod_to_diagnostics_entry(&pod, None).is_none());
        }
    }
}
