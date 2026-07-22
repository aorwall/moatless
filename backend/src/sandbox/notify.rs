//! Sandbox notification and wake-up helpers.

use super::client::SandboxClient;
use crate::config::SandboxProviderType;
use crate::sandbox::lifecycle::{LifecycleError, ObservedStatus};
use crate::state::AppState;

/// Convenience helper: notify the sandbox for a given task_id.
///
/// Constructs the gRPC URL based on the configured sandbox provider:
/// - **Kubernetes**: Uses K8s service DNS (`{svc}.{ns}.svc.cluster.local:{port}`)
/// - **Docker**: Uses Docker network DNS (`{container_name}:{port}`)
pub async fn notify_sandbox_for_task(state: &AppState, task_id: &str) {
    notify_task_over_grpc(
        state.config.sandbox_provider,
        &state.config.sandbox.namespace,
        state.config.sandbox.grpc_port,
        task_id,
    )
    .await;
}

/// Ping a Task's sandbox over gRPC to fetch its pending messages, from raw config
/// (no `AppState`) so background owners like the reconciler can re-notify a Ready
/// sandbox. Fire-and-forget: logs on failure, never propagates.
pub async fn notify_task_over_grpc(
    provider: SandboxProviderType,
    namespace: &str,
    grpc_port: u16,
    task_id: &str,
) {
    let url = match provider {
        SandboxProviderType::Kubernetes => {
            let svc_name = super::k8s::naming::generate_service_name(task_id);
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

/// Ensure the sandbox for `task_id` is running, then notify it about new messages.
///
/// - If already running (`Ready`): sends a gRPC notification.
/// - If stopped or not yet created: provisions/starts it. The sandbox will
///   fetch pending messages on startup, so no notification is needed.
/// - On failure: logs the error and returns `Err(String)` with a human-readable
///   message so callers can decide whether to persist it.
pub async fn ensure_sandbox_and_notify(state: &AppState, task_id: &str) -> Result<(), String> {
    // A closed task is terminal — never re-provision its sandbox. This is the
    // single backstop against resurrection from every caller (adapter inbound
    // comments, scheduled runs, the UI message handler). `Error` tasks stay
    // resumable to match `start_task`'s recovery semantics.
    match state.tasks.find_by_id_internal(task_id).await {
        Ok(Some(task)) if task.status == crate::task::models::TaskStatus::Closed => {
            tracing::debug!(
                task_id,
                "Task is closed — skipping sandbox provision/notify"
            );
            return Ok(());
        }
        Ok(_) => {}
        Err(e) => {
            // Fail-open: a DB blip must not silently drop a legitimate wake-up.
            tracing::warn!(task_id, error = %e, "Failed to fetch task status before sandbox provision; proceeding");
        }
    }

    match state.sandbox.start(task_id).await {
        Ok(outcome) if outcome.observed == ObservedStatus::Ready => {
            notify_sandbox_for_task(state, task_id).await;
        }
        Ok(_) => {
            tracing::info!(task_id = %task_id, "Sandbox starting, skipping notification");
        }
        Err(LifecycleError::OperationInProgress { kind }) => {
            // A concurrent lifecycle op holds the per-Task lock. The persisted
            // pending message is the reconciler's durable wake input, so this is
            // never surfaced as an agent error: a toward-Running op already
            // covers the wake, and a stop/remove is left for the reconciler.
            if kind.is_toward_running() {
                tracing::debug!(task_id, ?kind, "message wake coincided with an in-flight start; covered");
            } else {
                tracing::info!(task_id, ?kind, "message wake contended with stop/remove; left as pending reconciler input");
            }
        }
        Err(e) => {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to start sandbox for message delivery");
            return Err(e.to_string());
        }
    }

    Ok(())
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
