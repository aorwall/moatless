//! Message dispatcher — routes incoming message notifications.
//!
//! Invoked from:
//! - `grpc_server::execute_agent()` when backend requests execution
//! - `grpc_server::notify_messages()` when backend signals new messages
//! - `init::run_init()` after `InitPhase::Ready` is set
//!
//! The dispatcher fetches pending messages from the backend, sends text to
//! the agent lifecycle task via `agent_handle.send()`, and acknowledges the
//! messages. The coordinator handles whether to start a new session or inject
//! mid-turn.
//!
//! It self-gates on [`SandboxState::is_ready`]: the two gRPC entry points fire
//! from the moment the pod starts, so a call that races `run_init` is dropped
//! (the messages stay pending and `run_init` dispatches them after restoration).

use std::time::Duration;

use tracing::{error, info, warn};

use crate::files::uploads::fetch_uploads_for_messages;
use crate::state::SandboxState;

use super::prompt::{PromptResponse, PromptToolName};

/// Spawn a detached dispatcher task whose termination is never silent (Sandbox
/// spawned-task rule). The task itself logs every ordinary terminal branch; this
/// watcher additionally surfaces an abnormal end (a panic) at `error` with the
/// task's name, which a bare `tokio::spawn` would swallow.
///
/// # Why these tasks are not registered with the root supervisor
///
/// Dispatch work is per-request and unbounded in count, so handing every one to
/// [`crate::supervisor::Supervisor`] would grow its owned set for the life of
/// the pod. More importantly, dispatch owns *pending-message durability*: a
/// dispatcher holds the single-flight guard from before the fetch until
/// [`acknowledge_until_settled`] lands the ack, and that retry loop deliberately
/// keeps going for as long as the pod is up. The root supervisor's job is to
/// bound teardown; a task whose correct behaviour is to outlast every other one
/// must not be inside that bound. Each dispatcher already races every await
/// against `lifecycle_cancel`, which is what actually stops it.
pub(crate) fn spawn_supervised(
    what: &'static str,
    fut: impl std::future::Future<Output = ()> + Send + 'static,
) {
    let handle = tokio::spawn(fut);
    tokio::spawn(async move {
        if let Err(join_error) = handle.await {
            error!(error = %join_error, task = what, "dispatcher task terminated abnormally");
        }
    });
}

