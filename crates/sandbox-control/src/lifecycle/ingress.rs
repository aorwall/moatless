//! The `sandboxes`-row seam the Sandbox's own gRPC service reads and writes.

use std::sync::Arc;

use moatless_types::store::StoreError;

use super::dao::LifecycleDao;
use super::orchestrator::{OutstandingCommand, TaskIntent};

/// What the Sandbox's gRPC service reads off its own row, beside the intent
/// [`super::Lifecycle`] owns.
///
/// The pod reports its running-command count and asks which Workspace/run-as
/// identity its credentials are minted against; neither takes the per-Task
/// lock, since neither is a lifecycle transition.
pub struct SandboxIngress {
    store: Arc<dyn LifecycleDao>,
}

impl SandboxIngress {
    pub(crate) fn new(store: Arc<dyn LifecycleDao>) -> Self {
        Self { store }
    }

    /// The Task's persisted lifecycle intent, absent when the Task has no row.
    pub async fn intent(&self, task_id: &str) -> Result<Option<TaskIntent>, StoreError> {
        self.store.load(task_id).await
    }

    /// The Workspace whose placements the Sandbox checks out, absent when the
    /// Sandbox has no row.
    pub async fn workspace_id(&self, sandbox_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .store
            .sandbox_shape(sandbox_id)
            .await?
            .map(|shape| shape.workspace_id))
    }

    /// The identity the Sandbox's pod acts as outward, absent when the Sandbox
    /// has no row or pins no run-as identity.
    pub async fn run_as_user_id(&self, sandbox_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .store
            .sandbox_shape(sandbox_id)
            .await?
            .and_then(|shape| shape.run_as_user_id))
    }

    /// Record how many `moat cmd` commands the Sandbox is running, as the pod
    /// last reported. See [`LifecycleDao::record_running_commands`].
    pub async fn record_running_commands(
        &self,
        task_id: &str,
        running: i32,
    ) -> Result<(), StoreError> {
        self.store.record_running_commands(task_id, running).await
    }

    /// Mirror the commands the pod says still owe its agent an outcome. See
    /// [`LifecycleDao::record_unreported_commands`].
    pub async fn record_unreported_commands(
        &self,
        task_id: &str,
        commands: &[OutstandingCommand],
        revision: i64,
    ) -> Result<(), StoreError> {
        self.store
            .record_unreported_commands(task_id, commands, revision)
            .await
    }

    /// Read the stored set and clear it. See
    /// [`LifecycleDao::take_unreported_commands`].
    pub async fn take_unreported_commands(
        &self,
        task_id: &str,
    ) -> Result<Vec<OutstandingCommand>, StoreError> {
        self.store.take_unreported_commands(task_id).await
    }
}
