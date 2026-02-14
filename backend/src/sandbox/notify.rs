//! Fire-and-forget notification to sandbox pods via gRPC.

use super::grpc_client::SandboxGrpcClient;
use super::k8s::naming::generate_service_name;
use crate::state::AppState;

/// Convenience helper: notify the sandbox for a given task_id.
///
/// Derives the K8s service name and namespace from AppState + task_id.
pub async fn notify_sandbox_for_task(state: &AppState, task_id: &str) {
    let svc_name = generate_service_name(task_id);
    notify_sandbox(&state.config.sandbox.namespace, &svc_name, state.config.sandbox.grpc_port).await;
}

/// Notify a sandbox pod that new messages are available.
///
/// Fire-and-forget: logs warnings on failure, never propagates errors.
pub async fn notify_sandbox(namespace: &str, service_name: &str, grpc_port: u16) {
    let client = match SandboxGrpcClient::for_service(service_name, namespace, grpc_port) {
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
