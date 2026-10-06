//! Message dispatcher — routes incoming message notifications.
//!
//! Invoked from `grpc_server::execute_agent()`,
//! `grpc_server::notify_messages()`, and `init::run_init()` after
//! `InitPhase::Ready`.

use std::sync::Arc;
use std::time::Duration;

use tracing::{error, info, warn};

use crate::files::uploads::fetch_uploads_for_messages;
use crate::state::{SandboxState, TaskContext};
use crate::writer_admission::WriterPermit;

use super::prompt::{PromptResponse, PromptToolName};
use super::types::AgentStatus;

/// Spawn a detached task whose panic is logged rather than swallowed by a bare
/// `tokio::spawn`.
///
/// Not registered with the root supervisor: a dispatcher must outlive normal
/// teardown, holding the single-flight guard until the ack lands, and stops
/// only via `lifecycle_cancel`.
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
/// Resolves the pending prompt if the agent is waiting for user input
/// (AskUserQuestion/ExitPlanMode); otherwise sends text to the agent lifecycle
/// task.
pub async fn dispatch_pending_messages(state: SandboxState) {
    let Some(admitted) = context_for(&state, None).await else {
        return;
    };
    dispatch_pending_messages_for_context(state, admitted.context, Some(admitted.permit)).await;
}

pub async fn dispatch_pending_messages_for_task(state: SandboxState, task_id: String) {
    let Some(admitted) = context_for(&state, Some(&task_id)).await else {
        return;
    };
    dispatch_pending_messages_for_context(state, admitted.context, Some(admitted.permit)).await;
}

/// A task context, and the admission of the turn about to be opened against it.
struct AdmittedContext {
    context: Arc<TaskContext>,
    /// Held until the harness accepts the turn, not until it is enqueued:
    /// teardown's drain must not complete before the turn reaches the agent its
    /// stop pass then has to cancel.
    permit: WriterPermit,
}

/// The context this pod's work runs against, if a turn may still be opened
/// against it. `None` names the sole registered task.
///
/// Every producer of a turn resolves its context here — the two RPCs, a
/// scheduled job's injector, a wake note, `run_init` — so this is where
/// teardown's closed admission reaches all of them.
async fn context_for(state: &SandboxState, task_id: Option<&str>) -> Option<AdmittedContext> {
    let Some(permit) = state.writer_admission.admit() else {
        info!(?task_id, "Dispatch skipped: the sandbox is shutting down");
        return None;
    };
    match state.task_context(task_id).await {
        Ok(context) => Some(AdmittedContext { context, permit }),
        Err(e) => {
            warn!(error = %e, ?task_id, "Task context unavailable; nothing was delivered");
            None
        }
    }
}

/// Queue `text` as this pod's own message for `task_id`, then open the turn.
///
/// The queued row is what `find_interrupted_turn` selects on, so a pod dying
/// mid-turn is recoverable; a direct-send fallback is not. Pass
/// `idempotency_key` when a caller can resend the same text, or the second send
/// is swallowed.
pub(crate) async fn queue_and_dispatch(
    state: &SandboxState,
    task_id: &str,
    text: String,
    idempotency_key: Option<&str>,
) {
    match state
        .backend_client
        .enqueue_sandbox_message(task_id, &text, idempotency_key)
        .await
    {
        // Dispatching on a refusal would claim a row that nothing wrote.
        Ok(None) => {
            info!(
                task_id,
                "The backend refused the message; nothing to dispatch"
            );
        }
        Ok(Some(uuid)) => {
            info!(task_id, "Queued this pod's own message");
            dispatch_self_queued(state.clone(), task_id.to_string(), uuid).await;
        }
        Err(error) => {
            warn!(task_id, %error, "Falling back to a direct send; the turn it opens is not recoverable");
            send_directly(state, task_id, text).await;
        }
    }
}

/// Hand `text` straight to the agent, opening a turn no `user` row backs. Only
/// [`queue_and_dispatch`] calls this, when the backend refused the queue.
async fn send_directly(state: &SandboxState, task_id: &str, text: String) {
    let Some(admitted) = context_for(state, Some(task_id)).await else {
        return;
    };
    // Nothing to ack here, but the permit still waits on acceptance: the
    // lifecycle task takes this command off an unbounded channel, so the turn
    // starts after the send returns.
    let ack_rx = admitted
        .context
        .agent_handle
        .send(text, None, None, None, None, None, false, None, false);
    hold_until_accepted(state.clone(), admitted.permit, ack_rx);
}