/// Dispatch pending messages to the agent.
///
/// If the agent is waiting for user input (AskUserQuestion/ExitPlanMode),
/// resolves the pending prompt. Otherwise sends text to the agent lifecycle task.
pub async fn dispatch_pending_messages(state: SandboxState) {
    // The pod is shutting down (SIGTERM mid-boot or between notifications) —
    // don't start a new turn that would just be torn down. The lifecycle token
    // is cancelled by the shutdown driver.
    if state.lifecycle_cancel.is_cancelled() {
        info!("Dispatch skipped: sandbox is shutting down");
        return;
    }

    // Init is not finished: the gRPC server accepts `execute_agent` /
    // `notify_messages` from the moment the pod starts — before `run_init`
    // restores Claude/Codex Session state and reaches Ready. Driving the harness
    // now would submit input against un-restored `--resume` history. Dropping the
    // call is safe: the messages stay pending on the backend, and `run_init`
    // performs the authoritative dispatch itself right after restoration + Ready.
    if !state.is_ready().await {
        info!("Dispatch skipped: init not Ready — run_init will dispatch after restore");
        return;
    }

    // Single-flight: hold an owned guard from before the fetch (and before the
    // prompt branch) until the ack completes. `get_pending_messages` only returns
    // unacknowledged rows and the ack lands after the agent accepts, so without
    // this two dispatchers racing the Ready transition each fetch the same unacked
    // Message and submit it twice; and two racing the *prompt* branch each fetch
    // the same prompt Message — one answers the prompt, the loser hits the
    // "prompt already consumed" fallback and resubmits it as normal Input. A
    // concurrent dispatcher blocks here and then fetches empty.
    //
    // Acquisition is cancellation-aware: a dispatcher queued on the lock across a
    // shutdown must not acquire it and start new work after the pod is stopping.
    let dispatch_guard = tokio::select! {
        guard = state.dispatch_lock.clone().lock_owned() => guard,
        () = state.lifecycle_cancel.cancelled() => {
            info!("Dispatch skipped: shutting down while awaiting the dispatch lock");
            return;
        }
    };
    if state.lifecycle_cancel.is_cancelled() {
        info!("Dispatch skipped: shutting down after acquiring the dispatch lock");
        return;
    }

    // If agent is waiting for user input, resolve the prompt — under the guard, so
    // the prompt Message is answered and acknowledged before any concurrent
    // dispatcher can re-fetch it.
    if state.agent.has_pending_prompt().await {
        resolve_pending_prompt(&state).await;
        return;
    }

    // Fetch pending messages from backend
    let task_id = &state.config.task_id;
    let deliveries = match state.backend_client.get_pending_messages(task_id).await {
        Ok(deliveries) => deliveries,
        Err(e) => {
            warn!(error = %e, "Failed to fetch pending messages");
            return;
        }
    };

    let mut deliveries = deliveries.into_iter();
    let Some(delivery) = deliveries.next() else {
        return;
    };
    let deferred = deliveries.count();

    // Pre-fetch any attached uploaded files before invoking the agent so the
    // on-disk paths advertised in the prompt are valid by the time the agent
    // reads them. Best-effort: errors are logged but don't block dispatch.
    let file_ids = delivery.file_ids.clone();
    if !file_ids.is_empty() {
        fetch_uploads_for_messages(
            &state.backend_client,
            task_id,
            file_ids,
            &state.config.upload_dir,
        )
        .await;
    }

    let uuids = delivery.message_uuids.clone();
    let text = delivery.prompt_text;
    let model = delivery.model;
    let agent_type = delivery.agent_type;
    let agent_mode = delivery.agent_mode;
    let resume_session_id = delivery.resume_session_id;
    let start_fresh = delivery.start_fresh;

    if text.is_empty() {
        return;
    }

    info!(
        message_count = uuids.len(),
        deferred_count = deferred,
        text_len = text.len(),
        model = ?model,
        agent_type = ?agent_type,
        agent_mode = ?agent_mode,
        resume_session_id = ?resume_session_id,
        start_fresh,
        "Dispatching messages to agent"
    );

    // The fetch + upload work above awaited, so the pod may have begun shutting
    // down since the post-acquire recheck. Recheck immediately before opening a
    // turn so we don't submit work that would just be torn down.
    if state.lifecycle_cancel.is_cancelled() {
        info!("Dispatch aborted before send — sandbox is shutting down");
        return;
    }

    // Send to agent lifecycle task — it handles injection vs new session.
    // Ack is deferred until the agent confirms it processed the message.
    let ack_rx = state.agent_handle.send(
        text,
        model,
        agent_type,
        agent_mode,
        resume_session_id,
        start_fresh,
    );

    if !uuids.is_empty() {
        let redelivery_state = state.clone();
        spawn_supervised("dispatch-ack", async move {
            // Held from the fetch above until the ack below prunes the rows.
            // Released before any re-dispatch, which re-acquires the same lock.
            let dispatch_guard = dispatch_guard;
            // Bound the wait for harness acceptance against shutdown, so a
            // stopping pod that never reports acceptance does not pin the guard.
            let ack = tokio::select! {
                result = ack_rx => result,
                () = redelivery_state.lifecycle_cancel.cancelled() => {
                    info!("Dispatch ack wait aborted — sandbox is shutting down");
                    drop(dispatch_guard);
                    return;
                }
            };
            let Ok(ack) = ack else {
                warn!("Agent ack channel dropped — messages not acknowledged");
                drop(dispatch_guard);
                schedule_redispatch(redelivery_state).await;
                return;
            };
            // The harness has already accepted this Input, so the Message must
            // never be resubmitted — only the ack RPC may still fail. Retry it
            // until it lands or the pod stops (a failed ack is never treated as
            // success), holding the guard throughout so a waiting dispatcher
            // cannot re-fetch and re-run the already-accepted Message.
            let acked =
                acknowledge_until_settled(&redelivery_state, uuids, &ack.session_id, &ack.run_id)
                    .await;
            drop(dispatch_guard);
            // Messages held back by the start_fresh split are now the only
            // pending ones — re-dispatch them, but only once this batch is
            // durably acked and not while the pod is shutting down.
            if acked && deferred > 0 {
                schedule_dispatch(redelivery_state).await;
            }
        });
    }
}

/// How long to wait before re-fetching pending messages after a dropped ack —
/// enough for a follow-up agent start attempt to succeed (and ack) or fail
/// definitively, without hammering `GetPendingMessages` in a tight loop.
const REDISPATCH_BACKOFF: Duration = Duration::from_secs(15);

