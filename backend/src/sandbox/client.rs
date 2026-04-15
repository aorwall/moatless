//! gRPC client wrapper for calling sandbox pods.
//!
//! Replaces all HTTP calls to sandbox pods (health, status, execute-agent,
//! stop-agent, notify-messages, search-files) with gRPC.
//! Server management is handled by K8s (separate containers).

use std::time::Duration;

use sandbox_proto::sandbox_service_client::SandboxServiceClient;
use sandbox_proto::{
    ExecuteAgentRequest, GetAgentStatusRequest, GetAgentStatusResponse, GetFileTreeRequest,
    GetFileTreeResponse, ListFilesRequest, NotifyMessagesRequest, ReadFileRequest,
    ReadFileResponse, SearchFilesByNameRequest, SearchFilesByNameResponse, StopAgentRequest,
    WriteFileRequest,
};
use tonic::transport::Channel;
use tracing::debug;

use super::error::SandboxError;

/// Errors that can occur when constructing a [`SandboxClient`].
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("invalid gRPC endpoint URI: {0}")]
    InvalidUri(#[from] tonic::codegen::http::uri::InvalidUri),
}

/// Classify gRPC errors into appropriate `SandboxError` variants.
///
/// Unavailable/connection errors → `NotReady` (503),
/// everything else → `Internal` (500).
pub fn classify_grpc_error(e: tonic::Status, operation: &str) -> SandboxError {
    match e.code() {
        tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => {
            SandboxError::NotReady(format!("Sandbox unreachable: {operation}"))
        }
        _ => SandboxError::Internal(format!("gRPC {operation} failed: {e}")),
    }
}

/// Wrapper around `SandboxServiceClient` for calling sandbox pods via gRPC.
#[derive(Clone)]
pub struct SandboxClient {
    client: SandboxServiceClient<Channel>,
}

impl SandboxClient {
    /// Create a gRPC client for a sandbox pod using K8s service DNS.
    ///
    /// Uses `connect_lazy()` — no upfront TCP connection.
    pub fn for_service(
        service_name: &str,
        namespace: &str,
        port: u16,
    ) -> Result<Self, ClientError> {
        let url = format!("http://{service_name}.{namespace}.svc.cluster.local:{port}");
        Self::connect(&url)
    }

    /// Create a gRPC client from a URL.
    pub fn connect(url: &str) -> Result<Self, ClientError> {
        let channel = Channel::from_shared(url.to_string())?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(10))
            .connect_lazy();

        Ok(Self {
            client: SandboxServiceClient::new(channel),
        })
    }

    /// Execute agent on the sandbox.
    pub async fn execute_agent(&self, request: ExecuteAgentRequest) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        client.execute_agent(request).await?;
        Ok(())
    }

    /// Stop the running agent.
    pub async fn stop_agent(&self) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        client.stop_agent(StopAgentRequest {}).await?;
        Ok(())
    }

    /// Get the current agent status from the sandbox.
    pub async fn get_agent_status(&self) -> Result<GetAgentStatusResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            client.get_agent_status(GetAgentStatusRequest {}).await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Get agent status gRPC request timed out",
            )),
        }
    }

    /// Notify the sandbox that new messages are available.
    ///
    /// Fire-and-forget: logs warnings on failure, never propagates errors.
    pub async fn notify_messages(&self) {
        let mut client = self.client.clone();
        match client.notify_messages(NotifyMessagesRequest {}).await {
            Ok(_) => {
                debug!("Notified sandbox pod via gRPC");
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to notify sandbox pod via gRPC");
            }
        }
    }

    /// Get the file tree with git change overlay.
    pub async fn get_file_tree(
        &self,
        root_path: &str,
    ) -> Result<GetFileTreeResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            client
                .get_file_tree(GetFileTreeRequest {
                    root_path: root_path.to_string(),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Get file tree gRPC request timed out",
            )),
        }
    }

    /// Read a file from the sandbox workspace.
    pub async fn read_file(
        &self,
        path: &str,
        include_changes: bool,
    ) -> Result<ReadFileResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            client
                .read_file(ReadFileRequest {
                    path: path.to_string(),
                    include_changes,
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Read file gRPC request timed out",
            )),
        }
    }

    /// Write a file in the sandbox workspace.
    pub async fn write_file(&self, path: &str, content: &str) -> Result<(), tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client
                .write_file(WriteFileRequest {
                    path: path.to_string(),
                    content: content.to_string(),
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Write file gRPC request timed out",
            )),
        }
    }

    /// List files in the sandbox workspace.
    pub async fn list_files(
        &self,
        path: &str,
        recursive: bool,
    ) -> Result<Vec<String>, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client
                .list_files(ListFilesRequest {
                    path: path.to_string(),
                    recursive,
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner().files),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "List files gRPC request timed out",
            )),
        }
    }

    /// Search files by name in the sandbox workspace.
    pub async fn search_files_by_name(
        &self,
        query: &str,
        case_sensitive: bool,
        max_results: i32,
    ) -> Result<SearchFilesByNameResponse, tonic::Status> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            client
                .search_files_by_name(SearchFilesByNameRequest {
                    query: query.to_string(),
                    case_sensitive,
                    max_results,
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(tonic::Status::deadline_exceeded(
                "Search files by name gRPC request timed out",
            )),
        }
    }

}