/// Hold `permit` until the harness accepts the turn, or the pod stops.
///
/// [`crate::agent::handle::AgentHandle::send`] only enqueues, so a permit
/// released when it returns lets teardown's drain finish ahead of the turn — one
/// the stop pass has already gone past.
fn hold_until_accepted(
    state: SandboxState,
    permit: WriterPermit,
    ack_rx: tokio::sync::oneshot::Receiver<crate::agent::handle::RunAck>,
) {
    spawn_supervised("dispatch-admission", async move {
        let _permit = permit;
        tokio::select! {
            _ = ack_rx => {}
            () = state.lifecycle_cancel.cancelled() => {}
        }
    });
}

/// Dispatch a row this pod queued for itself (`uuid` from
/// `enqueue_sandbox_message`).
///
/// Recorded via [`TaskContext::mark_self_queued`] before dispatch, so a parked
/// `AskUserQuestion`/`ExitPlanMode` never consumes it as the person's answer;
/// `resolve_pending_prompt` leaves it pending and opens its own turn instead.
pub(crate) async fn dispatch_self_queued(state: SandboxState, task_id: String, uuid: String) {
    let Some(admitted) = context_for(&state, Some(&task_id)).await else {
        return;
    };
    admitted.context.mark_self_queued(uuid).await;
    dispatch_pending_messages_for_context(state, admitted.context, Some(admitted.permit)).await;
}

