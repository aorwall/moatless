//! Closing a task and its created-by cascade.
//!
//! A close reads six ports — the task and relation DAOs, the sandbox lifecycle,
//! the binding DAO, the close-retention config, and the event stream — and
//! every close (a single task, or a cascade down the created-by tree) threads
//! the same set through every frame, so they are bundled here rather than
//! passed function to function. [`TaskCloser::close_for`] is the choiceless
//! close the T3 compatibility surface makes; the HTTP `close_task` handler makes
//! the cascade choice explicit and drives [`TaskCloser::close_single`] /
//! [`TaskCloser::close_tree`].

use std::collections::HashSet;
use std::sync::Arc;

use moatless_backend_core::scope::ScopeContext;
use moatless_events::TaskSubType;

use super::dao::{TaskDao, TaskPatch};
use super::error::TaskError;
use super::handlers::{deactivate_task_bindings, task_resource};
use super::models::TaskStatus;
use super::relations::{TaskRelationDao, TaskRelationType};
use crate::bindings::dao::TaskBindingDao;
use crate::config::Config;
use crate::events::EventPublisher;
use crate::sandbox::lifecycle::{ArchiveRetention, Lifecycle};

/// The ports a close reads. Bundled because single and cascaded closes thread
/// the same set through every frame; constructed once at the composition root.
pub struct TaskCloser {
    pub(crate) tasks: Arc<dyn TaskDao>,
    pub(crate) task_relations: Arc<dyn TaskRelationDao>,
    pub(crate) config: Arc<Config>,
    pub(crate) sandbox: Arc<Lifecycle>,
    pub(crate) task_bindings: Arc<dyn TaskBindingDao>,
    pub(crate) events: Option<Arc<dyn EventPublisher>>,
}

impl TaskCloser {
    /// Close one task, leaving any children open.
    pub(crate) async fn close_single(&self, task_id: &str) -> Result<(), TaskError> {
        // reason: close cascade reaches child tasks owned by other users; system ctx
        // is required to traverse the full tree without re-checking per-child ownership.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();

        let Some(row) = self.tasks.find_by_id(task_id, &sys_ctx).await? else {
            tracing::warn!(task_id, "close cascade: task vanished mid-close");
            return Ok(());
        };

        if row.status == TaskStatus::Closed {
            return Ok(());
        }

        // Persist `Closed` before removing the sandbox — the Closed guard permits
        // remove, and lifecycle owns the sandbox intent plus the graceful agent stop.
        self.tasks
            .update_task(
                task_id,
                &TaskPatch {
                    status: Some(TaskStatus::Closed),
                    ..Default::default()
                },
                &sys_ctx,
            )
            .await?;

        // Remove the sandbox (best-effort on close). Preserve S3 archives when
        // configured so the task can later be revived via reopen; a failure leaves
        // the `Removed` intent for the reconciler.
        let retention = ArchiveRetention::on_close(self.config.sandbox.retain_storage_on_close);
        if let Err(e) = self.sandbox.remove(task_id, retention).await {
            tracing::warn!(task_id, error = %e, "Failed to remove sandbox on close");
        }

        // A closed task is terminal — stop its external listeners so inbound
        // comments don't resurrect it.
        deactivate_task_bindings(self.task_bindings.as_ref(), task_id).await;

        if let Some(events) = self.events.as_deref() {
            events
                .publish_task_event(
                    task_id,
                    TaskSubType::Closed,
                    Some(row.agent_type.as_str()),
                    serde_json::json!({}),
                )
                .await;
        }

        Ok(())
    }

    /// Close a task and every task in its created-by subtree, leaves first.
    pub(crate) async fn close_tree(
        &self,
        task_id: &str,
        visited: &mut HashSet<String>,
    ) -> Result<(), TaskError> {
        let mut stack = vec![task_id.to_string()];
        let mut close_order = Vec::new();

        while let Some(current_task_id) = stack.pop() {
            if !visited.insert(current_task_id.clone()) {
                continue;
            }

            match self
                .task_relations
                .list_incoming(&current_task_id, Some(TaskRelationType::CreatedBy))
                .await
            {
                Ok(relations) => {
                    for relation in relations {
                        stack.push(relation.from_task_id);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        task_id = %current_task_id,
                        error = %e,
                        "Failed to list child tasks during close cascade"
                    );
                    return Err(TaskError::Db(e));
                }
            }

            close_order.push(current_task_id);
        }

        for current_task_id in close_order.into_iter().rev() {
            self.close_single(&current_task_id).await?;
        }

        Ok(())
    }

    /// Close one task on `scope_ctx`'s behalf, leaving any children open.
    ///
    /// The close a caller with no way to ask about children makes — the T3
    /// compatibility surface, where archiving a thread is this. The HTTP route
    /// makes that choice explicit instead, and cascades when it is `true`.
    pub async fn close_for(
        &self,
        scope_ctx: &ScopeContext,
        task_id: &str,
    ) -> Result<(), TaskError> {
        let row = self
            .tasks
            .find_by_id(task_id, scope_ctx)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        if !scope_ctx.can_control(&task_resource(&row)) {
            return Err(TaskError::NotFound(task_id.to_string())); // existence hiding
        }

        self.close_single(task_id).await?;
        tracing::info!(task_id, user_id = %scope_ctx.user_id, "Task closed");
        Ok(())
    }
}
