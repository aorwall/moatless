//! Message dispatcher — routes incoming message notifications.
//!
//! Invoked from:
//! - `grpc_server::execute_agent()` when backend requests execution
//! - `grpc_server::notify_messages()` when backend signals new messages
//! - `init::run_init()` after `InitPhase::Ready` is set
//!
//! The dispatcher fetches pending messages from the backend, sends text to
//! the agent actor via `agent_handle.send()`, and acknowledges the messages.
//! The agent handles whether to start a new session or inject mid-turn.
//!
//! It self-gates on [`SandboxState::is_ready`]: the two gRPC entry points fire
//! from the moment the pod starts, so a call that races `run_init` is dropped
//! (the messages stay pending and `run_init` dispatches them after restoration).

use std::time::Duration;

use tracing::{info, warn};

use crate::files::uploads::fetch_uploads_for_messages;
use crate::state::SandboxState;

use super::prompt::PromptResponse;
use super::types::{content_blocks_from_proto, ContentBlock};

/// Dispatch pending messages to the agent.
///
/// If the agent is waiting for user input (AskUserQuestion/ExitPlanMode),
/// resolves the pending prompt. Otherwise sends text to the agent actor.
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
    let messages = match state.backend_client.get_pending_messages(task_id).await {
        Ok(msgs) => msgs,
        Err(e) => {
            warn!(error = %e, "Failed to fetch pending messages");
            return;
        }
    };

    if messages.is_empty() {
        return;
    }

    // A `start_fresh` message means "new agent session", which coalescing into
    // a batch would silently discard: the batch derives one resume decision for
    // all of its messages. Cut the batch at the first such message — dispatch
    // whatever preceded it, then it alone on the next pass.
    let (messages, deferred) = split_at_start_fresh(messages);

    // Pre-fetch any attached uploaded files before invoking the agent so the
    // on-disk paths advertised in the prompt are valid by the time the agent
    // reads them. Best-effort: errors are logged but don't block dispatch.
    let file_ids: Vec<String> = messages
        .iter()
        .flat_map(|m| m.file_ids.iter().cloned())
        .collect();
    if !file_ids.is_empty() {
        fetch_uploads_for_messages(
            &state.backend_client,
            task_id,
            file_ids,
            &state.config.upload_dir,
        )
        .await;
    }

    // Extract UUIDs, text content, and resume_session_id
    let uuids: Vec<String> = messages.iter().filter_map(|m| m.uuid.clone()).collect();
    let text = extract_text_from_messages(&messages);
    let model = extract_model(&messages);
    let agent_type = extract_agent_type(&messages);
    let agent_mode = extract_agent_mode(&messages);
    let resume_session_id = messages.iter().find_map(|m| m.resume_session_id.clone());
    let start_fresh = messages.iter().any(|m| m.start_fresh.unwrap_or(false));

    if text.is_empty() {
        return;
    }

    info!(
        message_count = messages.len(),
        deferred_count = deferred,
        text_len = text.len(),
        model = ?model,
        agent_type = ?agent_type,
        agent_mode = ?agent_mode,
        resume_session_id = ?resume_session_id,
        start_fresh,
        "Dispatching messages to agent"
    );

    // Send to agent actor — it handles injection vs new session.
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
        tokio::spawn(async move {
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

/// Split a pending batch before the first `start_fresh` message.
///
/// Returns the messages to dispatch now plus how many were held back. A
/// `start_fresh` message at the head is dispatched alone; one further in cuts
/// the batch so the messages before it run as their own turn first.
fn split_at_start_fresh(
    mut messages: Vec<sandbox_proto::PendingMessage>,
) -> (Vec<sandbox_proto::PendingMessage>, usize) {
    let Some(idx) = messages
        .iter()
        .position(|m| m.start_fresh.unwrap_or(false))
    else {
        return (messages, 0);
    };
    let keep = if idx == 0 { 1 } else { idx };
    let deferred = messages.len() - keep;
    messages.truncate(keep);
    (messages, deferred)
}

/// How long to wait before re-fetching pending messages after a dropped ack —
/// enough for a follow-up agent start attempt to succeed (and ack) or fail
/// definitively, without hammering `GetPendingMessages` in a tight loop.
const REDISPATCH_BACKOFF: Duration = Duration::from_secs(15);

/// Backoff between post-acceptance acknowledgement retries. The harness has
/// already accepted the Input (or the prompt has already been answered), so the
/// batch is never resubmitted — only the ack is re-attempted, and it is retried
/// until it lands or the pod stops (a failed ack is never treated as success),
/// so a waiter can never re-run the accepted Message during a transient outage.
const ACK_RETRY_BACKOFF: Duration = Duration::from_secs(2);

/// Skip re-dispatch once the agent has accumulated this many consecutive
/// errors — mirrors the agent actor's `MAX_CONSECUTIVE_ERRORS` circuit
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

    let proto_messages = match state.backend_client.get_pending_messages(task_id).await {
        Ok(msgs) => msgs,
        Err(e) => {
            warn!(error = %e, "Failed to fetch pending messages for prompt resolution");
            return;
        }
    };

    let Some(prompt_message) = proto_messages.first() else {
        return;
    };
    let prompt_response_uuid = prompt_message.uuid.clone();
    let has_remaining_messages = proto_messages.len() > 1;

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
        tokio::spawn(schedule_dispatch(state.clone()));
        return;
    };

    let text = extract_text_from_messages(std::slice::from_ref(prompt_message));

    info!(tool = ?prompt.tool_name, "Resolving interactive prompt with pending message");

    let response = match prompt.tool_name {
        super::prompt::PromptToolName::AskUserQuestion => PromptResponse::AskUserAnswer {
            answers: serde_json::json!({ "text": text }),
        },
        super::prompt::PromptToolName::ExitPlanMode => {
            if text.to_lowercase().contains("reject") || text.to_lowercase().contains("no") {
                PromptResponse::PlanRejected { feedback: text }
            } else {
                PromptResponse::PlanApproved
            }
        }
    };

    if prompt.sender.send(response).is_err() {
        warn!("Pending prompt receiver dropped before response was delivered");
        return;
    }

    if let Some(uuid) = prompt_response_uuid {
        // Same post-acceptance ack policy as ordinary messages: retry until the
        // ack lands or the pod stops, never converting the answered prompt into
        // a re-run or an ordinary Agent input.
        acknowledge_until_settled(state, vec![uuid], &session_id, &run_id).await;
    }

    if has_remaining_messages {
        tokio::spawn(schedule_dispatch(state.clone()));
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
                warn!(
                    error = %e,
                    attempt,
                    "Failed to acknowledge messages, retrying until acked or shutdown"
                );
                tokio::select! {
                    () = tokio::time::sleep(ACK_RETRY_BACKOFF) => {}
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

fn extract_model(messages: &[sandbox_proto::PendingMessage]) -> Option<String> {
    messages.iter().rev().find_map(|m| m.model.clone())
}

fn extract_agent_type(messages: &[sandbox_proto::PendingMessage]) -> Option<String> {
    messages.iter().find_map(|m| m.agent_type.clone())
}

fn extract_agent_mode(messages: &[sandbox_proto::PendingMessage]) -> Option<String> {
    messages.iter().rev().find_map(|m| m.agent_mode.clone())
}

/// Extract text from proto pending messages.
fn extract_text_from_messages(messages: &[sandbox_proto::PendingMessage]) -> String {
    messages
        .iter()
        .flat_map(|m| {
            let blocks = content_blocks_from_proto(m.content.clone());
            blocks.into_iter().filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text),
                _ => None,
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_joins_message_content() {
        let messages = vec![
            sandbox_proto::PendingMessage {
                content: vec![sandbox_proto::ContentBlock {
                    block: Some(sandbox_proto::content_block::Block::Text(
                        sandbox_proto::TextBlock {
                            text: "hello".to_string(),
                        },
                    )),
                }],
                ..Default::default()
            },
            sandbox_proto::PendingMessage {
                content: vec![sandbox_proto::ContentBlock {
                    block: Some(sandbox_proto::content_block::Block::Text(
                        sandbox_proto::TextBlock {
                            text: "world".to_string(),
                        },
                    )),
                }],
                ..Default::default()
            },
        ];
        assert_eq!(extract_text_from_messages(&messages), "hello\nworld");
    }

    #[test]
    fn extract_text_empty_messages() {
        assert_eq!(extract_text_from_messages(&[]), "");
    }

    fn msg(start_fresh: bool) -> sandbox_proto::PendingMessage {
        sandbox_proto::PendingMessage {
            start_fresh: Some(start_fresh),
            ..Default::default()
        }
    }

    #[test]
    fn split_keeps_whole_batch_without_start_fresh() {
        let (kept, deferred) = split_at_start_fresh(vec![msg(false), msg(false)]);
        assert_eq!(kept.len(), 2);
        assert_eq!(deferred, 0);
    }

    #[test]
    fn split_dispatches_leading_start_fresh_alone() {
        let (kept, deferred) = split_at_start_fresh(vec![msg(true), msg(false), msg(false)]);
        assert_eq!(kept.len(), 1);
        assert_eq!(deferred, 2);
    }

    #[test]
    fn split_cuts_batch_before_start_fresh() {
        // The two leading messages run as their own turn; the start_fresh one
        // is held back so it can start a session of its own.
        let (kept, deferred) = split_at_start_fresh(vec![msg(false), msg(false), msg(true)]);
        assert_eq!(kept.len(), 2);
        assert_eq!(deferred, 1);
        assert!(kept.iter().all(|m| m.start_fresh != Some(true)));
    }
}
