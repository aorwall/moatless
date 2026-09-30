//! Reaching the sandbox this command runs inside.
//!
//! `moat cmd` and a self-restart both talk to the pod's own `SandboxService`
//! rather than to the backend, because each asks the pod for something only the
//! pod holds: the command registry, and the wake notes.

use std::time::Duration;

use sandbox_proto::sandbox_service_client::SandboxServiceClient;

/// Connect to the sandbox's own gRPC server. It listens on loopback inside this
/// container, so there is nothing to authenticate against.
pub async fn connect(
    port: &str,
) -> Result<SandboxServiceClient<tonic::transport::Channel>, Box<dyn std::error::Error>> {
    let endpoint = format!("http://127.0.0.1:{port}");
    let channel = tonic::transport::Endpoint::new(endpoint.clone())?
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .map_err(|e| format!("cannot reach the sandbox at {endpoint}: {e}"))?;
    Ok(SandboxServiceClient::new(channel))
}
