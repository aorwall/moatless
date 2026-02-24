//! Fire-and-forget notification to sandbox pods/containers via gRPC.

use super::client::SandboxClient;
use crate::config::SandboxProviderType;
use crate::state::AppState;

/// Convenience helper: notify the sandbox for a given task_id.
///
/// Constructs the gRPC URL based on the configured sandbox provider:
/// - **Kubernetes**: Uses K8s service DNS (`{svc}.{ns}.svc.cluster.local:{port}`)
/// - **Docker**: Uses Docker network DNS (`{container_name}:{port}`)
pub async fn notify_sandbox_for_task(state: &AppState, task_id: &str) {
    let grpc_port = state.config.sandbox.grpc_port;

    let url = match state.config.sandbox_provider {
        SandboxProviderType::Kubernetes => {
            let svc_name = super::k8s::naming::generate_service_name(task_id);
            let namespace = &state.config.sandbox.namespace;
            format!("http://{svc_name}.{namespace}.svc.cluster.local:{grpc_port}")
        }
        SandboxProviderType::Docker => {
            let container_name = super::docker::naming::generate_container_name(task_id);
            format!("http://{container_name}:{grpc_port}")
        }
    };

    let client = match SandboxClient::connect(&url) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                task_id,
                error = %e,
                "Failed to create gRPC client for sandbox notification"
            );
            return;
        }
    };

    client.notify_messages().await;
}

/// Notify a sandbox pod that new messages are available (K8s-specific).
///
/// Fire-and-forget: logs warnings on failure, never propagates errors.
pub async fn notify_sandbox(namespace: &str, service_name: &str, grpc_port: u16) {
    let client = match SandboxClient::for_service(service_name, namespace, grpc_port) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                service_name,
                error = %e,
                "Failed to create gRPC client for sandbox notification"
            );
            return;
        }
    };

    client.notify_messages().await;
}
