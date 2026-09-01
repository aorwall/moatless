//! Sandbox lifecycle status and commands for the T3 compatibility surface.
//!
//! This module owns only environment lifecycle answers: whether a Task's
//! Sandbox is available, and start/stop requests. Preview server declarations
//! and server logs live in `servers.rs`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::warn;

use crate::data::{T3TaskDao, TaskView};
use moatless_types::sandbox::SandboxStopReason;
use moatless_backend_core::sandbox::{
    ClusterDiagnostics, SandboxError, SandboxIntentReader, SandboxObserver, SandboxStarter,
    SandboxStopper,
};
use moatless_types::access_control::ScopeContext;

/// How often a `sandbox.subscribeStatus` subscriber re-reads lifecycle state.
/// Matched to the server-list poller so the two panels move together.
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Everything the sandbox lifecycle handlers need.
#[derive(Clone)]
pub struct SandboxState {
    pub observer: Arc<dyn SandboxObserver>,
    pub tasks: Arc<dyn T3TaskDao>,
    pub intent: Arc<dyn SandboxIntentReader>,
    pub starter: Arc<dyn SandboxStarter>,
    pub stopper: Arc<dyn SandboxStopper>,
    /// Fleet-wide status and live resource usage. Deployment-scoped rather than
    /// per-Task, so it takes no `tasks` authorization: the caller's environment
    /// scope is the whole of it.
    pub diagnostics: Arc<dyn ClusterDiagnostics>,
}

