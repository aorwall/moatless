//! Pod data readers: fetch container statuses and stream logs from running pods.
//!
//! These functions perform K8s API reads against sandbox pods — listing pods
//! by task label, extracting container statuses, and streaming container logs.

use axum::response::sse::Event;
use futures::stream::BoxStream;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams, LogParams};
use tracing::{debug, warn};

use super::client::K8sClient;
use super::pod_status::{extract_all_container_statuses, extract_container_observed_statuses};
use crate::sandbox::models::ContainerObservedStatus;
use crate::sandbox::schemas::RuntimeEvent;

/// Errors from log streaming, distinguishing "no pod" from other failures.
#[derive(Debug)]
pub enum LogStreamError {
    /// No pod found for the given task — sandbox not running.
    NoPod(String),
    /// Other streaming error.
    Other(String),
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
pub async fn fetch_pod_runtime_events(
    k8s: &K8sClient,
    task_id: &str,
) -> Vec<RuntimeEvent> {
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
    let field_selector = format!(
        "involvedObject.name={pod_name},involvedObject.kind=Pod"
    );
    let ep = ListParams::default().fields(&field_selector);

    match events_api.list(&ep).await {
        Ok(event_list) => event_list
            .items
            .into_iter()
            .map(|e| {
                let last_ts = e
                    .last_timestamp
                    .as_ref()
                    .map(|t| t.0.to_string());

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
    tail_lines: i64,
    previous: bool,
) -> Result<BoxStream<'static, Result<Event, crate::sandbox::error::SandboxError>>, LogStreamError>
{
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
                LogStreamError::Other(format!("Failed to fetch previous logs: {e}"))
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

    // Try live (follow) streaming first.
    let live_params = LogParams {
        container: Some(container_name.to_string()),
        follow: true,
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
                    LogStreamError::Other(format!(
                        "Live log stream failed ({live_err}), previous logs also failed ({prev_err})"
                    ))
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
