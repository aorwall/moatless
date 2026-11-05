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

use moatless_api_schemas::task::{CreateTaskRequest, ForkTaskRequest};
use moatless_types::access_control::{ScopeContext, ScopedResource};
use moatless_types::agent::{AgentMode, Harness};
use moatless_types::task::CheckoutMode;

use super::creation::{ForkedFrom, TaskCreationOutcome, TaskCreationService};
use super::dao::TaskDao;
use super::error::TaskError;
use super::service::{self, CreateTaskIntent, CreateTaskPorts};
use super::switch::TaskSwitchService;
use crate::message::dao::MessageDao;
use crate::message::send::MessageSendService;
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

/// What a fork's name starts with until its agent renames it; the prompt
/// reads it to tell the agent the name is a placeholder.
pub(crate) const FORK_NAME_PREFIX: &str = "Fork: ";

/// The placeholder name a fork is created with. `None` when the source is
/// unnamed, so the fork gets the unnamed Task's naming directive.
fn fork_name(source_name: Option<&str>) -> Option<String> {
    let name = source_name.map(str::trim).filter(|name| !name.is_empty())?;
    let mut name = format!(
        "{FORK_NAME_PREFIX}{}",
        name.strip_prefix(FORK_NAME_PREFIX).unwrap_or(name)
    );
    // `validate_task_name` refuses past `MAX_TASK_NAME_BYTES`; cut on a char
    // boundary so the prefix never fails the create.
    while name.len() > service::MAX_TASK_NAME_BYTES {
        name.pop();
    }
    Some(name)
}

/// The fork's first message, opened by a line naming the Task and turn it was
/// forked from.
pub(crate) fn fork_first_message(
    source_task_id: &str,
    at_turn: Option<i32>,
    message: &str,
) -> String {
    let cut = at_turn.map_or_else(String::new, |turn| format!(" at turn {turn}"));
    format!("Forked from task `{source_task_id}`{cut}.\n\n{message}")
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
    pub(super) tasks: Arc<dyn TaskDao>,
    pub(super) turns: Arc<dyn TaskTurnDao>,
    pub(super) messages: Arc<dyn MessageDao>,
    tags: Arc<dyn TagDao>,
    trajectories: Arc<Trajectories>,
    pub(super) relations: Arc<dyn super::relations::TaskRelationDao>,
    pub(super) merge_backs: super::merge_back_dao::MergeBackDao,
}