/// `permit` is `None` where the caller waited before coming back here — a
/// redispatch, or a prompt the agent consumed — and those admit themselves
/// below rather than carrying an admission granted before the wait.
async fn dispatch_pending_messages_for_context(
    state: SandboxState,
    context: std::sync::Arc<TaskContext>,
    permit: Option<WriterPermit>,
) {
    let Some(permit) = permit.or_else(|| state.writer_admission.admit()) else {
        info!("Dispatch skipped: the sandbox is shutting down");
        return;
    };
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
        guard = context.dispatch_lock.clone().lock_owned() => guard,
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
    if context.agent.has_pending_prompt().await {
        resolve_pending_prompt(&state, &context).await;
        return;
    }

    // Fetch pending messages from backend
    let task_id = &context.task_id;
    let Some(deliveries) = fetch_or_redispatch(&state, &context).await else {
        return;
    };

    let mut deliveries = deliveries.into_iter();
    let Some(delivery) = deliveries.next() else {
        return;
    };
    let deferred = deliveries.count();

    // Pre-fetch any attached uploaded files before invoking the agent so the
    // on-disk paths advertised in the prompt are valid by the time the agent
    // reads them. A fetch that fails does not block dispatch — the rest of the
    // message is still worth delivering — but it does get named in the prompt
    // below, because the "Saved to:" line for that file is now a lie.
    let file_ids = delivery.file_ids.clone();
    let failed_uploads = if file_ids.is_empty() {
        Vec::new()
    } else {
        fetch_uploads_for_messages(
            &state.backend_client,
            task_id,
            file_ids,
            &state.config.upload_dir,
        )
        .await
    };

    let uuids = delivery.message_uuids.clone();
    let text = delivery.prompt_text;
    let model = delivery.model;
    let agent_profile = delivery.agent_profile;
    let agent_mode = delivery.agent_mode;
    let effort = delivery.effort;
    let resume_session_id = delivery.resume_session_id;
    let start_fresh = delivery.start_fresh;
    let switch_harness = delivery.switch_harness;
    let fork = delivery.fork.map(|fork| crate::agent::types::ForkSource {
        source_session_id: fork.source_session_id,
        source_message_uuid: fork.source_message_uuid,
        source_agent_turn_id: fork.source_agent_turn_id,
    });
    let skills = delivery.skills;

    // A delivery with neither text nor skills has nothing to open a turn with.
    // Skills alone are enough: they frontload into the prompt below, so a start
    // request that carries only skills still runs.
    if text.is_empty() && skills.is_empty() {
        return;
    }

    // A skill an activation record marked to frontload leads every fresh
    // session on top of whatever the turn asked for; a resumed one still
    // carries it, which is the rule the backend already applies to a task's own
    // skills. Applied after the gate above, so a standing skill cannot open a
    // turn on a delivery that has nothing in it.
    let skills = if resume_session_id.is_some() {
        skills
    } else {
        crate::agent::skills::with_always_frontloaded(&state.config, skills)
    };

    // Frontloading turns the requested skills into the prompt's leading preamble.
    let mut text = crate::agent::skills::frontload_skills(&state.config, &skills, &text).await;

    // Every requested skill may have failed to resolve, leaving a skills-only
    // delivery with an empty prompt: nothing to send, so don't open a turn.
    if text.is_empty() {
        return;
    }

    // After both emptiness gates, so a note about a file nobody will read cannot
    // be the whole turn; and last in the text, so it sits after the attachment
    // lines it contradicts.
    if let Some(note) = crate::files::uploads::failed_upload_note(&failed_uploads) {
        text.push_str(&note);
    }

    info!(
        message_count = uuids.len(),
        deferred_count = deferred,
        text_len = text.len(),
        skill_count = skills.len(),
        model = ?model,
        agent_profile = ?agent_profile,
        agent_mode = ?agent_mode,
        effort = ?effort,
        resume_session_id = ?resume_session_id,
        start_fresh,
        switch_harness,
        forked_from = ?fork.as_ref().map(|f| &f.source_session_id),
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
    let ack_rx = context.agent_handle.send(
        text,
        model,
        agent_profile,
        agent_mode,
        effort,
        resume_session_id,
        start_fresh,
        fork,
        switch_harness,
    );

    if uuids.is_empty() {
        // No rows to ack, but the same turn: the admission still has to outlive
        // the enqueue.
        hold_until_accepted(state, permit, ack_rx);
    } else {
        let redelivery_state = state.clone();
        let redelivery_context = std::sync::Arc::clone(&context);
        spawn_supervised("dispatch-ack", async move {
            // Held from the fetch above until the ack below prunes the rows.
            // Released before any re-dispatch, which re-acquires the same lock.
            let dispatch_guard = dispatch_guard;
            // Held until the harness accepts the turn: `send` only enqueues it,
            // so an admission released here would let teardown's drain finish
            // ahead of a turn its stop pass has already gone past.
            let permit = permit;
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
                drop(permit);
                drop(dispatch_guard);
                schedule_redispatch(redelivery_state, redelivery_context).await;
                return;
            };
            drop(permit);
            // Taken as ordinary input, which is what a self-queued row is here
            // for. Marked until acceptance and not before: every path above
            // leaves the row pending, and one that came back unmarked could
            // answer a prompt the agent parked in the meantime.
            redelivery_context.forget_self_queued(&uuids).await;
            // The harness has already accepted this Input, so the Message must
            // never be resubmitted — only the ack RPC may still fail. Retry it
            // until it lands or the pod stops (a failed ack is never treated as
            // success), holding the guard throughout so a waiting dispatcher
            // cannot re-fetch and re-run the already-accepted Message.
            let acked = acknowledge_until_settled(
                &redelivery_state,
                &redelivery_context,
                uuids,
                &ack.session_id,
                &ack.run_id,
                None,
            )
            .await;
            drop(dispatch_guard);
            // Messages held back by the start_fresh split are now the only
            // pending ones — re-dispatch them, but only once this batch is
            // durably acked and not while the pod is shutting down.
            if acked && deferred > 0 {
                schedule_dispatch(redelivery_state, redelivery_context).await;
            }
        });
    }
}

/// How long to wait before re-fetching pending messages after a dropped ack —
/// enough for a follow-up agent start attempt to succeed (and ack) or fail
/// definitively, without hammering `GetPendingMessages` in a tight loop.
const REDISPATCH_BACKOFF: Duration = Duration::from_secs(15);

/// Post-acceptance ack retry backoff: exponential from the base, capped at the
/// ceiling, with jitter seeded by `task_id` so a fleet doesn't retry in
/// lockstep during a shared outage.
const ACK_RETRY_BASE_BACKOFF: Duration = Duration::from_secs(2);
const ACK_RETRY_MAX_BACKOFF: Duration = Duration::from_mins(1);

