//! Closing a task and its created-by cascade.
//!
//! A close reads six ports — DAOs, sandbox lifecycle, bindings, retention
//! config, event stream — bundled here since single and cascaded closes thread
//! the same set through every frame. [`TaskCloser::close_for`] is T3's
//! choiceless close; `close_task` drives the cascade choice via
//! [`TaskCloser::close_single`] / [`TaskCloser::close_tree`], and
//! [`TaskCloser::close_with_event`] adds a caller-shaped `Closed` event for
//! provider webhooks (see [`CloseEvent`]).

use std::collections::HashSet;
use std::sync::Arc;

use moatless_types::access_control::ScopeContext;
use moatless_types::task::TaskSubType;

use super::bindings::deactivate_task_bindings;
use super::dao::TaskDao;
use super::error::TaskError;
use super::models::TaskPatch;
use super::models::TaskStatus;
use super::relations::TaskRelationDao;
use super::service::publish_user_stop;
use crate::message::send::MessageSendService;
use crate::task::bindings::TaskBindings;
use crate::task::models::SandboxStopReason;
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use moatless_ports::sandbox::SandboxLifecycle;
use moatless_types::sandbox::ArchiveRetention;
use moatless_types::task::TaskRelationType;

/// The ports a close reads. Bundled because single and cascaded closes thread
/// the same set through every frame; constructed once at the composition root.
pub struct TaskCloser {
    pub(crate) tasks: Arc<dyn TaskDao>,
    pub(crate) task_relations: Arc<dyn TaskRelationDao>,
    pub(crate) config: Arc<Config>,
    pub(crate) sandbox: Arc<dyn SandboxLifecycle>,
    pub(crate) task_bindings: Arc<dyn TaskBindings>,
    pub(crate) events: Option<Arc<dyn EventPublisher>>,
}

/// The event-shaping overrides a caller layers onto a close.
///
/// Provider close paths (GitHub PR / Linear webhooks) publish an *enriched*
/// `Closed` event — a `reason` string the UI renders plus provider metadata,
/// with a fixed actor — while every other close wants the neutral shape
/// ([`CloseEvent::neutral`]). Pass to [`TaskCloser::close_with_event`].
pub struct CloseEvent {
    /// Actor recorded on the `Closed` task event. `None` uses the task's own
    /// `agent_profile` — the neutral default.
    pub actor: Option<String>,
    /// Extra fields merged into the `Closed` task event payload. Empty (`{}`)
    /// in the neutral shape.
    pub payload: serde_json::Value,
}

impl CloseEvent {
    /// The neutral close event: the task's own `agent_profile`, empty payload —
    /// byte for byte what every close published before the enriched entry
    /// existed. Every non-provider close uses this.
    pub fn neutral() -> Self {
        Self {
            actor: None,
            payload: serde_json::json!({}),
        }
    }

    /// An enriched provider close event: `actor` is recorded on the
    /// published `Closed` event, and `payload` is merged into it verbatim.
    ///
    /// This crate never interprets `payload` — the provider owns its shape.
    /// Only `reason` is special: the frontend renders it as the event's
    /// message text (`event-messages.ts`); the rest rides through as-is.
    pub fn enriched(actor: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            actor: Some(actor.into()),
            payload,
        }
    }
}

impl TaskCloser {
    /// Wire the closer from the ports the backend composition root owns.
    pub(crate) fn new(
        tasks: Arc<dyn TaskDao>,
        task_relations: Arc<dyn TaskRelationDao>,
        config: Arc<Config>,
        sandbox: Arc<dyn SandboxLifecycle>,
        task_bindings: Arc<dyn TaskBindings>,
        events: Option<Arc<dyn EventPublisher>>,
    ) -> Self {
        Self {
            tasks,
            task_relations,
            config,
            sandbox,
            task_bindings,
            events,
        }
    }

