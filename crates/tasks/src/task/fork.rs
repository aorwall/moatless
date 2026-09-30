//! Forking a Task: where the cut lands, and what the pod needs to reproduce it.
//!
//! A fork is a Task branched off another at a completed turn, whose
//! conversation is projected from its ancestors' sources up to it (4.9a); its
//! first turn continues the Trajectory from the cut `task_forks` names.
//!
//! The session fork itself happens in the pod: Codex's `thread/fork` is an
//! app-server call only a pod can make, and a same-Sandbox fork copies no
//! archive.

use std::sync::Arc;

use moatless_types::access_control::{ScopeContext, ScopedResource};

use super::dao::TaskDao;
use super::error::TaskError;
use crate::message::dao::MessageDao;
use crate::tag::dao::TagDao;
use crate::task_turn::dao::TaskTurnDao;
use crate::task_turn::models::ForkPoint;
use crate::trajectory::operations::Trajectories;

#[cfg(test)]
pub(crate) use super::fork_dao::MockTaskForkDao;
pub(crate) use super::fork_dao::TaskForkDao;

use super::models::TaskFork;

/// The `task_forks` table, as the delivery path reads it. A pod asks for the
/// anchors of the Task it runs; nothing outside this crate writes a row.
pub struct TaskForks {
    store: Arc<dyn TaskForkDao>,
}

impl TaskForks {
    pub(crate) fn new(store: Arc<dyn TaskForkDao>) -> Self {
        Self { store }
    }

    /// See [`TaskForkDao::find`].
    pub async fn find(&self, task_id: &str) -> Result<Option<TaskFork>, sqlx::Error> {
        self.store.find(task_id).await
    }

    /// See [`TaskForkDao::pending`].
    pub async fn pending(&self, task_id: &str) -> Result<Option<TaskFork>, sqlx::Error> {
        self.store.pending(task_id).await
    }
}

/// A resolved fork: which Task, cut where, and what the pod will need. Produced
/// by [`TaskForkService::plan`] before anything is created, so every refusal
/// happens with nothing to undo.
#[derive(Debug, Clone)]
pub struct ForkPlan {
    /// The Task being forked.
    pub source_task_id: String,
    /// The turn the cut keeps. `None` when the parent has no turn at all.
    pub at_turn: Option<i32>,
    /// The cut as a trajectory event; `None` when the parent recorded nothing
    /// at or before it, and the fork starts empty on a fresh session.
    pub source_event_id: Option<uuid::Uuid>,
}

/// The refusal for forking or rewinding-by-fork a Task with no trajectory.
pub(crate) const FROZEN_FORK: &str = "This Task was created before recording and cannot be forked";

/// The in-flight turn a cut must not reach, if any. A cut strictly below it is
/// allowed; reaching it is refused —
/// its transcript ends on an unanswered call.
fn blocking_open_turn(point: &ForkPoint) -> Option<i32> {
    let open = point.open_turn?;
    match point.turn {
        Some(cut) if cut < open => None,
        _ => Some(open),
    }
}

/// Resolving a fork's cut point, and copying the parent's conversation onto it.
/// Creation is not here: a fork goes through the same `normalize` →
/// `TaskCreationService::create` pair as any Task.
pub struct TaskForkService {
    tasks: Arc<dyn TaskDao>,
    turns: Arc<dyn TaskTurnDao>,
    messages: Arc<dyn MessageDao>,
    tags: Arc<dyn TagDao>,
    trajectories: Arc<Trajectories>,
}

impl TaskForkService {
    pub(crate) fn new(
        tasks: Arc<dyn TaskDao>,
        turns: Arc<dyn TaskTurnDao>,
        messages: Arc<dyn MessageDao>,
        tags: Arc<dyn TagDao>,
        trajectories: Arc<Trajectories>,
    ) -> Self {
        Self {
            tasks,
            turns,
            messages,
            tags,
            trajectories,
        }
    }

    /// The source Task's tag ids, for the fork to be created with. Best-effort:
    /// a fork that lands without its parent's tags is still the fork the caller
    /// asked for, and tags are re-settable from the UI.
    pub async fn source_tag_ids(&self, source_task_id: &str) -> Vec<String> {
        match self.tags.get_tags_for_task(source_task_id).await {
            Ok(tags) => tags.into_iter().map(|tag| tag.tag_id).collect(),
            Err(e) => {
                tracing::warn!(
                    task_id = %source_task_id,
                    error = %e,
                    "Failed to read source task tags for fork; forking untagged"
                );
                Vec::new()
            }
        }
    }

