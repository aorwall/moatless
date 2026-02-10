//! Health check and status fetching for sandbox pods via gRPC.
//!
//! All sandbox communication now uses gRPC instead of HTTP.

use axum::response::sse::Event;
use futures::stream::BoxStream;
use tracing::{debug, warn};

use crate::sandbox::grpc_client::SandboxGrpcClient;
use crate::sandbox::types::{FileSearchResult, SandboxSnapshotStatus};

/// Check if a sandbox pod is healthy via its gRPC health endpoint.
pub async fn check_pod_health(host: &str, port: u16) -> bool {
    let url = format!("http://{host}:{port}");
    let client = match SandboxGrpcClient::connect(&url) {
        Ok(c) => c,
        Err(_) => return false,
    };
    client.check_health().await
}

/// Check health using K8s service DNS via gRPC.
pub async fn check_service_health(service_name: &str, namespace: &str, port: u16) -> bool {
    let client = match SandboxGrpcClient::for_service(service_name, namespace, port) {
        Ok(c) => c,
        Err(_) => return false,
    };
    client.check_health().await
}

/// Fetch the sandbox snapshot status from a running sandbox pod via gRPC.
///
/// Returns `None` if the request fails or times out.
pub async fn fetch_sandbox_status(
    service_name: &str,
    namespace: &str,
    port: u16,
) -> Option<SandboxSnapshotStatus> {
    let client = match SandboxGrpcClient::for_service(service_name, namespace, port) {
        Ok(c) => c,
        Err(e) => {
            warn!(service_name, error = %e, "Failed to create gRPC client for status");
            return None;
        }
    };
    client.get_status().await
}

/// Stream server logs from the sandbox pod via gRPC, converted to SSE events.
pub async fn stream_server_logs(
    service_name: &str,
    namespace: &str,
    port: u16,
    server_name: &str,
) -> Result<BoxStream<'static, Result<Event, crate::sandbox::error::SandboxError>>, String> {
    let client = SandboxGrpcClient::for_service(service_name, namespace, port)
        .map_err(|e| format!("Failed to create gRPC client: {e}"))?;

    let mut stream = client
        .stream_server_logs(server_name, Some(500))
        .await?;

    let event_stream = async_stream::stream! {
        use futures::StreamExt;
        while let Some(result) = stream.next().await {
            match result {
                Ok(log_line) => {
                    yield Ok(Event::default().data(log_line.line));
                }
                Err(e) => {
                    debug!(error = %e, "Server log stream ended");
                    break;
                }
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
