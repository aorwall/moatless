//! The turn a scheduled job opens, proven at the harness.
//!
//! A `ScheduleWakeup` or `moat cmd` completion prompt handed straight to the
//! `AgentHandle` opens a turn no `user` row backs, and `find_interrupted_turn`
//! selects on that row — so a pod replaced mid-turn never resumed it. These
//! tests pin the queue-then-dispatch path that closes it.

use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::backend::ReplayBackend;
use super::support::{
    park_plan_prompt, recorded_inputs, recording_state, restore_state, settled_inputs, text_message,
};
use crate::agent::dispatcher::{dispatch_pending_messages_for_task, dispatch_self_queued};
use crate::agent::queued_injector::QueuedInjector;
use crate::agent::scheduled_job::ScheduledJobRegistry;
use crate::agent::wakeup_job::WakeupJob;
use crate::config::test_support::minimal_config;
use crate::init::InitPhase;
use crate::state::SandboxState;

/// A registry wired the way `compose_task_context` wires the production one.
fn registry(state: &SandboxState) -> ScheduledJobRegistry {
    ScheduledJobRegistry::new(Arc::new(QueuedInjector::new(
        state.config.task_id.clone(),
        state.clone(),
    )))
}

fn wakeup(id: &str, prompt: &str) -> WakeupJob {
    WakeupJob::new_for_test(
        id.to_string(),
        "waiting".to_string(),
        prompt.to_string(),
        Instant::now() + Duration::from_millis(10),
    )
}

#[tokio::test]
async fn a_job_prompt_is_queued_under_the_job_id_and_reaches_the_agent() {
    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, recorder) = recording_state(Some("claude-code"), InitPhase::Ready, backend.clone());
    let jobs = registry(&state);

    jobs.register(
        Box::new(wakeup("toolu_wake_1", "continue work")),
        CancellationToken::new(),
    )
    .await;

    assert_eq!(recorded_inputs(&recorder, 1).await.len(), 1);
    let inputs = settled_inputs(&recorder).await;
    assert_eq!(inputs.len(), 1, "one prompt, one turn: {inputs:?}");
    assert_eq!(inputs[0].text, "continue work");
    assert_eq!(
        backend.queued().await,
        vec![("toolu_wake_1".to_string(), "continue work".to_string())],
        "the turn is only recoverable if the backend holds the row that opened it"
    );
}

/// The `/loop` case: every tick re-registers the same prompt under a fresh job
/// id. Keying the queue on the text would queue the first tick and swallow the
/// rest, so the loop would stop after one iteration.
#[tokio::test]
async fn two_ticks_of_one_prompt_each_open_a_turn() {
    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, recorder) = recording_state(Some("claude-code"), InitPhase::Ready, backend.clone());
    let jobs = registry(&state);

    jobs.register(
        Box::new(wakeup("toolu_tick_1", "tick")),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(recorded_inputs(&recorder, 1).await.len(), 1);

    jobs.register(
        Box::new(wakeup("toolu_tick_2", "tick")),
        CancellationToken::new(),
    )
    .await;

    let inputs = recorded_inputs(&recorder, 2).await;
    assert_eq!(inputs.len(), 2, "the second tick was swallowed: {inputs:?}");
    assert_eq!(backend.queued().await.len(), 2);
}

/// A `moat cmd` finishing while the agent is parked on `ExitPlanMode` must not
/// answer that parked call.
///
/// The dispatcher answers a parked call with the first pending message;
/// treating the wake as that answer would reject an unread plan.
#[tokio::test]
async fn a_job_prompt_does_not_answer_a_parked_prompt() {
    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, recorder) = recording_state(Some("claude-code"), InitPhase::Ready, backend.clone());
    let context = state.task_context(None).await.expect("task context");
    let jobs = registry(&state);

    let mut answer = park_plan_prompt(&context, "toolu_plan_1").await;

    jobs.register(
        Box::new(wakeup("toolu_wake_3", "a background command finished")),
        CancellationToken::new(),
    )
    .await;

    assert!(
        settled_inputs(&recorder).await.is_empty(),
        "a parked run takes no input"
    );
    assert!(
        answer.try_recv().is_err(),
        "the job's wake was delivered as the person's verdict on the plan"
    );
    assert!(
        context.agent.has_pending_prompt().await,
        "the plan is still waiting on a person"
    );
    // Left pending rather than dropped: the dispatch that follows the real
    // answer opens the turn this wake was owed.
    assert_eq!(backend.queued().await.len(), 1);
    assert_eq!(backend.pending().await.len(), 1);
}

