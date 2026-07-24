//! Sandbox notification and wake-up helpers.

use super::access::SandboxConnector;
use crate::sandbox::lifecycle::{LifecycleError, ObservedStatus};
use crate::state::AppState;

/// Convenience helper: notify the sandbox for a given task_id.
///
pub async fn notify_sandbox_for_task(state: &AppState, task_id: &str) {
    state.sandbox_notify.notify(task_id).await;
}

/// Ping a Task's sandbox through the connector so background owners never
/// construct service names or raw gRPC URLs.
pub async fn notify_task_over_grpc(connector: &dyn SandboxConnector, task_id: &str) {
    let client = match connector.sandbox_client(task_id).await {
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

    // A concurrent lifecycle op holds the per-Task lock. Inline handling of
    // contention is a latency fast path only — the persisted pending message is
    // the reconciler's durable wake input, never an agent error. A toward-Running
    // op already covers the wake and stops inline work; a stop/remove gets a
    // bounded inline retry so a fresh message lands with low latency once that op
    // releases the lock, then defers to the reconciler.
    const MAX_WAKE_RETRIES: u32 = 3;
    const WAKE_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(300);

    let mut retries = 0u32;
    loop {
        match state.sandbox.start(task_id).await {
            Ok(outcome) if outcome.observed == ObservedStatus::Ready => {
                notify_sandbox_for_task(state, task_id).await;
                return Ok(());
            }
            Ok(_) => {
                tracing::info!(task_id = %task_id, "Sandbox starting, skipping notification");
                return Ok(());
            }
            Err(LifecycleError::OperationInProgress { kind }) if kind.is_toward_running() => {
                tracing::debug!(
                    task_id,
                    ?kind,
                    "message wake coincided with an in-flight start; covered"
                );
                return Ok(());
            }
            Err(LifecycleError::OperationInProgress { kind }) => {
                if retries >= MAX_WAKE_RETRIES {
                    tracing::info!(task_id, ?kind, retries, "message wake contended with stop/remove past retry budget; left as pending reconciler input");
                    return Ok(());
                }
                retries += 1;
                tracing::debug!(
                    task_id,
                    ?kind,
                    retries,
                    "message wake contended with stop/remove; retrying inline"
                );
                tokio::time::sleep(WAKE_RETRY_BACKOFF).await;
            }
            Err(e) => {
                tracing::warn!(task_id = %task_id, error = %e, "Failed to start sandbox for message delivery");
                return Err(e.to_string());
            }
        }
    }
}