/// Compute the capped exponential backoff for a 1-based retry `attempt`, plus
/// up to 25% jitter derived from `task_id` + `attempt` (no `rand` dependency;
/// the `task_id` seed decorrelates retries across the fleet).
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
/// errors.
///
/// Mirrors the coordinator service's `MAX_CONSECUTIVE_ERRORS` circuit breaker,
/// so a persistently failing agent can't loop dispatch -> drop -> re-dispatch
/// forever.
const MAX_REDISPATCH_CONSECUTIVE_ERRORS: u32 = 3;

/// Re-fetch and re-dispatch messages after the agent drops an ack — it died
/// before `SessionInit`, so the backend never acknowledged them.
///
/// Retries only if the new ack also drops, and stops once the consecutive-error
/// gate trips; boxed, not `async fn`, since the recursion needs type erasure
/// for `Send`.
fn schedule_redispatch(
    state: SandboxState,
    context: std::sync::Arc<TaskContext>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        if matches!(context.agent.status().await, AgentStatus::Stopped) {
            info!("Skipping pending-message re-dispatch — agent is stopped");
            return;
        }
        let consecutive_errors = context.agent.consecutive_errors().await;
        if consecutive_errors >= MAX_REDISPATCH_CONSECUTIVE_ERRORS {
            warn!(
                task_id = %context.task_id,
                consecutive_errors,
                "Skipping pending-message re-dispatch — agent circuit breaker active"
            );
            return;
        }
        tokio::time::sleep(REDISPATCH_BACKOFF).await;
        if matches!(context.agent.status().await, AgentStatus::Stopped) {
            info!("Skipping pending-message re-dispatch after backoff — agent is stopped");
            return;
        }
        dispatch_pending_messages_for_context(state, context, None).await;
    })
}

/// The Task's pending deliveries, or `None` after a failed read, which
/// schedules a redispatch: the Messages stay pending and no notification
/// repeats, so a transient backend failure would strand them until the next.
async fn fetch_or_redispatch(
    state: &SandboxState,
    context: &Arc<TaskContext>,
) -> Option<Vec<sandbox_proto::PendingAgentDelivery>> {
    let hold_queued = context.agent.status().await.in_turn();
    match state
        .backend_client
        .get_deliverable_messages(&context.task_id, hold_queued)
        .await
    {
        Ok(deliveries) => {
            if hold_queued {
                dispatch_after_turn(state, context).await;
            }
            Some(deliveries)
        }
        Err(e) => {
            warn!(error = %e, "Failed to fetch pending messages; retrying after a backoff");
            tokio::spawn(schedule_redispatch(state.clone(), Arc::clone(context)));
            None
        }
    }
}

/// Fetch again once the open turn ends, for the `queue` messages a mid-turn
/// fetch may have left pending. One waiter per turn; an empty fetch is a no-op.
async fn dispatch_after_turn(state: &SandboxState, context: &Arc<TaskContext>) {
    if context.agent.set_messages_pending().await {
        return;
    }
    spawn_supervised(
        "queued-dispatch",
        dispatch_when_turn_ends(state.clone(), Arc::clone(context)),
    );
}

/// Boxed, as [`schedule_redispatch`] is: it re-enters the dispatch cycle that
/// spawns it, and an unboxed recursive future cannot be proven `Send`.
fn dispatch_when_turn_ends(
    state: SandboxState,
    context: Arc<TaskContext>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        tokio::select! {
            () = context.agent.turn_ended() => {}
            () = state.lifecycle_cancel.cancelled() => return,
        }
        context.agent.take_messages_pending().await;
        // A stop is a person's "no more turns", and an erroring agent is left to
        // `schedule_redispatch`'s error cap; the rows stay pending for the next send.
        if matches!(context.agent.status().await, AgentStatus::Stopped)
            || context.agent.consecutive_errors().await >= MAX_REDISPATCH_CONSECUTIVE_ERRORS
        {
            return;
        }
        dispatch_pending_messages_for_context(state, context, None).await;
    })
}

