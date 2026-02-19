//! Health check and file search for Docker sandbox containers.
//!
//! Provides gRPC-based file search for sandbox containers.

use crate::sandbox::grpc_client::SandboxGrpcClient;
use crate::sandbox::schemas::FileSearchResult;

/// Search files in a sandbox workspace via gRPC (provider-agnostic).
pub async fn search_sandbox_files_docker(
    container_name: &str,
    grpc_port: u16,
    query: &str,
    limit: u32,
) -> Result<Vec<FileSearchResult>, String> {
    let url = format!("http://{container_name}:{grpc_port}");
    let client = SandboxGrpcClient::connect(&url)
        .map_err(|e| format!("Failed to create gRPC client: {e}"))?;

    let files = client
        .search_files(query, limit as i32)
        .await
        .map_err(|e| format!("gRPC search files failed: {e}"))?;

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