    /// Close one task, leaving any children open.
    pub(crate) async fn close_single(
        &self,
        task_id: &str,
        event: &CloseEvent,
    ) -> Result<(), TaskError> {
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
            let actor = event.actor.as_deref().unwrap_or(row.agent_profile.as_str());
            events
                .publish_task_event(
                    task_id,
                    TaskSubType::Closed,
                    Some(actor),
                    event.payload.clone(),
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
        let close_order = self.subtree(task_id, visited).await?;

        let neutral = CloseEvent::neutral();
        for current_task_id in close_order.into_iter().rev() {
            self.close_single(&current_task_id, &neutral).await?;
        }

        Ok(())
    }

    /// Close one task on `scope_ctx`'s behalf, leaving any children open.
    ///
    /// The choiceless close T3 compatibility uses; the HTTP route makes the
    /// cascade choice explicit instead.
    pub async fn close_for(
        &self,
        scope_ctx: &ScopeContext,
        task_id: &str,
    ) -> Result<(), TaskError> {
        self.close_with_event(scope_ctx, task_id, &CloseEvent::neutral())
            .await?;
        tracing::info!(task_id, user_id = %scope_ctx.user_id, "Task closed");
        Ok(())
    }

    /// Close one task on `scope_ctx`'s behalf, publishing a caller-shaped
    /// `Closed` event.
    ///
    /// The entry provider webhooks use to carry their own `reason` + metadata
    /// via [`CloseEvent`]; otherwise identical to
    /// [`close_for`](Self::close_for) — the two differ only in event shape.
    pub async fn close_with_event(
        &self,
        scope_ctx: &ScopeContext,
        task_id: &str,
        event: &CloseEvent,
    ) -> Result<(), TaskError> {
        let row = self
            .tasks
            .find_by_id(task_id, scope_ctx)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        if !scope_ctx.can_write(&row.scoped_resource(), Some(row.visibility)) {
            return Err(TaskError::NotFound(task_id.to_string())); // existence hiding
        }

        self.close_single(task_id, event).await
    }
}

impl TaskCloser {
    /// `task_id` and its created-by descendants, each parent before its
    /// children. `visited` guards against a cycle in the relations.
    async fn subtree(
        &self,
        task_id: &str,
        visited: &mut HashSet<String>,
    ) -> Result<Vec<String>, TaskError> {
        let mut stack = vec![task_id.to_string()];
        let mut order = Vec::new();

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
                        "Failed to list child tasks during cascade"
                    );
                    return Err(TaskError::Db(e));
                }
            }

            order.push(current_task_id);
        }

        Ok(order)
    }

    /// Cancel a task's run and its created-by subtree's on `scope_ctx`'s behalf.
    ///
    /// Disposes the subtree's pending Messages before stopping each agent, so
    /// no disposed Message opens a turn. Tasks and sandboxes stay open.
    pub async fn cancel_requested(
        &self,
        scope_ctx: &ScopeContext,
        task_id: &str,
        messages: &MessageSendService,
    ) -> Result<CancelOutcome, TaskError> {
        let row = self
            .tasks
            .find_by_id(task_id, scope_ctx)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        if !scope_ctx.can_write(&row.scoped_resource(), Some(row.visibility)) {
            return Err(TaskError::NotFound(task_id.to_string())); // existence hiding
        }

        let subtree = self.subtree(task_id, &mut HashSet::new()).await?;
        let disposed_messages = messages.dispose_pending(&subtree).await?;

        // reason: the subtree reaches child tasks owned by other users, as in `close_single`.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        let mut stopped_task_ids = Vec::new();
        for current_task_id in subtree {
            match self.tasks.find_by_id(&current_task_id, &sys_ctx).await? {
                Some(task) if task.status != TaskStatus::Closed => {}
                _ => continue,
            }
            self.sandbox
                .stop_agent(&current_task_id, SandboxStopReason::User)
                .await;
            publish_user_stop(self.events.as_deref(), &current_task_id).await;
            stopped_task_ids.push(current_task_id);
        }
        tracing::info!(task_id, user_id = %scope_ctx.user_id, disposed_messages, "Task cancelled");

        Ok(CancelOutcome {
            stopped_task_ids,
            disposed_messages,
        })
    }
}

/// What [`TaskCloser::cancel_requested`] did.
pub struct CancelOutcome {
    /// The open tasks whose agents it stopped, the cancelled one first.
    pub stopped_task_ids: Vec<String>,
    /// How many pending Messages it cancelled.
    pub disposed_messages: u64,
}

impl TaskCloser {
    /// Close a task after validating access and the caller's child-close
    /// choice.
    pub async fn close_requested(
        &self,
        scope_ctx: &ScopeContext,
        task_id: String,
        req: super::schemas::CloseTaskRequest,
    ) -> Result<(), TaskError> {
        let row = self
            .tasks
            .find_by_id(&task_id, scope_ctx)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
        if !scope_ctx.can_write(&row.scoped_resource(), Some(row.visibility)) {
            return Err(TaskError::NotFound(task_id.clone())); // existence hiding
        }

        let children = self
            .task_relations
            .list_incoming(&task_id, Some(TaskRelationType::CreatedBy))
            .await
            .map_err(TaskError::Db)?;
        if !children.is_empty() && req.close_children.is_none() {
            return Err(TaskError::BadRequest(
            "closeChildren must be explicitly set to true or false when closing a task with children"
                .to_string(),
        ));
        }

        if req.close_children == Some(true) {
            let mut visited = HashSet::new();
            self.close_tree(&task_id, &mut visited).await?;
        } else {
            self.close_single(&task_id, &CloseEvent::neutral()).await?;
        }
        tracing::info!(task_id = %task_id, user_id = %scope_ctx.user_id, "Task closed");

        Ok(())
    }
}