/// The backend coalesces the person's answer and a waiting wake into one
/// delivery; acking it whole wrongly settles the wake as the answer.
///
/// The answer must still reach the parked call, and the wake must survive to
/// open its own turn afterward.
#[tokio::test]
async fn a_withheld_wake_survives_the_answer_and_opens_its_own_turn() {
    let backend = ReplayBackend::new(Ok(vec![]), None);
    let (state, recorder) = recording_state(Some("claude-code"), InitPhase::Ready, backend.clone());
    let context = state.task_context(None).await.expect("task context");
    let jobs = registry(&state);

    let mut answer = park_plan_prompt(&context, "toolu_plan_2").await;
    jobs.register(
        Box::new(wakeup("toolu_wake_4", "a background command finished")),
        CancellationToken::new(),
    )
    .await;
    assert!(settled_inputs(&recorder).await.is_empty());

    backend
        .push_pending(text_message("go ahead", "uuid-person"))
        .await;
    dispatch_pending_messages_for_task(state.clone(), state.config.task_id.clone()).await;

    assert!(
        answer.try_recv().is_ok(),
        "the person's answer never reached the parked plan"
    );
    let pending = backend.pending().await;
    assert_eq!(
        pending
            .iter()
            .flat_map(|row| row.message_uuids.clone())
            .collect::<Vec<_>>(),
        vec!["queued-1".to_string()],
        "the answer must be acked and the wake left pending"
    );

    // The redispatch the answer schedules runs after `REDISPATCH_BACKOFF`; drive
    // it directly rather than holding the test open for 15s.
    dispatch_pending_messages_for_task(state.clone(), state.config.task_id.clone()).await;
    let inputs = settled_inputs(&recorder).await;
    assert_eq!(inputs.len(), 1, "the wake never opened a turn: {inputs:?}");
    assert_eq!(inputs[0].text, "a background command finished");
    assert!(backend.pending().await.is_empty());
}

/// A dispatch the agent never accepted (its ack channel dropped) leaves the row
/// pending and still marked as this pod's own wake.
///
/// The mark must outlive that failed dispatch through the `REDISPATCH_BACKOFF`
/// retry, even if a run parks a prompt in between.
#[tokio::test]
async fn a_wake_the_agent_never_accepted_keeps_its_mark() {
    let backend = ReplayBackend::new(Ok(vec![]), None);
    // A bare agent channel with no receiver: the send's ack is dropped, which is
    // the harness never taking the row.
    let state = restore_state(
        Arc::new(minimal_config()),
        InitPhase::Ready,
        backend.clone(),
    );
    let context = state.task_context(None).await.expect("task context");

    let uuid = state
        .backend_client
        .enqueue_sandbox_message(
            &state.config.task_id,
            "a background command finished",
            Some("toolu_wake_5"),
        )
        .await
        .expect("enqueue")
        .expect("a fresh row");
    dispatch_self_queued(state.clone(), state.config.task_id.clone(), uuid).await;

    let mut answer = park_plan_prompt(&context, "toolu_plan_3").await;

    // Blocks on the dispatch lock until the dispatch above gives it up, so this
    // is what the redispatch sees and not a race with it.
    dispatch_pending_messages_for_task(state.clone(), state.config.task_id.clone()).await;

    assert!(
        answer.try_recv().is_err(),
        "a wake the agent never took was delivered as the person's verdict on the plan"
    );
    assert!(context.agent.has_pending_prompt().await);
    assert_eq!(backend.pending().await.len(), 1);
}

/// A backend the pod cannot reach must not cost the agent its wake. The turn
/// this opens is the unrecoverable one the fix exists to avoid, so it is the
/// fallback and never the path.
#[tokio::test]
async fn an_unreachable_backend_falls_back_to_a_direct_send() {
    let backend = ReplayBackend::new(Ok(vec![]), None);
    backend.fail_enqueue();
    let (state, recorder) = recording_state(Some("claude-code"), InitPhase::Ready, backend.clone());
    let jobs = registry(&state);

    jobs.register(
        Box::new(wakeup("toolu_wake_2", "continue work")),
        CancellationToken::new(),
    )
    .await;

    let inputs = recorded_inputs(&recorder, 1).await;
    assert_eq!(inputs.len(), 1, "the wake must still reach the agent");
    assert_eq!(inputs[0].text, "continue work");
    assert!(backend.queued().await.is_empty());
}