/// Post-acceptance acknowledgement retry backoff: exponential from the base,
/// capped at the ceiling, plus per-attempt jitter seeded by `task_id` so a
/// fleet of pods riding out the same backend/ack outage does not retry in
/// lockstep. The harness has already accepted the Input (or the prompt has been
/// answered), so the batch is never resubmitted — only the ack is re-attempted,
/// retried until it lands or the pod stops (a failed ack is never treated as
/// success), so a waiter can never re-run the accepted Message during an outage.
const ACK_RETRY_BASE_BACKOFF: Duration = Duration::from_secs(2);
const ACK_RETRY_MAX_BACKOFF: Duration = Duration::from_mins(1);

/// Compute the capped exponential backoff for a 1-based retry `attempt`, plus up
/// to 25% jitter derived from `task_id` + `attempt` (no `rand` dependency; the
/// `task_id` seed decorrelates retries across the fleet).
fn ack_retry_delay(task_id: &str, attempt: u32) -> Duration {
    use std::hash::{Hash, Hasher};
    // Cap the exponent so the shift can't overflow; 2^5 * base already saturates
    // past the 60s ceiling.
    let exp = (attempt - 1).min(5);
    let backoff = ACK_RETRY_BASE_BACKOFF
        .saturating_mul(1u32 << exp)
        .min(ACK_RETRY_MAX_BACKOFF);
    let jitter_span_ms = (backoff.as_millis() / 4) as u64;
    if jitter_span_ms == 0 {
        return backoff;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    task_id.hash(&mut hasher);
    attempt.hash(&mut hasher);
    backoff + Duration::from_millis(hasher.finish() % (jitter_span_ms + 1))
}

/// Warn on the first ack failure and then only every 10th attempt, so a
/// prolonged outage does not flood the log with a line per retry.
const ACK_RETRY_WARN_EVERY: u32 = 10;

/// Skip re-dispatch once the agent has accumulated this many consecutive
/// errors — mirrors the coordinator service's `MAX_CONSECUTIVE_ERRORS` circuit
/// breaker so a persistently failing agent can't loop dispatch → drop →
/// re-dispatch forever.
const MAX_REDISPATCH_CONSECUTIVE_ERRORS: u32 = 3;

/// Re-fetch and re-dispatch pending messages after the agent dropped an ack.
///
/// A dropped ack means the agent died before `SessionInit`, so the messages
/// were never acknowledged (`agent_session_id` stays NULL on the backend).
/// Without this, nothing re-delivers them until the next sandbox startup —
/// in production that meant a ~10 minute gap. Bounded: each re-dispatch only
/// schedules another retry if its own ack drops again, and the consecutive-
/// error gate stops the cycle once the agent is persistently failing.
///
/// Safe against double-delivery: the backend only returns messages that are
/// still unacknowledged, so if a later attempt acked them this fetch is empty.
///
/// Not an `async fn`: the recursion (`dispatch_pending_messages` → ack task →
/// here → `dispatch_pending_messages`) must go through a type-erased boxed
/// future, otherwise the compiler cannot resolve the futures' `Send` bounds
/// through the cycle.
fn schedule_redispatch(
    state: SandboxState,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        let consecutive_errors = state.agent.consecutive_errors().await;
        if consecutive_errors >= MAX_REDISPATCH_CONSECUTIVE_ERRORS {
            warn!(
                consecutive_errors,
                "Skipping pending-message re-dispatch — agent circuit breaker active"
            );
            return;
        }
        tokio::time::sleep(REDISPATCH_BACKOFF).await;
        dispatch_pending_messages(state).await;
    })
}