/// Resolve a pending interactive prompt with the first pending message.
///
/// Runs under the dispatch guard held by the caller, so the prompt Message is
/// answered and acknowledged before any concurrent dispatcher can re-fetch it.
async fn resolve_pending_prompt(state: &SandboxState, context: &std::sync::Arc<TaskContext>) {
    let task_id = &context.task_id;

    // A failed read takes nothing: the prompt stays parked for the redispatch.
    let Some(deliveries) = fetch_or_redispatch(state, context).await else {
        return;
    };

    let Some(prompt_delivery) = deliveries.first() else {
        return;
    };

    // A row this pod queued for itself is a wake and not an answer — see
    // [`dispatch_self_queued`]. `build_pending_deliveries` coalesces every pending
    // row into this one delivery, so a wake arrives alongside the person's answer
    // and has to be withheld row by row: it is left unacknowledged, and the
    // redispatch below opens the turn it was owed. Answering with the delivery
    // whole would ack the wake as part of the answer and lose it.
    let (wakes, prompt_response_uuids) = context
        .split_self_queued(&prompt_delivery.message_uuids)
        .await;
    if prompt_response_uuids.is_empty() && !wakes.is_empty() {
        info!("Only this pod's own wakes are pending; leaving the prompt parked");
        return;
    }
    let has_remaining_messages = deliveries.len() > 1 || !wakes.is_empty();

    // The answer joins the run that asked the question, so it is acknowledged
    // against that run's identity. Resolve it *before* consuming the prompt:
    // without it the answer cannot be acknowledged, and answering-then-failing-
    // to-ack would leave the row pending to be re-fetched later as ordinary
    // input (with no prompt outstanding). A pending prompt implies an active
    // run, so this is present in practice; if it is somehow absent, leave the
    // prompt and message pending for a later pass rather than answering
    // unacknowledgeably.
    let (Some(session_id), Some(run_id)) = (
        context.agent.last_session_id().await,
        context.agent.current_run_id().await,
    ) else {
        warn!(
            task_id = %task_id,
            "Cannot resolve prompt without an active run identity — leaving it pending"
        );
        return;
    };

    let Some(prompt) = context.agent.take_pending_prompt().await else {
        // The prompt was consumed elsewhere between the guard's check and here,
        // so this is no longer a prompt answer. Re-dispatch the messages through
        // the normal acked path (spawned so it re-acquires the guard we still
        // hold) rather than submitting unacknowledged input.
        info!("Pending prompt was already consumed, dispatching normally");
        spawn_supervised(
            "prompt-redispatch",
            schedule_dispatch(state.clone(), std::sync::Arc::clone(context)),
        );
        return;
    };

    // A withheld wake's uuid is held back but its text is not: the rows were joined
    // with `\n` before the delivery left the backend. See `docs/deferred-changes.md`,
    // "A withheld wake's text still rides in the answer that carried it".
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
        // A structured answer came through the panel, which has already settled
        // the call it names. An ordinary message settles nothing on its way in,
        // so the ack names the call it answered and the backend settles it —
        // otherwise the panel keeps asking a question that has been answered.
        let answered_prompt = prompt_delivery
            .prompt_response
            .is_none()
            .then_some(prompt.tool_use_id.as_str());
        // Same post-acceptance ack policy as ordinary messages: retry until the
        // ack lands or the pod stops, never converting the answered prompt into
        // a re-run or an ordinary Agent input.
        acknowledge_until_settled(
            state,
            context,
            prompt_response_uuids,
            &session_id,
            &run_id,
            answered_prompt,
        )
        .await;
    }

    if has_remaining_messages {
        spawn_supervised(
            "prompt-remaining-redispatch",
            schedule_dispatch(state.clone(), std::sync::Arc::clone(context)),
        );
    }
}

/// What the parked call is told.
///
/// A structured answer (from the panel) is used as-is; plain text answers a
/// question as its freeform response, but for a plan it is never read as
/// approval — a passing remark becomes a rejection carrying what was said.
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
    context: std::sync::Arc<TaskContext>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        dispatch_pending_messages_for_context(state, context, None).await;
    })
}