/// Why a sandbox lifecycle request could not be answered.
#[derive(Debug, thiserror::Error)]
pub enum SandboxRpcError {
    /// The viewer cannot read this Task, or it does not exist.
    #[error("conversation not found")]
    NotFound,
    #[error("the sandbox could not be read")]
    Sandbox(#[source] SandboxError),
    #[error("the task could not be read")]
    Database(#[source] sqlx::Error),
}

/// The Task, if this viewer may be told it exists. The read gate, for the
/// status calls.
async fn readable_task(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<TaskView, SandboxRpcError> {
    state
        .tasks
        .find_by_id(task_id, viewer)
        .await
        .map_err(SandboxRpcError::Database)?
        .ok_or(SandboxRpcError::NotFound)
}

/// The Task, if this viewer may operate it. The gate on start and stop, which
/// move a Sandbox rather than report on one.
async fn controlled_task(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<TaskView, SandboxRpcError> {
    state
        .tasks
        .controlled_by_id(task_id, viewer)
        .await
        .map_err(SandboxRpcError::Database)?
        .ok_or(SandboxRpcError::NotFound)
}

/// A Task's sandbox lifecycle status only, without declared server rows.
pub async fn status(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let task = readable_task(state, viewer, task_id).await?;
    let desired = state
        .intent
        .desired_state(task_id)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    let status = state
        .observer
        .live_status(task_id, task.status, desired)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    Ok(json!({
        "sandboxStatus": status.sandbox_status.as_str(),
    }))
}

/// Push a Task's sandbox status for as long as the subscription lives.
///
/// Lifecycle has no event to hang this on — a Sandbox reaches Ready because a
/// pod did, not because anything here was told — so the push is a poll the
/// server runs once per subscriber instead of every client running its own.
/// Only changes are sent: an idle subscription is silent, so a client reads the
/// absence of a message as "nothing moved" rather than "nothing is known".
///
/// `seeded` is the status the subscribe arm already sent. It starts the change
/// detector, because a subscriber that has been told "ready" and is then told
/// "ready" again a moment later has been sent a change that did not happen.
pub async fn watch_status(
    request_id: crate::RequestId,
    state: SandboxState,
    viewer: ScopeContext,
    task_id: String,
    seeded: Value,
    frames: mpsc::Sender<Value>,
) {
    let mut last = Some(seeded);
    loop {
        tokio::time::sleep(STATUS_POLL_INTERVAL).await;
        let snapshot = match status(&state, &viewer, &task_id).await {
            Ok(snapshot) => snapshot,
            Err(SandboxRpcError::NotFound) => {
                // The conversation stopped being readable. No poll recovers
                // from that, and staying silent would leave the indicator
                // showing its last answer as current.
                let _ = frames
                    .send(crate::failure_exit(&request_id, "Conversation not found"))
                    .await;
                return;
            }
            Err(error) => {
                warn!(%error, task_id, "Failed to read a Task's sandbox status");
                continue;
            }
        };
        if last.as_ref() == Some(&snapshot) {
            continue;
        }
        last = Some(snapshot.clone());
        let chunk = json!({
            "_tag": "Chunk",
            "requestId": request_id.wire(),
            "values": [snapshot],
        });
        if frames.send(chunk).await.is_err() {
            return;
        }
    }
}

/// Request that a Task's Sandbox move toward Running, then return lifecycle
/// status only.
pub async fn start(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let _task = controlled_task(state, viewer, task_id).await?;
    state
        .starter
        .start(task_id)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    status(state, viewer, task_id).await
}

/// Every Sandbox in the deployment, with its status and live resource usage.
///
/// Deployment-global rather than per-Task: there is no `threadId` to authorize
/// against, so reaching this at all is the authorization, and the read is
/// already scoped to the caller's environment by the RPC dispatch.
pub async fn list_diagnostics(state: &SandboxState) -> Result<Value, SandboxRpcError> {
    let entries = state
        .diagnostics
        .list_sandbox_diagnostics()
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    Ok(json!({ "sandboxes": entries }))
}

/// Request that a Task's Sandbox move toward Stopped, then return lifecycle
/// status only.
pub async fn stop(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let _task = controlled_task(state, viewer, task_id).await?;
    state
        .stopper
        .stop(task_id, SandboxStopReason::User)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    status(state, viewer, task_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_trait::async_trait;
    use moatless_backend_core::sandbox::{
        LiveStatus, LogOptions, SandboxDesiredState, SandboxStatus, ContractSandboxStatus,
        ServerLogStream,
    };
    use moatless_types::task::TaskStatus;

    use crate::data::{MockT3TaskDao, TaskView};

    /// Reports `Initializing` until `moved` is set, then `Ready`.
    struct MovingStatus {
        moved: AtomicBool,
    }

    #[async_trait]
    impl SandboxIntentReader for MovingStatus {
        async fn desired_state(&self, _task_id: &str) -> Result<SandboxDesiredState, SandboxError> {
            Ok(SandboxDesiredState::Running)
        }
    }

    #[async_trait]
    impl SandboxObserver for MovingStatus {
        async fn sandbox_status(
            &self,
            _task_id: &str,
            _task_status: TaskStatus,
            _desired_state: SandboxDesiredState,
        ) -> Result<ContractSandboxStatus, SandboxError> {
            unreachable!("the lifecycle read uses live_status")
        }

        async fn live_status(
            &self,
            _task_id: &str,
            _task_status: TaskStatus,
            _desired_state: SandboxDesiredState,
        ) -> Result<LiveStatus, SandboxError> {
            Ok(LiveStatus {
                sandbox_status: if self.moved.load(Ordering::SeqCst) {
                    SandboxStatus::Ready
                } else {
                    SandboxStatus::Initializing
                },
                agent_status: None,
                sandbox_error: None,
            })
        }

        async fn server_logs(
            &self,
            _task_id: &str,
            _server_name: &str,
            _options: LogOptions,
        ) -> Result<ServerLogStream, SandboxError> {
            unreachable!("the lifecycle read reads no logs")
        }
    }

    fn watched_state(observer: Arc<MovingStatus>) -> SandboxState {
        let mut tasks = MockT3TaskDao::new();
        tasks.expect_find_by_id().returning(|_, _| {
            Ok(Some(TaskView {
                status: TaskStatus::Open,
            }))
        });
        SandboxState {
            observer: observer.clone(),
            tasks: Arc::new(tasks),
            intent: observer,
            starter: Arc::new(UnreachableStarter),
            stopper: Arc::new(UnreachableStarter),
            diagnostics: Arc::new(UnreachableStarter),
        }
    }

    struct UnreachableStarter;

    #[async_trait]
    impl SandboxStarter for UnreachableStarter {
        async fn start(&self, _task_id: &str) -> Result<(), SandboxError> {
            unreachable!("watching a status starts nothing")
        }
    }

    #[async_trait]
    impl SandboxStopper for UnreachableStarter {
        async fn stop(
            &self,
            _task_id: &str,
            _reason: SandboxStopReason,
        ) -> Result<(), SandboxError> {
            unreachable!("watching a status stops nothing")
        }
    }

    #[async_trait]
    impl ClusterDiagnostics for UnreachableStarter {
        async fn list_sandbox_diagnostics(
            &self,
        ) -> Result<Vec<moatless_backend_core::sandbox::SandboxDiagnosticsEntry>, SandboxError>
        {
            unreachable!("watching a status reads no fleet")
        }
    }

    fn viewer() -> ScopeContext {
        ScopeContext {
            user_id: "viewer".to_string(),
            is_admin: false,
        }
    }

    /// The seed is a value the client already has, so the first poll must not
    /// repeat it. Starting the change detector empty would send every idle
    /// subscriber a duplicate one interval in, and "silent unless something
    /// moved" would be false for exactly the case it is meant to cover.
    #[tokio::test(start_paused = true)]
    async fn a_status_that_has_not_moved_since_the_seed_sends_nothing() {
        let observer = Arc::new(MovingStatus {
            moved: AtomicBool::new(false),
        });
        let (frames, mut received) = mpsc::channel(8);
        let request_id: crate::RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
        let watcher = tokio::spawn(watch_status(
            request_id,
            watched_state(observer.clone()),
            viewer(),
            "task-1".to_string(),
            json!({ "sandboxStatus": "initializing" }),
            frames,
        ));

        tokio::time::sleep(STATUS_POLL_INTERVAL * 3).await;
        assert!(
            received.try_recv().is_err(),
            "an unchanged status is not news"
        );

        observer.moved.store(true, Ordering::SeqCst);
        tokio::time::sleep(STATUS_POLL_INTERVAL * 2).await;
        let frame = received.try_recv().expect("the move is news");
        assert_eq!(frame["values"][0]["sandboxStatus"], "ready");

        watcher.abort();
    }
}