/// Resolve a pending interactive prompt with the first pending message.
///
/// Runs under the dispatch guard held by the caller, so the prompt Message is
/// answered and acknowledged before any concurrent dispatcher can re-fetch it.
async fn resolve_pending_prompt(state: &SandboxState) {
    let task_id = &state.config.task_id;

    let deliveries = match state.backend_client.get_pending_messages(task_id).await {
        Ok(deliveries) => deliveries,
        Err(e) => {
            warn!(error = %e, "Failed to fetch pending messages for prompt resolution");
            return;
        }
    };

    let Some(prompt_delivery) = deliveries.first() else {
        return;
    };
    let prompt_response_uuids = prompt_delivery.message_uuids.clone();
    let has_remaining_messages = deliveries.len() > 1;

    // The answer joins the run that asked the question, so it is acknowledged
    // against that run's identity. Resolve it *before* consuming the prompt:
    // without it the answer cannot be acknowledged, and answering-then-failing-
    // to-ack would leave the row pending to be re-fetched later as ordinary
    // input (with no prompt outstanding). A pending prompt implies an active
    // run, so this is present in practice; if it is somehow absent, leave the
    // prompt and message pending for a later pass rather than answering
    // unacknowledgeably.
    let (Some(session_id), Some(run_id)) = (
        state.agent.last_session_id().await,
        state.agent.current_run_id().await,
    ) else {
        warn!(
            task_id = %task_id,
            "Cannot resolve prompt without an active run identity — leaving it pending"
        );
        return;
    };

    let Some(prompt) = state.agent.take_pending_prompt().await else {
        // The prompt was consumed elsewhere between the guard's check and here,
        // so this is no longer a prompt answer. Re-dispatch the messages through
        // the normal acked path (spawned so it re-acquires the guard we still
        // hold) rather than submitting unacknowledged input.
        info!("Pending prompt was already consumed, dispatching normally");
        spawn_supervised("prompt-redispatch", schedule_dispatch(state.clone()));
        return;
    };

    let text = prompt_delivery.prompt_text.clone();

    info!(tool = ?prompt.tool_name, "Resolving interactive prompt with pending message");

    let response = prompt_response(
        &prompt.tool_name,
        prompt_delivery.prompt_response.as_ref(),
        text,
    );

    if prompt.sender.send(response).is_err() {
        warn!("Pending prompt receiver dropped before response was delivered");
        return;
    }

    if !prompt_response_uuids.is_empty() {
        // Same post-acceptance ack policy as ordinary messages: retry until the
        // ack lands or the pod stops, never converting the answered prompt into
        // a re-run or an ordinary Agent input.
        acknowledge_until_settled(state, prompt_response_uuids, &session_id, &run_id).await;
    }

    if has_remaining_messages {
        spawn_supervised(
            "prompt-remaining-redispatch",
            schedule_dispatch(state.clone()),
        );
    }
}

/// What the parked call is told.
///
/// A delivery that carries a structured answer is one, and the answer is used as
/// it stands: an answer per question, an approval, or a rejection with its
/// feedback. Nothing is inferred from the words.
///
/// A delivery that carries none is an ordinary message that arrived while the
/// prompt was parked. A question takes it as prose, which is how someone
/// answering in the chat box answers, and it travels as the freeform response
/// rather than as an answer to any one question: the CLI keys answers on each
/// question's own text and drops a question it finds none for, so prose under
/// any other key would leave the model told that nobody answered. A proposed
/// plan does **not**: an approval is a specific thing to say, and reading a
/// passing remark as one would let the agent act on a plan nobody approved. It
/// is delivered as a rejection carrying what was said, which is the same thing
/// the agent would have been told had the person typed it into the reject box.
fn prompt_response(
    tool_name: &PromptToolName,
    answer: Option<&sandbox_proto::PromptResponse>,
    text: String,
) -> PromptResponse {
    match (tool_name, answer) {
        (PromptToolName::AskUserQuestion, Some(answer)) => PromptResponse::AskUserAnswer {
            answers: serde_json::json!(answer.answers),
        },
        (PromptToolName::AskUserQuestion, None) => {
            PromptResponse::AskUserResponse { response: text }
        }
        (PromptToolName::ExitPlanMode, Some(answer)) if answer.approved => {
            PromptResponse::PlanApproved
        }
        (PromptToolName::ExitPlanMode, Some(answer)) => PromptResponse::PlanRejected {
            feedback: answer.feedback.clone().unwrap_or_default(),
        },
        (PromptToolName::ExitPlanMode, None) => PromptResponse::PlanRejected { feedback: text },
    }
}

fn schedule_dispatch(
    state: SandboxState,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        dispatch_pending_messages(state).await;
    })
}