    /// Resolve where a fork of `source_task_id` cuts, refusing before anything
    /// is created. `at_turn` is the caller's selector; omitting it cuts after
    /// the parent's newest turn that reached a terminal status.
    pub async fn plan(
        &self,
        scope_ctx: &ScopeContext,
        source_task_id: &str,
        at_turn: Option<i32>,
    ) -> Result<(super::models::Task, ForkPlan), TaskError> {
        let source = self
            .tasks
            .find_by_id(source_task_id, scope_ctx)
            .await?
            .ok_or_else(|| TaskError::NotFound(source_task_id.to_string()))?;
        if !scope_ctx.can_read(&ScopedResource::borrowed(
            source.scope,
            source
                .owner_user_id
                .as_deref()
                .or(source.created_by.as_deref()),
        )) {
            // Existence hiding, as everywhere else: a Task the caller may not
            // read is reported the same way as one that is not there.
            return Err(TaskError::NotFound(source_task_id.to_string()));
        }

        let point = self.turns.fork_point(source_task_id, at_turn).await?;
        if at_turn.is_some() && point.turn.is_none() {
            return Err(TaskError::NotFound(format!(
                "task {source_task_id} has no turn {}",
                at_turn.unwrap_or_default()
            )));
        }
        if let Some(open) = blocking_open_turn(&point) {
            return Err(TaskError::Conflict(format!(
                "task {source_task_id} has turn {open} in flight; fork at a turn before it, or wait for it to finish"
            )));
        }

        let cut = match point.turn {
            Some(turn) => {
                self.trajectories
                    .resolve_cut(source_task_id, Some(turn))
                    .await?
            }
            None => None,
        };
        // A frozen conversation with no recorded cut has nothing to inherit
        // from but the rows the freeze copied, which a fork cannot project.
        if cut.is_none() && self.messages.is_frozen(source_task_id).await? {
            return Err(TaskError::Conflict(FROZEN_FORK.to_string()));
        }

        Ok((
            source,
            ForkPlan {
                source_task_id: source_task_id.to_string(),
                at_turn: point.turn,
                source_event_id: cut.map(|cut| cut.event_id),
            },
        ))
    }

    /// Project the fork's inherited conversation onto its id. Runs before the
    /// fork's Task row exists, which puts the inherited Messages behind the
    /// fork's own first message rather than in front of it.
    pub async fn inherit_conversation(
        &self,
        plan: &ForkPlan,
        fork_task_id: &str,
    ) -> Result<(), TaskError> {
        let Some(cut) = plan.source_event_id else {
            return Ok(());
        };
        Ok(self
            .messages
            .inherit_conversation(fork_task_id, &plan.source_task_id, cut)
            .await?)
    }

    /// Undo [`Self::inherit_conversation`] when the following create fails, so
    /// no rows survive behind a Task that never existed. Best-effort and
    /// logged, not propagated — call only on the failure path.
    pub async fn discard_conversation(&self, fork_task_id: &str) {
        if let Err(error) = self.messages.discard_conversation(fork_task_id).await {
            tracing::warn!(
                task_id = %fork_task_id,
                %error,
                "Failed to discard a forked conversation after the fork's create failed; \
                 the inherited rows are orphaned until reaped"
            );
        }
    }
}

/// Dependencies shared by normalization and creation of a fork.
pub struct ForkCreationPorts<'a> {
    pub normalize: super::service::CreateTaskPorts<'a>,
    pub task_creation: &'a super::creation::TaskCreationService,
    pub message_send: std::sync::Arc<crate::message::send::MessageSendService>,
}

