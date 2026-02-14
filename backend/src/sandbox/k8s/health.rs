//! Health check and status fetching for sandbox pods.
//!
//! All status detection uses K8s-native mechanisms (pod conditions, container
//! statuses, readiness probes). Agent state comes via event publishing to Redis.

use axum::response::sse::Event;
use futures::stream::BoxStream;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams, LogParams};
use tracing::warn;

use super::client::K8sClient;
use super::pod_status::extract_container_observed_statuses;
use crate::sandbox::grpc_client::SandboxGrpcClient;
use crate::sandbox::models::ContainerObservedStatus;
use crate::sandbox::schemas::FileSearchResult;

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

/// Stream server container logs via K8s API, converted to SSE events.
pub async fn stream_container_logs_k8s(
    k8s: &K8sClient,
    task_id: &str,
    container_name: &str,
    tail_lines: i64,
) -> Result<BoxStream<'static, Result<Event, crate::sandbox::error::SandboxError>>, String> {
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels(&format!("moatless.io/task-id={task_id}"));

    let pod_name = pods_api
        .list(&lp)
        .await
        .map_err(|e| format!("Failed to list pods: {e}"))?
        .items
        .first()
        .and_then(|p| p.metadata.name.clone())
        .ok_or_else(|| format!("No pod found for task {task_id}"))?;

    let log_params = LogParams {
        container: Some(container_name.to_string()),
        follow: true,
        tail_lines: Some(tail_lines),
        ..Default::default()
    };

    let log_stream = pods_api
        .log_stream(&pod_name, &log_params)
        .await
        .map_err(|e| format!("Failed to stream logs: {e}"))?;

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

/// Search files in a sandbox workspace via gRPC.
pub async fn search_sandbox_files(
    service_name: &str,
    namespace: &str,
    port: u16,
    query: &str,
    limit: u32,
) -> Result<Vec<FileSearchResult>, String> {
    let client = SandboxGrpcClient::for_service(service_name, namespace, port)
        .map_err(|e| format!("Failed to create gRPC client: {e}"))?;

    let files = client.search_files(query, limit as i32).await?;

    Ok(files
        .into_iter()
        .map(|path| {
            let name = path
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .to_string();
            FileSearchResult {
                path,
                name,
                file_type: None,
            }
        })
        .collect())
}