/// Acknowledge accepted messages, retrying until the RPC lands or the sandbox
/// shuts down. The harness has already accepted the Input (ordinary message) or
/// the prompt has already been answered, so the batch is never resubmitted —
/// only the ack is re-attempted. The caller holds the dispatch guard across the
/// whole retry, so a waiting dispatcher cannot re-fetch and re-run the accepted
/// Message (or re-submit an answered prompt as ordinary input) during a
/// transient ack outage; and because a failed ack is never treated as success,
/// the row is never dropped back to "pending" for a later notification to re-run.
///
/// Each RPC and each backoff is raced against lifecycle cancellation. Returns
/// `true` once the ack lands; `false` only when cancellation interrupted it —
/// the pod is stopping, so the still-pending row is left for the replacement
/// pod's post-restore dispatch (the existing cross-pod at-least-once path).
async fn acknowledge_until_settled(
    state: &SandboxState,
    uuids: Vec<String>,
    session_id: &str,
    run_id: &str,
) -> bool {
    if uuids.is_empty() {
        return true;
    }
    let mut attempt = 1u32;
    loop {
        let ack = tokio::select! {
            result = state.backend_client.acknowledge_messages(
                &state.config.task_id,
                uuids.clone(),
                session_id,
                run_id,
            ) => result,
            () = state.lifecycle_cancel.cancelled() => {
                info!("Acknowledgement aborted mid-RPC — sandbox is shutting down");
                return false;
            }
        };
        match ack {
            Ok(()) => return true,
            Err(e) => {
                // Rate-limit the warnings so a long outage doesn't flood the log.
                if attempt == 1 || attempt.is_multiple_of(ACK_RETRY_WARN_EVERY) {
                    warn!(
                        error = %e,
                        attempt,
                        "Failed to acknowledge messages, retrying until acked or shutdown"
                    );
                }
                let delay = ack_retry_delay(&state.config.task_id, attempt);
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    () = state.lifecycle_cancel.cancelled() => {
                        info!("Acknowledgement retry aborted — sandbox is shutting down");
                        return false;
                    }
                }
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(
        answers: &[(&str, &str)],
        approved: bool,
        feedback: Option<&str>,
    ) -> sandbox_proto::PromptResponse {
        sandbox_proto::PromptResponse {
            tool_use_id: "tu-1".to_string(),
            tool_name: "AskUserQuestion".to_string(),
            answers: answers
                .iter()
                .map(|(q, a)| ((*q).to_string(), (*a).to_string()))
                .collect(),
            approved,
            feedback: feedback.map(str::to_string),
        }
    }

    /// The answer is used as it stands: one per question, keyed the way the CLI
    /// looks them up.
    #[test]
    fn a_structured_answer_reaches_the_question_unchanged() {
        let response = prompt_response(
            &PromptToolName::AskUserQuestion,
            Some(&answer(&[("Which store?", "Postgres")], false, None)),
            "ignored".to_string(),
        );
        let PromptResponse::AskUserAnswer { answers } = response else {
            panic!("a question takes answers: {response:?}");
        };
        assert_eq!(answers["Which store?"], "Postgres");
    }

    /// Someone answering in the chat box while a question is parked is
    /// answering it — as the freeform response, which belongs to the call rather
    /// than to any one question. An answer keyed on anything but a question's
    /// own text reaches the model as no answer at all.
    #[test]
    fn prose_answers_a_parked_question() {
        let response = prompt_response(
            &PromptToolName::AskUserQuestion,
            None,
            "use Postgres".to_string(),
        );
        let PromptResponse::AskUserResponse { response } = response else {
            panic!("prose is a freeform response: {response:?}");
        };
        assert_eq!(response, "use Postgres");
    }

    #[test]
    fn a_verdict_approves_or_rejects_by_what_it_says() {
        assert!(matches!(
            prompt_response(
                &PromptToolName::ExitPlanMode,
                Some(&answer(&[], true, None)),
                String::new()
            ),
            PromptResponse::PlanApproved
        ));

        let rejected = prompt_response(
            &PromptToolName::ExitPlanMode,
            Some(&answer(&[], false, Some("use the second approach"))),
            String::new(),
        );
        let PromptResponse::PlanRejected { feedback } = rejected else {
            panic!("a verdict of no is a rejection: {rejected:?}");
        };
        assert_eq!(feedback, "use the second approach");
    }

    /// An approval is a specific thing to say. A message that merely arrived
    /// while a plan was parked is not one, however it reads — the agent must
    /// never act on a plan nobody approved.
    #[test]
    fn a_passing_remark_never_approves_a_plan() {
        for text in ["yes", "sounds good", "go ahead", "approve"] {
            let response = prompt_response(&PromptToolName::ExitPlanMode, None, text.to_string());
            let PromptResponse::PlanRejected { feedback } = response else {
                panic!("{text} must not approve a plan: {response:?}");
            };
            assert_eq!(feedback, text, "what was said reaches the agent");
        }
    }
}
