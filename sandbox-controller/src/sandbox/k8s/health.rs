//! HTTP health check and status fetching for worker pods.

use http_body_util::BodyExt;
use std::time::Duration;
use tracing::debug;

use crate::sandbox::types::SandboxSnapshotStatus;

/// Check if a worker pod is healthy via its HTTP health endpoint.
///
/// Uses a simple TCP connect + HTTP GET. Returns true if the pod responds with 2xx.
pub async fn check_pod_health(host: &str, port: u16) -> bool {
    let url = format!("http://{}:{}/health", host, port);

    let result = tokio::time::timeout(Duration::from_secs(5), async {
        match hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build_http::<axum::body::Body>()
            .get(url.parse().unwrap())
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    })
    .await;

    match result {
        Ok(healthy) => healthy,
        Err(_) => {
            debug!(host, port, "Health check timed out");
            false
        }
    }
}

/// Check health using K8s service DNS
pub async fn check_service_health(service_name: &str, namespace: &str, port: u16) -> bool {
    let host = format!("{}.{}.svc.cluster.local", service_name, namespace);
    check_pod_health(&host, port).await
}

/// Fetch the sandbox snapshot status from a running sandbox pod's `/status` endpoint.
///
/// Returns `None` if the request fails or times out (5s).
pub async fn fetch_sandbox_status(
    service_name: &str,
    namespace: &str,
    port: u16,
) -> Option<SandboxSnapshotStatus> {
    let host = format!("{}.{}.svc.cluster.local", service_name, namespace);
    let url = format!("http://{}:{}/status", host, port);

    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let client = hyper_util::client::legacy::Client::builder(
            hyper_util::rt::TokioExecutor::new(),
        )
        .build_http::<axum::body::Body>();

        match client.get(url.parse().unwrap()).await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    return None;
                }
                let body = resp.into_body().collect().await.ok()?.to_bytes();
                serde_json::from_slice(&body).ok()
            }
            Err(_) => None,
        }
    })
    .await;

    match result {
        Ok(status) => status,
        Err(_) => {
            debug!(service_name, "Sandbox status fetch timed out");
            None
        }
    }
}