impl TaskForkService {
    /// Inherit the selected conversation and compensate every failed task
    /// creation.
    pub async fn fork<EvFut>(
        &self,
        ports: ForkCreationPorts<'_>,
        scope_ctx: &ScopeContext,
        task_id: &str,
        req: super::schemas::ForkTaskRequest,
        build_event_payload: impl FnOnce(super::models::Task) -> EvFut + Send,
    ) -> Result<super::creation::TaskCreationOutcome, TaskError>
    where
        EvFut: std::future::Future<Output = serde_json::Value> + Send,
    {
        use super::creation::ForkedFrom;
        use super::schemas::CreateTaskRequest;
        use super::service;
        let same_sandbox = req.same_sandbox.unwrap_or(false);
        let user_id = scope_ctx.user_id.clone();
        let (source, plan) = self.plan(scope_ctx, task_id, req.at_turn).await?;

        // The fork's id is minted here rather than by the creation service: the
        // conversation is projected onto it before the Task row exists, which is
        // what puts the inherited Messages behind the fork's own first message.
        let fork_task_id = uuid::Uuid::new_v4().to_string();
        self.inherit_conversation(&plan, &fork_task_id).await?;

        // Everything the fork runs against comes from the Task it branched off. A
        // fork is deliberately created unnamed and undescribed: "Fork of X" says
        // where it came from, not what it is doing, and the agent sets both with
        // `moat set-name` once it knows.
        let create = CreateTaskRequest {
            repository_id: Some(source.repository_id.clone()),
            workspace_id: source.workspace_id.clone(),
            sandbox_id: same_sandbox.then(|| source.sandbox_id.clone()).flatten(),
            checkout_mode: same_sandbox.then_some(moatless_types::task::CheckoutMode::Shared),
            agent_profile: Some(
                req.agent_profile
                    .clone()
                    .unwrap_or_else(|| source.agent_profile.to_string()),
            ),
            name: None,
            description: None,
            branch: source.branch.clone(),
            message: req.message.clone(),
            // The source's model belongs to the source's profile.
            model: req.model.clone().or_else(|| {
                req.agent_profile
                    .is_none()
                    .then(|| source.model.clone())
                    .flatten()
            }),
            agent_mode: req.agent_mode.clone(),
            effort: req.effort.clone(),
            contexts: Vec::new(),
            file_ids: Vec::new(),
            parent_task_id: None,
            // A fork inherits the source Task's run-as through its Sandbox row, so
            // naming one here would be a second answer to the same question.
            run_as_user_id: None,
            headless: None,
            skills: req.skills.clone(),
        };

        let create_result = async {
            let mut new_task = service::normalize(
                ports.normalize,
                scope_ctx,
                &user_id,
                service::CreateTaskIntent {
                    request: create,
                    task_id: Some(fork_task_id.clone()),
                    provision: true,
                    first_message_uuid: None,
                },
            )
            .await?;
            new_task.tag_ids = self.source_tag_ids(task_id).await;
            new_task.forked_from = Some(ForkedFrom {
                source_task_id: task_id.to_string(),
                at_turn: plan.at_turn,
                source_event_id: plan.source_event_id,
            });

            service::create_task(
                ports.task_creation,
                ports.message_send,
                new_task,
                build_event_payload,
            )
            .await
        }
        .await;

        let outcome = match create_result {
            Ok(outcome) => outcome,
            Err(error) => {
                // The Task never came to exist; take its inherited conversation with it
                // rather than leave rows behind a Task that is not there.
                self.discard_conversation(&fork_task_id).await;
                return Err(error);
            }
        };

        tracing::info!(
            task_id = %fork_task_id,
            source_task_id = %task_id,
            at_turn = ?plan.at_turn,
            source_event_id = ?plan.source_event_id,
            same_sandbox,
            "Task forked"
        );

        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(open_turn: Option<i32>, turn: Option<i32>) -> ForkPoint {
        ForkPoint {
            open_turn,
            turn,
            agent_turn_id: None,
        }
    }

    #[test]
    fn a_quiet_task_blocks_nothing() {
        assert_eq!(blocking_open_turn(&point(None, Some(4))), None);
    }

    #[test]
    fn a_cut_below_the_running_turn_is_allowed() {
        assert_eq!(blocking_open_turn(&point(Some(2), Some(1))), None);
    }

    #[test]
    fn a_cut_at_the_running_turn_is_refused() {
        assert_eq!(blocking_open_turn(&point(Some(2), Some(2))), Some(2));
    }

    /// `fork_point` resolves a cut at head to the newest turn that *ended*, so
    /// `turn: None` here means no turn ever ended — the running turn is all
    /// there is, and a fork of it would carry the in-flight rows.
    #[test]
    fn a_task_whose_only_turn_is_running_is_refused() {
        assert_eq!(blocking_open_turn(&point(Some(1), None)), Some(1));
    }
}
