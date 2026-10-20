//! Interrupted-turn recovery budget, intent gates, and ordered recovery events.

use super::operations::TaskTurns;
use crate::{
    message::models::InterruptedTurn, message::operations::Messages, task::operations::TaskCommands,
};
use moatless_core::events::EventPublisher;
use moatless_types::{access_control::ScopeContext, agent::AgentSubType};
use tracing::{debug, info, warn};

/// A recorded recovery attempt that may be replayed by the sandbox.
pub struct RecoveredTurn {
    pub interrupted: InterruptedTurn,
    pub agent_profile: Option<String>,
}

/// A failed read or attempt write that prevents recovery replay.
#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error("Failed to query interrupted turn")]
    Query(#[source] sqlx::Error),
    #[error("Failed to increment recovery attempts")]
    RecordAttempt(#[source] sqlx::Error),
}

/// Recover a turn after the caller has authorized sandbox access to `task_id`.
pub async fn recover_interrupted_turn(
    tasks: Option<&TaskCommands>,
    messages: Option<&Messages>,
    task_turns: Option<&TaskTurns>,
    events: Option<&dyn EventPublisher>,
    task_id: &str,
) -> Result<Option<RecoveredTurn>, RecoveryError> {
    let Some(messages) = messages else {
        return Ok(None);
    };
    let Some((interrupted, task)) = gated_interrupted_turn(tasks, messages, task_id).await? else {
        return Ok(None);
    };

    const MAX_RECOVERY_ATTEMPTS: i32 = 3;

    // Only consecutive no-progress recoveries count toward the cap.
    let mut recovery_attempts = interrupted.recovery_attempts;
    if recovery_attempts > 0
        && messages
            .turn_progressed_since_last_recovery(task_id, interrupted.turn_number)
            .await
            .unwrap_or(false)
    {
        if let Some(task_turns) = task_turns {
            match task_turns
                .store()
                .reset_recovery_attempts(task_id, interrupted.turn_number)
                .await
            {
                Ok(()) => {
                    info!(
                        task_id = %task_id,
                        turn_number = interrupted.turn_number,
                        prior_attempts = recovery_attempts,
                        "RecoverInterruptedRun: last re-drive made progress; resetting recovery counter"
                    );
                    recovery_attempts = 0;
                }
                Err(e) => {
                    warn!(error = %e, task_id = %task_id, "reset_recovery_attempts failed");
                }
            }
        }
    }

    if recovery_attempts >= MAX_RECOVERY_ATTEMPTS {
        warn!(
            task_id = %task_id,
            turn_number = interrupted.turn_number,
            attempts = recovery_attempts,
            "RecoverInterruptedRun: cap reached, marking aborted"
        );
        if let Some(publisher) = events {
            publisher
                .publish_agent_event(
                    task_id,
                    AgentSubType::RecoveryExhausted,
                    None,
                    serde_json::json!({
                        "turn_number": interrupted.turn_number,
                        "session_id": interrupted.session_id,
                        "attempts": recovery_attempts,
                    }),
                )
                .await;
        }
        return Ok(None);
    }

    let Some(task_turns) = task_turns else {
        warn!(task_id = %task_id, "Cannot record a recovery attempt without task_turns");
        return Ok(None);
    };
    let new_attempts = match task_turns
        .store()
        .increment_recovery_attempts(task_id, interrupted.turn_number)
        .await
    {
        Ok(n) => n,
        Err(e) => {
            warn!(error = %e, task_id = %task_id, "increment_recovery_attempts failed");
            return Err(RecoveryError::RecordAttempt(e));
        }
    };

    let task_agent_profile = task.as_ref().map(|t| t.agent_profile.to_string());

    if let Some(publisher) = events {
        publisher
            .publish_agent_event(
                task_id,
                AgentSubType::RecoveryStarted,
                task_agent_profile.as_deref(),
                serde_json::json!({
                    "turn_number": interrupted.turn_number,
                    "session_id": interrupted.session_id,
                    "attempt": new_attempts,
                    "max_attempts": MAX_RECOVERY_ATTEMPTS,
                }),
            )
            .await;
    }

    info!(
        task_id = %task_id,
        session_id = %interrupted.session_id,
        turn_number = interrupted.turn_number,
        attempt = new_attempts,
        "RecoverInterruptedRun: replaying interrupted turn"
    );

    Ok(Some(RecoveredTurn {
        interrupted,
        agent_profile: task_agent_profile,
    }))
}

/// The interrupted turn [`recover_interrupted_turn`] would consider, read
/// without recording an attempt or publishing an event.
///
/// Skips the attempt cap, so it may return a turn a replay would refuse as
/// exhausted.
pub async fn probe_interrupted_turn(
    tasks: Option<&TaskCommands>,
    messages: Option<&Messages>,
    task_id: &str,
) -> Result<Option<InterruptedTurn>, RecoveryError> {
    let Some(messages) = messages else {
        return Ok(None);
    };
    Ok(gated_interrupted_turn(tasks, messages, task_id)
        .await?
        .map(|(interrupted, _)| interrupted))
}

/// The newest interrupted turn, with its Task, unless the Task is Closed or its
/// Sandbox's intent is Stopped or Removed.
async fn gated_interrupted_turn(
    tasks: Option<&TaskCommands>,
    messages: &Messages,
    task_id: &str,
) -> Result<Option<(InterruptedTurn, Option<moatless_types::task::Task>)>, RecoveryError> {
    // Unknown task or intent reads fail open; only confirmed terminal state gates replay.
    let task = if let Some(tasks) = tasks {
        // reason: recovery gate reads task status across all owners (no user ctx)
        #[allow(clippy::disallowed_methods)]
        let sys = ScopeContext::system();
        tasks.find_by_id(task_id, &sys).await.ok().flatten()
    } else {
        None
    };
    if matches!(
        task.as_ref().map(|t| t.status),
        Some(moatless_types::task::TaskStatus::Closed)
    ) {
        debug!(task_id = %task_id, "recover_interrupted_run: task is Closed, skipping recovery");
        return Ok(None);
    }
    // No Sandbox row means the Task never requested runtime resources;
    // only explicit Stopped or Removed intent gates recovery.
    let sandbox_desired_state = if let Some(tasks) = tasks {
        match tasks.get_sandbox_intents(&[task_id.to_owned()]).await {
            Ok(mut intents) => intents
                .remove(task_id)
                .filter(crate::task::models::SandboxIntentView::has_explicit_intent)
                .map(|intent| intent.desired_state),
            Err(e) => {
                warn!(
                    error = %e,
                    task_id = %task_id,
                    "Failed to load Sandbox lifecycle intent for recovery gate"
                );
                None
            }
        }
    } else {
        None
    };
    if matches!(
        sandbox_desired_state,
        Some(moatless_types::sandbox::SandboxDesiredState::Stopped)
            | Some(moatless_types::sandbox::SandboxDesiredState::Removed)
    ) {
        debug!(
            task_id = %task_id,
            "recover_interrupted_run: task does not desire a running sandbox, skipping recovery"
        );
        return Ok(None);
    }

    let interrupted = match messages.find_interrupted_turn(task_id).await {
        Ok(Some(turn)) => turn,
        Ok(None) => {
            debug!(task_id = %task_id, "RecoverInterruptedRun: no interrupted turn");
            return Ok(None);
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "find_interrupted_turn failed");
            return Err(RecoveryError::Query(e));
        }
    };
    Ok(Some((interrupted, task)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::dao::MockMessageDao;
    use crate::task::dao::MockTaskDao;
    use crate::task::models::SandboxIntentView;
    use crate::task_turn::dao::MockTaskTurnDao;
    use crate::task_turn::operations::TaskTurns;
    use moatless_core::events::test_support::{RecordedEvent, RecordingPublisher};
    use moatless_types::sandbox::SandboxDesiredState;
    use std::sync::Arc;

    use moatless_types::agent::AgentProfile;
    use moatless_types::task::{Task, TaskStatus, Visibility};

    fn commands(store: MockTaskDao) -> TaskCommands {
        TaskCommands::new(Arc::new(store))
    }

    fn transcript(store: MockMessageDao) -> Messages {
        Messages::new(
            Arc::new(store),
            crate::inbox::InboxService::new(
                sqlx::PgPool::connect_lazy("postgres://unused").expect("a lazy pool"),
            ),
        )
    }

    fn turn_ops(store: MockTaskTurnDao) -> TaskTurns {
        TaskTurns::new(Arc::new(store))
    }

    fn closed_task() -> Task {
        Task {
            task_id: "task-1".to_string(),
            short_id: None,
            created_at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00.000Z")
                .expect("fixture timestamp")
                .with_timezone(&chrono::Utc),
            last_message_at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00.000Z")
                .expect("fixture timestamp")
                .with_timezone(&chrono::Utc),
            agent_profile: AgentProfile::Codex,
            repository_id: "repo-1".to_string(),
            status: TaskStatus::Closed,
            total_cost_usd: 0.0,
            site_url: None,
            template_id: None,
            vibe: None,
            name: None,
            description: None,
            site_id: None,
            site_provider: None,
            branch: None,
            model: None,
            created_by: None,
            visibility: Visibility::Private,
            scope: moatless_types::access_control::Scope::User,
            owner_user_id: Some("user-1".to_string()),
            run_as_user_id: None,
            updated_at: None,
            append_system_prompt: None,
            headless: false,
            ttl_minutes: 60,
            inactive_ttl_days: 7,
            loop_id: None,
            workspace_id: None,
            sandbox_id: None,
            checkout_mode: None,
            memory_preload: Vec::new(),
            skills: Vec::new(),
        }
    }

    #[tokio::test]
    async fn closed_task_does_not_read_intent_turn_or_consume_attempt() {
        let mut tasks = MockTaskDao::new();
        tasks
            .expect_find_by_id()
            .return_once(|_, _| Ok(Some(closed_task())));
        assert!(
            recover_interrupted_turn(
                Some(&commands(tasks)),
                Some(&transcript(MockMessageDao::new())),
                Some(&turn_ops(MockTaskTurnDao::new())),
                None,
                "task"
            )
            .await
            .unwrap()
            .is_none()
        );
    }

    fn interrupted(attempts: i32) -> InterruptedTurn {
        InterruptedTurn {
            task_id: "task".into(),
            session_id: "session".into(),
            turn_number: 7,
            prompt_text: "continue".into(),
            agent_mode: Some("code".into()),
            effort: Some("high".into()),
            model: Some("claude-haiku-4-5".into()),
            recovery_attempts: attempts,
            run_id: Some("run".into()),
        }
    }

    #[tokio::test]
    async fn explicit_stop_or_removal_does_not_read_turn_or_consume_attempt() {
        for desired_state in [SandboxDesiredState::Stopped, SandboxDesiredState::Removed] {
            let mut tasks = MockTaskDao::new();
            tasks.expect_find_by_id().return_once(|_, _| Ok(None));
            tasks.expect_get_sandbox_intents().return_once(move |_| {
                Ok(std::collections::HashMap::from([(
                    "task".into(),
                    SandboxIntentView {
                        desired_state,
                        desired_state_updated_at: chrono::Utc::now(),
                        ..Default::default()
                    },
                )]))
            });
            let messages = transcript(MockMessageDao::new());
            let turns = turn_ops(MockTaskTurnDao::new());
            assert!(
                recover_interrupted_turn(
                    Some(&commands(tasks)),
                    Some(&messages),
                    Some(&turns),
                    None,
                    "task"
                )
                .await
                .unwrap()
                .is_none()
            );
        }
    }

    #[tokio::test]
    async fn progress_resets_budget_before_recording_and_replaying() {
        let mut sequence = mockall::Sequence::new();
        let mut messages = MockMessageDao::new();
        messages
            .expect_find_interrupted_turn()
            .return_once(|_| Ok(Some(interrupted(3))));
        messages
            .expect_turn_progressed_since_last_recovery()
            .return_once(|_, _| Ok(true));
        let mut turns = MockTaskTurnDao::new();
        turns
            .expect_reset_recovery_attempts()
            .times(1)
            .in_sequence(&mut sequence)
            .return_once(|_, _| Ok(()));
        turns
            .expect_increment_recovery_attempts()
            .times(1)
            .in_sequence(&mut sequence)
            .return_once(|_, _| Ok(1));
        let events = RecordingPublisher::new();
        let messages = transcript(messages);
        let turns = turn_ops(turns);
        let recovered =
            recover_interrupted_turn(None, Some(&messages), Some(&turns), Some(&events), "task")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(recovered.interrupted.run_id.as_deref(), Some("run"));
        assert_eq!(recovered.interrupted.effort.as_deref(), Some("high"));
        assert_eq!(
            events.events(),
            vec![RecordedEvent::Agent(AgentSubType::RecoveryStarted)]
        );
    }

    #[tokio::test]
    async fn exhausted_budget_emits_exhaustion_without_another_attempt() {
        let mut messages = MockMessageDao::new();
        messages
            .expect_find_interrupted_turn()
            .return_once(|_| Ok(Some(interrupted(3))));
        messages
            .expect_turn_progressed_since_last_recovery()
            .return_once(|_, _| Ok(false));
        let turns = turn_ops(MockTaskTurnDao::new());
        let events = RecordingPublisher::new();
        let messages = transcript(messages);
        assert!(
            recover_interrupted_turn(None, Some(&messages), Some(&turns), Some(&events), "task")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            events.events(),
            vec![RecordedEvent::Agent(AgentSubType::RecoveryExhausted)]
        );
    }

    #[tokio::test]
    async fn failed_attempt_write_never_replays_or_emits_started() {
        let mut messages = MockMessageDao::new();
        messages
            .expect_find_interrupted_turn()
            .return_once(|_| Ok(Some(interrupted(0))));
        let mut turns = MockTaskTurnDao::new();
        turns
            .expect_increment_recovery_attempts()
            .return_once(|_, _| Err(sqlx::Error::PoolClosed));
        let events = RecordingPublisher::new();
        let messages = transcript(messages);
        let turns = turn_ops(turns);
        let result =
            recover_interrupted_turn(None, Some(&messages), Some(&turns), Some(&events), "task")
                .await;
        assert!(matches!(
            result,
            Err(RecoveryError::RecordAttempt(sqlx::Error::PoolClosed))
        ));
        assert!(events.events().is_empty());
    }

    #[tokio::test]
    async fn missing_turn_store_never_replays() {
        let mut messages = MockMessageDao::new();
        messages
            .expect_find_interrupted_turn()
            .return_once(|_| Ok(Some(interrupted(0))));
        let messages = transcript(messages);
        assert!(
            recover_interrupted_turn(None, Some(&messages), None, None, "task")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn probe_reports_the_turn_a_replay_would_take() {
        let mut messages = MockMessageDao::new();
        messages
            .expect_find_interrupted_turn()
            .return_once(|_| Ok(Some(interrupted(2))));
        let probed = probe_interrupted_turn(None, Some(&transcript(messages)), "task")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(probed.turn_number, 7);
    }

    #[tokio::test]
    async fn probe_of_a_closed_task_reads_no_turn() {
        let mut tasks = MockTaskDao::new();
        tasks
            .expect_find_by_id()
            .return_once(|_, _| Ok(Some(closed_task())));
        assert!(
            probe_interrupted_turn(
                Some(&commands(tasks)),
                Some(&transcript(MockMessageDao::new())),
                "task"
            )
            .await
            .unwrap()
            .is_none()
        );
    }
}