impl TaskForkService {
    pub(crate) fn new(
        tasks: Arc<dyn TaskDao>,
        turns: Arc<dyn TaskTurnDao>,
        messages: Arc<dyn MessageDao>,
        tags: Arc<dyn TagDao>,
        trajectories: Arc<Trajectories>,
        relations: Arc<dyn super::relations::TaskRelationDao>,
        db: moatless_core::db::DbPool,
    ) -> Self {
        Self {
            tasks,
            turns,
            messages,
            tags,
            trajectories,
            relations,
            merge_backs: super::merge_back_dao::MergeBackDao(db),
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
        self.plan_cut(scope_ctx, source_task_id, at_turn, false)
            .await
    }

    async fn plan_cut(
        &self,
        scope_ctx: &ScopeContext,
        source_task_id: &str,
        at_turn: Option<i32>,
        side_chat: bool,
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
        if let Some(open) = blocking_open_turn(&point)
            .filter(|_| !(side_chat && at_turn.is_none() && point.turn.is_none()))
        {
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

/// The Agent profile a fork runs.
pub enum ForkProfile {
    /// The source's, on the source's model unless the request names one.
    Source,
    /// A named profile. The source's model belongs to the source's profile,
    /// so only the request's model applies.
    Named(String),
    /// The profile `harness` runs in the Sandbox the fork lands in; only the
    /// request's model applies.
    Harness(Harness),
}

/// A fork of `source_task_id`: the cut, and what the fork changes from its
/// source.
pub struct ForkRequest {
    pub side_chat: bool,
    pub source_task_id: String,
    /// The fork's id, when the caller minted it; `None` mints one here.
    pub new_task_id: Option<String>,
    /// The source's turn the copy stops after; `None` cuts after its newest
    /// settled turn.
    pub at_turn: Option<i32>,
    /// The fork's own first message. `None` creates it idle.
    pub message: Option<String>,
    /// The fork's name. `None` names it after the source.
    pub name: Option<String>,
    /// Join the source's Sandbox on its shared checkout instead of
    /// provisioning one.
    pub same_sandbox: bool,
    pub profile: ForkProfile,
    pub model: Option<String>,
    pub agent_mode: Option<AgentMode>,
    pub effort: Option<String>,
    /// Skills to activate for the first turn.
    pub skills: Vec<String>,
}

impl ForkRequest {
    /// The fork the HTTP endpoint asks for, under an id minted here.
    pub fn minted(source_task_id: &str, req: ForkTaskRequest) -> Self {
        Self {
            source_task_id: source_task_id.to_string(),
            new_task_id: None,
            side_chat: false,
            at_turn: req.at_turn,
            message: req.message,
            name: None,
            same_sandbox: req.same_sandbox.unwrap_or(false),
            profile: req
                .agent_profile
                .map_or(ForkProfile::Source, ForkProfile::Named),
            model: req.model,
            agent_mode: req.agent_mode,
            effort: req.effort,
            skills: req.skills,
        }
    }
}

/// Why [`TaskForkService::fork`] refused, split by which Task the refusal is
/// about.
#[derive(Debug)]
pub enum ForkError {
    /// Refused before anything was written: the source cannot be forked, or
    /// the harness asked for cannot run there.
    Source(TaskError),
    /// Refused writing the fork; whatever it inherited is discarded.
    Fork(TaskError),
}

impl From<ForkError> for TaskError {
    fn from(error: ForkError) -> Self {
        match error {
            ForkError::Source(error) | ForkError::Fork(error) => error,
        }
    }
}

/// Dependencies shared by normalization and creation of a fork.
pub struct ForkCreationPorts<'a> {
    pub normalize: CreateTaskPorts<'a>,
    pub task_creation: &'a TaskCreationService,
    pub message_send: Arc<MessageSendService>,
    pub switch: &'a TaskSwitchService,
}

impl TaskForkService {
    /// Fork `request.source_task_id` into a new Task, copying its conversation
    /// up to the cut and discarding the copy when the create fails.
    ///
    /// The fork keeps the source's repository, Workspace, branch and tags.
    pub async fn fork<EvFut>(
        &self,
        ports: ForkCreationPorts<'_>,
        scope_ctx: &ScopeContext,
        request: ForkRequest,
        build_event_payload: impl FnOnce(super::models::Task) -> EvFut + Send,
    ) -> Result<TaskCreationOutcome, ForkError>
    where
        EvFut: std::future::Future<Output = serde_json::Value> + Send,
    {
        let source_task_id = request.source_task_id.as_str();
        let same_sandbox = request.same_sandbox || request.side_chat;
        if request.side_chat {
            let parent = self
                .relations
                .find_parent(source_task_id)
                .await
                .map_err(TaskError::Db)
                .map_err(ForkError::Source)?;
            validate_side_chat_source(parent.as_ref()).map_err(ForkError::Source)?;
        }
        let (source, plan) = self
            .plan_cut(
                scope_ctx,
                source_task_id,
                request.at_turn,
                request.side_chat,
            )
            .await
            .map_err(ForkError::Source)?;

        let (agent_profile, model) = match request.profile {
            ForkProfile::Source => (
                source.agent_profile.to_string(),
                request.model.or_else(|| source.model.clone()),
            ),
            ForkProfile::Named(profile) => (profile, request.model),
            ForkProfile::Harness(harness) => {
                let sandbox_id =
                    same_sandbox.then(|| source.sandbox_id.as_deref().unwrap_or(source_task_id));
                let profile = ports
                    .switch
                    .profile_for(sandbox_id, harness)
                    .await
                    .map_err(ForkError::Source)?;
                (profile.to_string(), request.model)
            }
        };

        // Minted before the creation service runs: the conversation is
        // projected onto it before the Task row exists, which is what puts the
        // inherited Messages behind the fork's own first message.
        let fork_task_id = request
            .new_task_id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.inherit_conversation(&plan, &fork_task_id)
            .await
            .map_err(ForkError::Fork)?;

        let create = CreateTaskRequest {
            repository_id: source.repository_id.clone(),
            workspace_id: source.workspace_id.clone(),
            sandbox_id: same_sandbox.then(|| source.sandbox_id.clone()).flatten(),
            checkout_mode: same_sandbox.then_some(CheckoutMode::Shared),
            agent_profile: Some(agent_profile),
            name: request.name.or_else(|| fork_name(source.name.as_deref())),
            description: None,
            branch: source.branch.clone(),
            message: request
                .message
                .map(|message| fork_first_message(source_task_id, plan.at_turn, &message)),
            model,
            agent_mode: request.agent_mode,
            effort: request.effort,
            contexts: Vec::new(),
            file_ids: Vec::new(),
            parent_task_id: None,
            // A fork inherits the source Task's run-as through its Sandbox row, so
            // naming one here would be a second answer to the same question.
            run_as_user_id: None,
            headless: None,
            skills: request.skills,
            agent_properties: source.agent_properties.clone(),
        };

        let create_result = async {
            let mut new_task = service::normalize(
                ports.normalize,
                scope_ctx,
                &scope_ctx.user_id,
                CreateTaskIntent {
                    request: create,
                    task_id: Some(fork_task_id.clone()),
                    provision: true,
                    first_message_uuid: None,
                },
            )
            .await?;
            new_task.tag_ids = self.source_tag_ids(source_task_id).await;
            new_task.reporter_user_id = source.reporter_user_id.clone();
            new_task.forked_from = Some(ForkedFrom {
                source_task_id: source_task_id.to_string(),
                at_turn: plan.at_turn,
                source_event_id: plan.source_event_id,
                side_chat: request.side_chat,
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
                return Err(ForkError::Fork(error));
            }
        };

        tracing::info!(
            task_id = %fork_task_id,
            %source_task_id,
            at_turn = ?plan.at_turn,
            source_event_id = ?plan.source_event_id,
            same_sandbox,
            "Task forked"
        );

        Ok(outcome)
    }
}

fn validate_side_chat_source(
    parent: Option<&super::relations::TaskRelation>,
) -> Result<(), TaskError> {
    if parent.is_some_and(super::relations::TaskRelation::is_side_chat) {
        return Err(TaskError::Conflict(
            "Side chats cannot start from a side chat".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_chats_refuse_nested_sources_but_allow_child_tasks_and_ordinary_forks() {
        use super::super::relations::{TaskRelation, TaskRelationType};
        assert!(validate_side_chat_source(None).is_ok());
        for (kind, side_chat, allowed) in [
            (TaskRelationType::CreatedBy, false, true),
            (TaskRelationType::ForkedFrom, true, false),
            (TaskRelationType::ForkedFrom, false, true),
        ] {
            let parent = TaskRelation::new(
                "source",
                "parent",
                kind,
                serde_json::json!({"side_chat": side_chat}),
            );
            assert_eq!(validate_side_chat_source(Some(&parent)).is_ok(), allowed);
        }
    }

    #[tokio::test]
    async fn a_side_chat_can_start_with_an_empty_cut_during_the_first_turn() {
        use crate::message::dao::MockMessageDao;
        use crate::tag::dao::MockTagDao;
        use crate::task::dao::MockTaskDao;
        use crate::task_turn::dao::MockTaskTurnDao;
        use crate::trajectory::dao::PgTrajectoryDao;
        let mut tasks = MockTaskDao::new();
        tasks
            .expect_find_by_id()
            .returning(|id, _| Ok(Some(super::super::test_support::make_task_row(id, "owner"))));
        let mut turns = MockTaskTurnDao::new();
        turns
            .expect_fork_point()
            .returning(|_, _| Ok(point(Some(1), None)));
        let mut messages = MockMessageDao::new();
        messages.expect_is_frozen().returning(|_| Ok(false));
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@localhost/unused")
            .unwrap();
        let service = TaskForkService::new(
            Arc::new(tasks),
            Arc::new(turns),
            Arc::new(messages),
            Arc::new(MockTagDao::new()),
            Arc::new(Trajectories::new(PgTrajectoryDao(pool.clone()))),
            Arc::new(super::super::relations::MockTaskRelationDao::new()),
            pool,
        );
        let viewer = ScopeContext::user("owner");
        assert!(service.plan(&viewer, "parent", None).await.is_err());
        let (_, plan) = service
            .plan_cut(&viewer, "parent", None, true)
            .await
            .unwrap();
        assert!(plan.at_turn.is_none());
        assert!(plan.source_event_id.is_none());
    }

    fn point(open_turn: Option<i32>, turn: Option<i32>) -> ForkPoint {
        ForkPoint {
            open_turn,
            turn,
            agent_turn_id: None,
        }
    }

    #[test]
    fn a_fork_is_named_after_its_source() {
        assert_eq!(
            fork_name(Some("Fix login")).as_deref(),
            Some("Fork: Fix login")
        );
    }

    /// A fork of a fork reads `Fork: X`, not `Fork: Fork: X`.
    #[test]
    fn a_fork_of_a_fork_does_not_stack_the_prefix() {
        assert_eq!(
            fork_name(Some("Fork: Fix login")).as_deref(),
            Some("Fork: Fix login")
        );
    }

    #[test]
    fn a_long_source_name_is_cut_to_the_name_limit() {
        let name = fork_name(Some(&"é".repeat(100))).unwrap();
        assert!(name.len() <= service::MAX_TASK_NAME_BYTES);
        assert!(name.starts_with(FORK_NAME_PREFIX));
    }

    #[test]
    fn a_fork_of_an_unnamed_task_is_unnamed() {
        assert_eq!(fork_name(None), None);
        assert_eq!(fork_name(Some("  ")), None);
    }

    #[test]
    fn the_first_message_opens_with_the_source_and_turn() {
        assert_eq!(
            fork_first_message("task-1", Some(3), "try again"),
            "Forked from task `task-1` at turn 3.\n\ntry again"
        );
        assert_eq!(
            fork_first_message("task-1", None, "try again"),
            "Forked from task `task-1`.\n\ntry again"
        );
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
