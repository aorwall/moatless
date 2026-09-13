//! Rewinding a Task: dropping its turns above a cut, in place.
//!
//! A fork branches a cut onto a new Task and leaves the parent whole
//! (`task/fork.rs`); a rewind applies the same cut in place, and the next turn
//! resumes the agent session branched at the cut via the same `task_forks`
//! anchors a fork uses.
//!
//! The Sandbox's working copy is untouched — a rewind moves what was *said*,
//! not the files the agent wrote.

use std::sync::Arc;

use moatless_types::access_control::{ScopeContext, ScopedResource};

use super::dao::TaskDao;
use super::error::TaskError;
use super::models::TaskFork;
use super::rewind_dao;
use crate::message::dao::MessageDao;
use crate::message::fork_dao::ForkAnchors;
use crate::task_turn::dao::TaskTurnDao;
use crate::task_turn::models::ForkPoint;

/// Cutting a Task's own conversation back to one of its completed turns.
pub struct TaskRewindService {
    db: moatless_core::db::DbPool,
    tasks: Arc<dyn TaskDao>,
    turns: Arc<dyn TaskTurnDao>,
    messages: Arc<dyn MessageDao>,
}

impl TaskRewindService {
    pub(crate) fn new(
        db: moatless_core::db::DbPool,
        tasks: Arc<dyn TaskDao>,
        turns: Arc<dyn TaskTurnDao>,
        messages: Arc<dyn MessageDao>,
    ) -> Self {
        Self {
            db,
            tasks,
            turns,
            messages,
        }
    }

    /// Drop every turn of `task_id` above `keep_through`, which `0` empties.
    ///
    /// The cut is refused for the reasons [`blocking`] lists.
    pub async fn rewind(
        &self,
        scope_ctx: &ScopeContext,
        task_id: &str,
        keep_through: i32,
    ) -> Result<(), TaskError> {
        let task = self
            .tasks
            .find_by_id(task_id, scope_ctx)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        // Existence hiding, as on every other write: a Task the caller may not
        // control is reported the same way as one that is not there.
        if !scope_ctx.can_control(
            &ScopedResource::borrowed(
                task.scope,
                task.owner_user_id.as_deref().or(task.created_by.as_deref()),
            ),
            task.visibility,
        ) {
            return Err(TaskError::NotFound(task_id.to_string()));
        }

        let point = self.turns.fork_point(task_id, Some(keep_through)).await?;
        if let Some(refusal) = blocking(task_id, keep_through, &point) {
            return Err(refusal);
        }

        let anchor = self.anchor(&task, keep_through, &point).await?;
        let counts =
            rewind_dao::truncate_conversation(&self.db, task_id, keep_through, anchor.as_ref())
                .await?;

        tracing::info!(
            task_id,
            keep_through,
            messages = counts.messages,
            activities = counts.activities,
            turns = counts.turns,
            resumes_session = anchor.is_some(),
            "Task rewound"
        );
        Ok(())
    }

    /// The `task_forks` row the next turn branches from, or `None` when the cut
    /// leaves no agent session. Which anchor it carries follows the agent:
    /// Codex by turn id, Claude by Message uuid.
    async fn anchor(
        &self,
        task: &super::models::Task,
        keep_through: i32,
        point: &ForkPoint,
    ) -> Result<Option<TaskFork>, TaskError> {
        if keep_through == 0 {
            return Ok(None);
        }
        let Some(ForkAnchors {
            session_id,
            message_uuid,
        }) = self
            .messages
            .fork_anchors(&task.task_id, Some(keep_through))
            .await?
        else {
            return Ok(None);
        };

        let is_codex = task.agent_type.as_str() == "codex";
        if is_codex && point.agent_turn_id.is_none() {
            return Err(TaskError::Conflict(format!(
                "turn {keep_through} of task {} has no recorded agent turn id; \
                 it cannot be rewound to",
                task.task_id
            )));
        }

        Ok(Some(TaskFork {
            task_id: task.task_id.clone(),
            // Its own source: the session being branched is this Task's. The
            // fork edge a fork was created with lives in `task_relations`
            // (`relations::record_forked_from`) and is not what this overwrites.
            source_task_id: task.task_id.clone(),
            at_turn: Some(keep_through),
            source_session_id: session_id,
            source_message_uuid: (!is_codex).then_some(message_uuid).flatten(),
            source_agent_turn_id: is_codex.then(|| point.agent_turn_id.clone()).flatten(),
        }))
    }
}

/// Why `keep_through` cannot be cut to, or `None` when it can. Unlike
/// `fork.rs`'s `blocking_open_turn`, an open turn refuses any cut here since
/// this deletes rows; `interrupted` counts as open too.
fn blocking(task_id: &str, keep_through: i32, point: &ForkPoint) -> Option<TaskError> {
    if keep_through < 0 {
        return Some(TaskError::BadRequest(format!(
            "cannot rewind task {task_id} to turn {keep_through}"
        )));
    }
    if let Some(open) = point.open_turn {
        return Some(TaskError::Conflict(format!(
            "task {task_id} has turn {open} in flight; stop it before rewinding"
        )));
    }
    if keep_through > 0 && point.turn.is_none() {
        return Some(TaskError::NotFound(format!(
            "task {task_id} has no turn {keep_through}"
        )));
    }
    None
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
    fn a_quiet_task_cut_at_a_turn_it_has_is_allowed() {
        assert!(blocking("t", 4, &point(None, Some(4))).is_none());
    }

    #[test]
    fn emptying_a_task_needs_no_turn_to_land_on() {
        assert!(blocking("t", 0, &point(None, None)).is_none());
    }

    #[test]
    fn a_negative_cut_is_a_bad_request() {
        assert!(matches!(
            blocking("t", -1, &point(None, Some(4))),
            Some(TaskError::BadRequest(_))
        ));
    }

    /// Where a fork would be allowed: the cut sits below the open turn.
    #[test]
    fn a_cut_below_an_open_turn_is_still_refused() {
        assert!(matches!(
            blocking("t", 1, &point(Some(2), Some(1))),
            Some(TaskError::Conflict(_))
        ));
    }

    /// An open turn outranks a missing one, so the person is told to stop the
    /// turn rather than that a turn they can see does not exist.
    #[test]
    fn an_open_turn_refuses_before_a_missing_one() {
        assert!(matches!(
            blocking("t", 9, &point(Some(2), None)),
            Some(TaskError::Conflict(_))
        ));
    }

    #[test]
    fn a_cut_at_a_turn_the_task_does_not_have_is_not_found() {
        assert!(matches!(
            blocking("t", 4, &point(None, None)),
            Some(TaskError::NotFound(_))
        ));
    }
}
