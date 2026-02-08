//! HTTP health check and status fetching for sandbox pods.

use axum::response::sse::Event;
use futures::stream::BoxStream;
use http_body_util::BodyExt;
use std::time::Duration;
use tracing::debug;

use crate::sandbox::error::SandboxError;
use crate::sandbox::types::{FileSearchResult, SandboxSnapshotStatus};

/// Check if a sandbox pod is healthy via its HTTP health endpoint.
///
/// Uses a simple TCP connect + HTTP GET. Returns true if the pod responds with 2xx.
pub async fn check_pod_health(host: &str, port: u16) -> bool {
    let url = format!("http://{host}:{port}/health");

    let result = tokio::time::timeout(Duration::from_secs(2), async {
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
    let host = format!("{service_name}.{namespace}.svc.cluster.local");
    check_pod_health(&host, port).await
}

/// Fetch the sandbox snapshot status from a running sandbox pod's `/status` endpoint.
///
/// Returns `None` if the request fails or times out (2s).
pub async fn fetch_sandbox_status(
    service_name: &str,
    namespace: &str,
    port: u16,
) -> Option<SandboxSnapshotStatus> {
    let host = format!("{service_name}.{namespace}.svc.cluster.local");
    let url = format!("http://{host}:{port}/status");

    let result = tokio::time::timeout(Duration::from_secs(2), async {
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

/// Stream server logs from the sandbox pod via SSE proxy.
///
/// Connects to the sandbox pod's `/servers/{name}/logs` SSE endpoint
/// and forwards events to the caller.
pub async fn stream_server_logs(
    service_name: &str,
    namespace: &str,
    port: u16,
    server_name: &str,
) -> Result<BoxStream<'static, Result<Event, SandboxError>>, String> {
    let host = format!("{service_name}.{namespace}.svc.cluster.local");
    let url = format!("http://{host}:{port}/servers/{server_name}/logs");

    let client = hyper_util::client::legacy::Client::builder(
        hyper_util::rt::TokioExecutor::new(),
    )
    .build_http::<axum::body::Body>();

    let resp = client
        .get(url.parse().map_err(|e| format!("Invalid URL: {e}"))?)
        .await
        .map_err(|e| format!("Failed to connect to sandbox for logs: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Sandbox logs endpoint returned status {}",
            resp.status()
        ));
    }

    // Convert the response body into an SSE stream
    let body = resp.into_body();
    let stream = futures::stream::unfold(
        http_body_util::BodyStream::new(body),
        |mut body_stream| async move {
            use futures::StreamExt;
            match body_stream.next().await {
                Some(Ok(frame)) => {
                    if let Ok(bytes) = frame.into_data() {
                        let text = String::from_utf8_lossy(&bytes).to_string();
                        // Parse SSE lines: lines starting with "data: " contain event data
                        for line in text.lines() {
                            if let Some(data) = line.strip_prefix("data: ") {
                                return Some((
                                    Ok(Event::default().data(data.to_string())),
                                    body_stream,
                                ));
                            }
                        }
                        // Non-data line, emit as comment to keep stream alive
                        Some((Ok(Event::default().comment("keepalive")), body_stream))
                    } else {
                        Some((Ok(Event::default().comment("keepalive")), body_stream))
                    }
                }
                Some(Err(_e)) => None,
                None => None,
            }
        },
    );

    Ok(Box::pin(stream))
}

/// Search files in a sandbox workspace via HTTP proxy.
pub async fn search_sandbox_files(
    service_name: &str,
    namespace: &str,
    port: u16,
    query: &str,
    limit: u32,
) -> Result<Vec<FileSearchResult>, String> {
    let host = format!("{service_name}.{namespace}.svc.cluster.local");
    let url = format!("http://{host}:{port}/search-files");

    let body = serde_json::json!({ "query": query, "limit": limit });
    let body_bytes = serde_json::to_vec(&body).map_err(|e| format!("JSON serialize error: {e}"))?;

    let req = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri(&url)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body_bytes))
        .map_err(|e| format!("Failed to build request: {e}"))?;

    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let client = hyper_util::client::legacy::Client::builder(
            hyper_util::rt::TokioExecutor::new(),
        )
        .build_http::<axum::body::Body>();

        match client.request(req).await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    return Err(format!("Search files returned status {}", resp.status()));
                }
                let body = resp.into_body().collect().await
                    .map_err(|e| format!("Failed to read response: {e}"))?
                    .to_bytes();
                serde_json::from_slice(&body)
                    .map_err(|e| format!("Failed to parse search results: {e}"))
            }
            Err(e) => Err(format!("HTTP request failed: {e}")),
        }
    })
    .await;

    match result {
        Ok(inner) => inner,
        Err(_) => Err("Search files request timed out".to_string()),
    }
}
