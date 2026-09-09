//! Closing a task and its created-by cascade.
//!
//! A close reads six ports — the task and relation DAOs, the sandbox lifecycle,
//! the binding DAO, the close-retention config, and the event stream — and
//! every close (a single task, or a cascade down the created-by tree) threads
//! the same set through every frame, so they are bundled here rather than
//! passed function to function. [`TaskCloser::close_for`] is the choiceless
//! close the T3 compatibility surface makes; the HTTP `close_task` handler makes
//! the cascade choice explicit and drives [`TaskCloser::close_single`] /
//! [`TaskCloser::close_tree`]. [`TaskCloser::close_with_event`] is the same close
//! with a caller-shaped `Closed` event — the entry the provider webhooks use to
//! carry their own `reason` + metadata (see [`CloseEvent`]).

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
use crate::task::bindings::TaskBindings;
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
/// The provider close paths (GitHub PR / Linear webhooks) publish an *enriched*
/// `Closed` task event — a `reason` string the UI renders (`event-messages.ts`
/// falls back to it as the event's message text) plus provider metadata — and
/// record a fixed actor, while every other close wants the neutral shape. The
/// payload is kept opaque: the caller builds a `serde_json::Value`, so provider
/// specifics never reach this crate. Pass one to [`TaskCloser::close_with_event`];
/// [`CloseEvent::neutral`] is what the plain closes use.
pub struct CloseEvent {
    /// Actor recorded on the `Closed` task event. `None` uses the task's own
    /// `agent_type` — the neutral default.
    pub actor: Option<String>,
    /// Extra fields merged into the `Closed` task event payload. Empty (`{}`) in
    /// the neutral shape.
    pub payload: serde_json::Value,
}

impl CloseEvent {
    /// The neutral close event: the task's own `agent_type`, empty payload — byte
    /// for byte what every close published before the enriched entry existed.
    /// Every non-provider close uses this.
    pub fn neutral() -> Self {
        Self {
            actor: None,
            payload: serde_json::json!({}),
        }
    }

    /// An enriched provider close event: `actor` is recorded on the published
    /// `Closed` event, and `payload` is merged into it verbatim.
    ///
    /// `tasks` does NOT interpret `payload` — the provider owns its shape and
    /// this crate forwards it untouched, so provider specifics never travel down
    /// into band 5. GitHub PR builds `{ "reason": "pr_closed", "pr_number": N }`;
    /// Linear builds `{ "reason": "linear_state_terminal", "issue_identifier":
    /// ..., "state_name": ... }`. The `reason` is what the frontend renders as
    /// the event's message text (`event-messages.ts`); the rest rides wholesale
    /// through the event sinks.
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
            let actor = event.actor.as_deref().unwrap_or(row.agent_type.as_str());
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

        let neutral = CloseEvent::neutral();
        for current_task_id in close_order.into_iter().rev() {
            self.close_single(&current_task_id, &neutral).await?;
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
        self.close_with_event(scope_ctx, task_id, &CloseEvent::neutral())
            .await?;
        tracing::info!(task_id, user_id = %scope_ctx.user_id, "Task closed");
        Ok(())
    }

    /// Close one task on `scope_ctx`'s behalf, publishing a caller-shaped
    /// `Closed` event.
    ///
    /// The enriched entry the provider webhooks use: a GitHub PR / Linear close
    /// carries its own `reason` + metadata and a fixed actor via [`CloseEvent`],
    /// which this forwards verbatim to the event stream (the payload stays opaque
    /// here). Authorization and termination are otherwise identical to
    /// [`close_for`](Self::close_for) — the two differ only in the event shape.
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
        if !scope_ctx.can_write(&row.scoped_resource()) {
            return Err(TaskError::NotFound(task_id.to_string())); // existence hiding
        }

        self.close_single(task_id, event).await
    }
}

impl TaskCloser {
    /// Close a task after validating access and the caller's child-close choice.
    pub async fn close_requested(
        &self,
        scope_ctx: &ScopeContext,
        task_id: String,
        req: super::schemas::CloseTaskRequest,
    ) -> Result<(), TaskError> {
        // Read-check + control-check: find_by_id uses access_filter (visibility);
        // the inline can_write check enforces owner/admin policy without the
        // deleted ensure_can_write_task helper.
        let row = self
            .tasks
            .find_by_id(&task_id, scope_ctx)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.clone()))?;
        if !scope_ctx.can_write(&row.scoped_resource()) {
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
