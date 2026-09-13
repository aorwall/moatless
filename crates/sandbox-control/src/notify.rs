//! Sandbox notification and wake-up helpers.

use std::sync::Arc;

use async_trait::async_trait;
use sandbox_proto::client::{SandboxConnector, classify_grpc_error};

use moatless_sandbox_runtime::operation::ObservedStatus;

use crate::lifecycle::{Lifecycle, LifecycleError, NotifyPort};
use moatless_ports::sandbox::{CredentialBroadcast, SandboxError, SandboxWaker};

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

    client.notify_messages_for_task(Some(task_id)).await;
}

/// Sandbox-control implementation of the [`SandboxWaker`] port.
///
/// Holds the two ports the wake reads — the lifecycle orchestrator (closed-task
/// guard, start) and the notification port — so the wake logic names neither
/// through `AppState`.
pub struct SandboxWakeService {
    sandbox: Arc<Lifecycle>,
    sandbox_notify: Arc<dyn NotifyPort>,
}

impl SandboxWakeService {
    pub fn new(sandbox: Arc<Lifecycle>, sandbox_notify: Arc<dyn NotifyPort>) -> Self {
        Self {
            sandbox,
            sandbox_notify,
        }
    }
}

#[async_trait]
impl SandboxWaker for SandboxWakeService {
    /// Ready sends a gRPC notification; stopped or not-yet-created
    /// provisions/starts it, since it fetches pending messages on startup.
    ///
    /// On failure, logs the error and returns `Err(String)` with a message
    /// callers can choose to persist.
    async fn wake(&self, task_id: &str) -> Result<(), String> {
        // A closed task is terminal — never re-provision its sandbox. This is the
        // single backstop against resurrection from every caller (adapter inbound
        // comments, scheduled runs, the UI message handler). `Error` tasks stay
        // resumable to match `start_task`'s recovery semantics.
        match self.sandbox.intent(task_id).await {
            Ok(intent) if intent.status == moatless_types::task::TaskStatus::Closed => {
                tracing::debug!(
                    task_id,
                    "Task is closed — skipping sandbox provision/notify"
                );
                return Ok(());
            }
            Ok(_) => {}
            Err(e) => {
                // Fail-open: a DB blip must not silently drop a legitimate wake-up.
                tracing::warn!(task_id, error = %e, "Failed to read task status before sandbox provision; proceeding");
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
            match self.sandbox.start(task_id).await {
                Ok(outcome) if outcome.observed == ObservedStatus::Ready => {
                    self.sandbox_notify.notify(task_id).await;
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
                        tracing::info!(
                            task_id,
                            ?kind,
                            retries,
                            "message wake contended with stop/remove past retry budget; left as pending reconciler input"
                        );
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
}

/// Sandbox-control implementation of the [`CredentialBroadcast`] port.
///
/// Owns the connector so the tasks-band fan-out names no gRPC type.
pub struct CredentialBroadcastService {
    connector: Arc<dyn SandboxConnector>,
}

impl CredentialBroadcastService {
    pub fn new(connector: Arc<dyn SandboxConnector>) -> Self {
        Self { connector }
    }
}

#[async_trait]
impl CredentialBroadcast for CredentialBroadcastService {
    async fn refresh_credential(
        &self,
        task_id: &str,
        key: &str,
        value: &str,
    ) -> Result<(), SandboxError> {
        let client = self.connector.sandbox_client(task_id).await?;
        client
            .refresh_credential(task_id, key, value)
            .await
            .map_err(|status| classify_grpc_error(status, "refresh_credential"))
    }
}
