//! gRPC client wrapper for backend communication.
//!
//! Replaces both Redis event publishing and HTTP message polling with
//! a single gRPC connection to the backend's `BackendSandboxService`.
//! Includes an auth interceptor that injects `authorization: Bearer <token>`
//! metadata on every outbound call.

use std::time::Duration;

use sandbox_proto::backend_sandbox_service_client::BackendSandboxServiceClient;
use sandbox_proto::{
    GetCredentialsRequest, GetCredentialsResponse, GetPendingMessagesRequest, PendingMessage,
    PublishEventRequest,
};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;
use tonic::{Request, Status};
use tracing::{debug, warn};

/// Auth interceptor that injects a Bearer token on every outbound gRPC call.
#[derive(Clone)]
pub struct AuthInterceptor {
    token: Option<String>,
}

impl tonic::service::Interceptor for AuthInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        if let Some(ref token) = self.token {
            let header_value = format!("Bearer {token}")
                .parse()
                .map_err(|_| Status::internal("Invalid auth token format"))?;
            request
                .metadata_mut()
                .insert("authorization", header_value);
        }
        Ok(request)
    }
}

/// Wrapper around `BackendSandboxServiceClient` with auth interceptor.
#[derive(Clone)]
pub struct BackendClient {
    client: BackendSandboxServiceClient<InterceptedService<Channel, AuthInterceptor>>,
}

impl BackendClient {
    /// Create a new client with lazy connection to the backend gRPC server.
    ///
    /// `connect_lazy()` returns immediately — the actual TCP/HTTP2 connection
    /// is established on the first RPC call. This lets the sandbox start
    /// before the backend gRPC server is ready.
    ///
    /// The `auth_token` is injected as a Bearer token on every outbound call.
    pub fn connect(
        url: &str,
        auth_token: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let channel = Channel::from_shared(url.to_string())?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(10))
            .connect_lazy();

        let interceptor = AuthInterceptor { token: auth_token };
        let client = BackendSandboxServiceClient::with_interceptor(channel, interceptor);

        Ok(Self { client })
    }

    /// Publish an event to the backend (fire-and-forget).
    ///
    /// Logs warnings on failure, never propagates errors — matches the
    /// existing Redis event publisher behavior.
    pub async fn publish_event(&self, req: PublishEventRequest) {
        let mut client = self.client.clone();
        match client.publish_event(req).await {
            Ok(resp) => {
                debug!(
                    event_id = %resp.into_inner().event_id,
                    "Event published via gRPC"
                );
            }
            Err(e) => {
                warn!(error = %e, "Failed to publish event via gRPC");
            }
        }
    }

    /// Fetch pending messages for a task from the backend.
    ///
    /// Auth is handled by the interceptor — no explicit token parameter needed.
    pub async fn get_pending_messages(
        &self,
        task_id: &str,
    ) -> Result<Vec<PendingMessage>, tonic::Status> {
        let mut client = self.client.clone();
        let resp = client
            .get_pending_messages(GetPendingMessagesRequest {
                task_id: task_id.to_string(),
                auth_token: String::new(), // kept for proto compat, auth via interceptor
            })
            .await?;

        Ok(resp.into_inner().messages)
    }

    /// Fetch credentials for sandbox initialization from the backend.
    pub async fn get_credentials(
        &self,
        task_id: &str,
    ) -> Result<GetCredentialsResponse, tonic::Status> {
        let mut client = self.client.clone();
        let resp = client
            .get_credentials(GetCredentialsRequest {
                task_id: task_id.to_string(),
            })
            .await?;

        Ok(resp.into_inner())
    }
}
