//! The `sandboxes`-row seam the Sandbox's own gRPC service reads and writes.

use std::sync::Arc;

use crate::commit_author::Attribution;
use moatless_types::store::StoreError;

use super::dao::LifecycleDao;
use super::orchestrator::{OutstandingCommand, TaskIntent};

/// The identities a Sandbox row names, which `secrets::resolve_task_actor`
/// takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxIdentity {
    /// The run-as column as stored, owner fallback included: what
    /// auto-provision picked the pod's plugins for.
    pub run_as_user_id: Option<String>,
    /// See [`super::dao::SandboxShape::pinned_run_as`].
    pub pinned_run_as: Option<String>,
    /// The driver, else the owner: the person the Sandbox acts for.
    pub acting_user_id: Option<String>,
    /// See [`super::dao::SandboxShape::driver_user_id`].
    pub driver_user_id: Option<String>,
    pub owner_user_id: Option<String>,
}

impl SandboxIdentity {
    /// Who the Sandbox's outward writes credit, given its Task's Reporter.
    pub fn attribution(&self, reporter: Option<String>) -> Attribution {
        Attribution {
            driver: self.driver_user_id.clone(),
            reporter,
            owner: self.owner_user_id.clone(),
        }
    }
}

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

    /// Who the Sandbox's credentials resolve for, absent when it has no row.
    pub async fn identity(&self, sandbox_id: &str) -> Result<Option<SandboxIdentity>, StoreError> {
        Ok(self
            .store
            .sandbox_shape(sandbox_id)
            .await?
            .map(|shape| SandboxIdentity {
                run_as_user_id: shape.run_as_user_id.clone(),
                pinned_run_as: shape.pinned_run_as().map(str::to_string),
                acting_user_id: shape.acting_user_id().map(str::to_string),
                driver_user_id: shape.driver_user_id.clone(),
                owner_user_id: shape.owner_user_id.clone(),
            }))
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
