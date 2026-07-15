//! Sandbox notification and wake-up helpers.

use super::client::SandboxClient;
use super::models::EnvironmentStatus;
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

/// Best-effort: stop the running agent on `task_id`'s sandbox before teardown.
///
/// Drives the same `StopAgent` gRPC the manual stop handler uses, so the agent
/// cancels its in-flight turn and publishes `agent.stopped` promptly instead of
/// riding the pod's full ~30s termination grace until SIGKILL (exit 137). Every
/// failure is swallowed — `AgentNotRunning` (idle agent), an unreachable/gone
/// pod, and transport errors must never block the teardown that follows. Idle
/// agents make `stop_agent` a no-op, so this adds no spurious stopped message
/// on a routine idle-down.
pub async fn stop_agent_for_task(state: &AppState, task_id: &str) {
    match state.sandbox.sandbox_client(task_id).await {
        Ok(client) => {
            if let Err(e) = client.stop_agent().await {
                tracing::debug!(task_id, error = %e, "stop_agent before teardown failed (continuing)");
            } else {
                tracing::debug!(task_id, "Stopped agent before teardown");
            }
        }
        Err(e) => {
            tracing::debug!(task_id, error = %e, "No sandbox client for stop_agent before teardown (continuing)");
        }
    }
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

    let request = super::auto_provision::build_provision_request(
        state.tasks.as_ref(),
        state.repositories.as_ref(),
        state.provider_tokens.as_ref(),
        state.plugin_repos.as_ref(),
        task_id,
    )
    .await
    .map_err(|e| {
        tracing::warn!(task_id = %task_id, error = %e, "Failed to build provision request for message delivery");
        format!("Failed to build provision request: {e}")
    })?;

    match state.sandbox.provision(task_id, &request).await {
        Ok(resp) if resp.status == EnvironmentStatus::Ready => {
            notify_sandbox_for_task(state, task_id).await;
        }
        Ok(_) => {
            tracing::info!(task_id = %task_id, "Sandbox starting, skipping notification");
        }
        Err(e) => {
            tracing::warn!(task_id = %task_id, error = %e, "Failed to provision sandbox for message delivery");
            return Err(format!("Failed to provision sandbox: {e}"));
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