/// Acknowledge accepted messages, retrying until the RPC lands or the sandbox
/// shuts down; only the ack retries since the Input was already accepted.
///
/// Returns `true` once acked; `false` only on shutdown, leaving the row for the
/// replacement pod's post-restore dispatch.
async fn acknowledge_until_settled(
    state: &SandboxState,
    context: &TaskContext,
    uuids: Vec<String>,
    session_id: &str,
    run_id: &str,
    answered_prompt: Option<&str>,
) -> bool {
    if uuids.is_empty() {
        return true;
    }
    let mut attempt = 1u32;
    loop {
        let ack = tokio::select! {
            result = state.backend_client.acknowledge_messages(
                &context.task_id,
                uuids.clone(),
                session_id,
                run_id,
                answered_prompt,
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
                let delay = ack_retry_delay(&context.task_id, attempt);
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

    /// A turn is enqueued onto an unbounded channel, so the send returns before
    /// the lifecycle has taken it. An admission released there lets the drain
    /// finish, and the stop pass then runs against a turn that has not started.
    #[tokio::test]
    async fn an_enqueued_turn_holds_the_drain_open_until_the_harness_accepts_it() {
        let state = crate::state::test_support::state_for_test();
        let permit = state.writer_admission.admit().expect("open");
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        hold_until_accepted(state.clone(), permit, ack_rx);

        let draining = tokio::spawn({
            let admission = Arc::clone(&state.writer_admission);
            async move { admission.close_and_drain(Duration::from_secs(30)).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !draining.is_finished(),
            "the drain finished while a turn was still only enqueued"
        );

        let _ = ack_tx.send(crate::agent::handle::RunAck {
            session_id: "session-1".to_string(),
            run_id: "run-1".to_string(),
        });
        assert_eq!(
            draining.await.expect("join"),
            crate::writer_admission::Drain::Complete
        );
    }

    /// A read that fails while a question is parked leaves the question parked
    /// and reads again after the backoff, as the ordinary dispatch does.
    #[tokio::test(start_paused = true)]
    async fn a_failed_read_during_a_parked_question_is_retried() {
        let backend = Arc::new(crate::state::test_support::NoopBackend {
            failing_fetches: 1,
            ..Default::default()
        });
        let state = crate::state::test_support::state_with(
            crate::state::test_support::state_for_test()
                .config
                .as_ref()
                .clone(),
            backend.clone(),
        );
        let context = context_for(&state, Some("task-1")).await.expect("context");
        let (sender, _answer) = tokio::sync::oneshot::channel();
        assert!(
            context
                .context
                .agent
                .set_pending_prompt(crate::agent::prompt::PendingPrompt {
                    tool_name: PromptToolName::AskUserQuestion,
                    tool_use_id: "tu-1".to_string(),
                    input: serde_json::json!({}),
                    sender,
                })
                .await
        );

        dispatch_pending_messages_for_context(
            state.clone(),
            Arc::clone(&context.context),
            Some(context.permit),
        )
        .await;
        assert_eq!(backend.fetches.load(std::sync::atomic::Ordering::SeqCst), 1);

        tokio::time::sleep(REDISPATCH_BACKOFF + Duration::from_secs(1)).await;
        assert_eq!(
            backend.fetches.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the read is retried after the backoff"
        );
        assert!(context.context.agent.has_pending_prompt().await);
    }

    /// A stopped turn opens no queued turn, and a turn that ends normally does.
    #[tokio::test(start_paused = true)]
    async fn a_queued_message_waits_for_the_next_send_after_a_stop() {
        let backend = Arc::new(crate::state::test_support::NoopBackend::default());
        let state = crate::state::test_support::state_with(
            crate::state::test_support::state_for_test()
                .config
                .as_ref()
                .clone(),
            backend.clone(),
        );
        let context = context_for(&state, Some("task-1")).await.expect("context");
        let agent = &context.context.agent;
        let fetches = || backend.fetches.load(std::sync::atomic::Ordering::SeqCst);

        for (end_turn_with_stop, expected_fetches) in [(true, 0), (false, 1)] {
            agent
                .set_running(tokio_util::sync::CancellationToken::new())
                .await;
            let waiter = tokio::spawn(dispatch_when_turn_ends(
                state.clone(),
                Arc::clone(&context.context),
            ));
            tokio::time::sleep(Duration::from_millis(10)).await;
            if end_turn_with_stop {
                agent.set_stopped().await;
            } else {
                agent.set_idle().await;
            }
            waiter.await.expect("join");
            assert_eq!(fetches(), expected_fetches, "stop: {end_turn_with_stop}");
        }
    }

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
    /// answering it — as the freeform response, which belongs to the call
    /// rather than to any one question.
    ///
    /// An answer keyed on anything but a question's own text reaches the model
    /// as no answer at all.
    #[test]
    fn text_answers_a_parked_question() {
        let response = prompt_response(
            &PromptToolName::AskUserQuestion,
            None,
            "use Postgres".to_string(),
        );
        let PromptResponse::AskUserResponse { response } = response else {
            panic!("text is a freeform response: {response:?}");
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
