//! gRPC client wrapper for calling sandbox pods.
//!
//! Replaces all HTTP calls to sandbox pods (health, status, execute-agent,
//! stop-agent, notify-messages, search-files) with gRPC.
//! Server management is handled by K8s (separate containers).

use std::time::Duration;

use sandbox_proto::sandbox_service_client::SandboxServiceClient;
use sandbox_proto::{
    ExecuteAgentRequest, NotifyMessagesRequest,
    SearchFilesRequest, StopAgentRequest,
};
use tonic::transport::Channel;
use tracing::debug;

/// Wrapper around `SandboxServiceClient` for calling sandbox pods via gRPC.
#[derive(Clone)]
pub struct SandboxGrpcClient {
    client: SandboxServiceClient<Channel>,
}

impl SandboxGrpcClient {
    /// Create a gRPC client for a sandbox pod using K8s service DNS.
    ///
    /// Uses `connect_lazy()` — no upfront TCP connection.
    pub fn for_service(
        service_name: &str,
        namespace: &str,
        port: u16,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "http://{service_name}.{namespace}.svc.cluster.local:{port}"
        );
        Self::connect(&url)
    }

    /// Create a gRPC client from a URL.
    pub fn connect(url: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
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

    /// Search files in the sandbox workspace.
    pub async fn search_files(
        &self,
        query: &str,
        limit: i32,
    ) -> Result<Vec<String>, String> {
        let mut client = self.client.clone();
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            client
                .search_files(SearchFilesRequest {
                    query: query.to_string(),
                    limit,
                })
                .await
        })
        .await;

        match result {
            Ok(Ok(resp)) => Ok(resp.into_inner().files),
            Ok(Err(e)) => Err(format!("gRPC search files failed: {e}")),
            Err(_) => Err("Search files gRPC request timed out".to_string()),
        }
    }
}
