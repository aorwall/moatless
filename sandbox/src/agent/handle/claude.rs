//! Claude SDK message processing: turning `SdkMessage`s from the active
//! session into agent events and state transitions.

use claude_agent_sdk::types::content::{ApiMessage, ContentBlock};
use claude_agent_sdk::types::messages::{
    AssistantMessage, MessagePriority, ResultMessage, UserMessage,
};
use claude_agent_sdk::SdkMessage;
use moatless_events::AgentSubType;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use std::collections::HashSet;

use crate::agent::wakeup_job::WakeupJob;
use crate::events::GrpcEventPublisher;

use super::actor::TurnContext;
use super::session::publish_turn_start;
use super::truncate;

/// Walk assistant content blocks for a `ScheduleWakeup` tool call and parse
/// it into a `WakeupJob`. Returns `None` if no such block is present or any
/// required field is missing.
fn extract_wakeup_job(content: &[ContentBlock]) -> Option<WakeupJob> {
    for block in content {
        if let ContentBlock::ToolUse {
            id, name, input, ..
        } = block
        {
            if name == "ScheduleWakeup" {
                return WakeupJob::from_tool_use(id, input);
            }
        }
    }
    None
}

pub(super) async fn publish_session_init(
    ctx: &mut TurnContext<'_>,
    new_session_id: String,
    model: Option<String>,
) {
    *ctx.session_id = new_session_id.clone();
    ctx.agent_state.set_session_id(new_session_id.clone()).await;

    while let Some(ack) = ctx.pending_acks.pop_front() {
        ctx.ack_current_run(ack);
    }

    ctx.run_events()
        .publish_agent_event(
            ctx.task_id,
            AgentSubType::SessionInit,
            serde_json::json!({
                "session_id": new_session_id,
                "model": model,
            }),
        )
        .await;
}

/// Process a Claude SDK message from the active session.
pub(super) async fn process_claude_message(
    msg: SdkMessage,
    ctx: &mut TurnContext<'_>,
    use_tmux: bool,
    turn_cancel: &CancellationToken,
    assistant_buffer: &mut AssistantBuffer,
) {
    match msg {
        SdkMessage::System(init) => {
            info!(
                session_id = %init.session_id,
                model = %init.model,
                "Claude session initialized"
            );
            publish_session_init(ctx, init.session_id, Some(init.model)).await;
        }
        SdkMessage::Assistant(assistant) => {
            // Remember this turn's latest stop_reason — the `result` message has
            // none, so this is the only place it's available for agent/completed.
            if assistant.message.stop_reason.is_some() {
                *ctx.last_stop_reason = assistant.message.stop_reason.clone();
            }

            if use_tmux {
                // The CLI handles ScheduleWakeup internally in the persistent
                // tmux session, so no sandbox WakeupJob is armed. A message
                // arriving while no turn is in flight means the CLI started a
                // turn on its own (internal wakeup fired / Monitor event) —
                // surface the turn start that `start_session`/the paste path
                // would otherwise publish.
                if !*ctx.tmux_turn_in_flight {
                    info!("CLI-internal turn detected in persistent tmux session");
                    *ctx.tmux_turn_in_flight = true;
                    ctx.begin_run().await;
                    ctx.agent_state.set_running(turn_cancel.clone()).await;
                    publish_turn_start(ctx.events, ctx.task_id, ctx.run_id).await;
                }
            } else if let Some(job) = extract_wakeup_job(&assistant.message.content) {
                // Check for ScheduleWakeup tool call (subprocess mode: the
                // sandbox owns the wakeup timer because the CLI process dies
                // at turn end).
                debug!(
                    reason = %job.reason(),
                    "ScheduleWakeup tool call detected"
                );
                *ctx.pending_wakeup = Some(job);
            }

            // Held until the next message proves this one complete.
            if let Some(complete) = assistant_buffer.absorb(assistant) {
                publish_assistant(ctx, complete).await;
            }
        }
        SdkMessage::Result(result) => {
            // Release the final assistant message before the turn's terminal
            // event so the backend sees them in order.
            if let Some(complete) = assistant_buffer.take() {
                publish_assistant(ctx, complete).await;
            }

            let (is_error, sid, sub_type, payload) =
                extract_result_info(&result, ctx.task_id, ctx.last_stop_reason.as_deref());
            *ctx.session_id = sid;
            ctx.agent_state.set_session_id(ctx.session_id.clone()).await;

            if is_error {
                let error_text = extract_error_text(&result);
                ctx.agent_state.set_error(error_text).await;
                *ctx.consecutive_errors += 1;
            } else {
                ctx.agent_state.set_idle().await;
                *ctx.consecutive_errors = 0;
            }

            ctx.run_events()
                .publish_agent_event(ctx.task_id, sub_type, payload)
                .await;

            // Turn is over — clear so the next turn starts without a stale reason.
            *ctx.last_stop_reason = None;
            assistant_buffer.end_turn();
            *ctx.tmux_turn_in_flight = false;
            // A successful turn in the persistent tmux session keeps the
            // handle (and the TUI process) alive so the CLI's internal
            // ScheduleWakeup timer and Monitor watcher keep running.
            // Everything else — subprocess turn end or a tmux error result —
            // closes the session; the retry/restart paths respawn with
            // `--resume`.
            if !use_tmux || is_error {
                *ctx.handle = None;
            }
        }
        SdkMessage::User(user_msg) => {
            // A tool_result answers the tool_use in the pending message, so that
            // message must be published first to keep the conversation ordered.
            if let Some(complete) = assistant_buffer.take() {
                publish_assistant(ctx, complete).await;
            }

            // A user record arriving while no turn is in flight is the start
            // of a CLI-internal turn (the wakeup prompt the CLI injected when
            // its internal ScheduleWakeup timer fired) — publish turn-start
            // BEFORE the message event so the backend opens the turn first.
            if use_tmux && !*ctx.tmux_turn_in_flight {
                info!("CLI-internal turn detected in persistent tmux session");
                *ctx.tmux_turn_in_flight = true;
                ctx.begin_run().await;
                ctx.agent_state.set_running(turn_cancel.clone()).await;
                publish_turn_start(ctx.events, ctx.task_id, ctx.run_id).await;
            }

            debug!(
                uuid = ?user_msg.uuid,
                blocks = user_msg.message.content.len(),
                "Claude user message"
            );
            let payload = serde_json::to_value(&user_msg).unwrap_or_default();
            ctx.run_events()
                .publish_agent_event(ctx.task_id, AgentSubType::Message, payload)
                .await;
        }
        SdkMessage::ToolProgress(_) => {
            // A long-running tool call (e.g. a multi-minute build) streams
            // progress but emits no assistant/user message, so nothing else
            // advances the reaper's activity floor and an in-flight turn can
            // be mistaken for idle. Publish a lightweight liveness event,
            // rate-limited — the floor only needs minute-level granularity
            // against a TTL measured in tens of minutes, and a chatty tool
            // would otherwise flood the event stream.
            const ACTIVITY_PUBLISH_INTERVAL: std::time::Duration =
                std::time::Duration::from_mins(1);
            let due = ctx
                .last_activity_publish
                .is_none_or(|at| at.elapsed() >= ACTIVITY_PUBLISH_INTERVAL);
            if due {
                *ctx.last_activity_publish = Some(std::time::Instant::now());
                ctx.run_events()
                    .publish_agent_event(
                        ctx.task_id,
                        AgentSubType::Activity,
                        serde_json::json!({ "kind": "tool_progress" }),
                    )
                    .await;
            }
        }
        _ => {
            debug!("Claude SDK message (non-content)");
        }
    }
}

/// Reassembles one API response from the CLI's per-content-block lines.
///
/// The CLI emits one `assistant` SDK message per content block (thinking / text
/// / tool_use), each repeating the same Anthropic `message.id` and the same
/// `usage`. Holding a message until its id changes — or until any other message
/// arrives — lets one API response reach the backend as one event carrying all
/// its blocks rather than one per block.
///
/// Reassembly is best-effort by necessity. The CLI interleaves independent
/// chains into one stream: a sub-agent's traffic, and the `tool_result` for each
/// call of a multi-tool response, arrive *between* the lines of the response
/// that spawned them. Holding across those would publish a `tool_use` after its
/// own result, so the pending message is released instead and one API message
/// legitimately becomes several rows. `usage_seen` is what keeps that from
/// multiplying the token counts.
#[derive(Default)]
pub(super) struct AssistantBuffer {
    pending: Option<AssistantMessage>,
    /// Message ids already released this turn. Usage rides on every line the CLI
    /// emits, so once a message id has been published the repeats must not carry
    /// it again — otherwise a response split across N rows reports N× its tokens.
    usage_seen: HashSet<String>,
}

impl AssistantBuffer {
    /// Fold `next` into the pending message when both are the same API response.
    ///
    /// Returns the previous message once it is complete, for the caller to
    /// publish. Lines with no `message.id` can't be correlated, so each is
    /// returned as its own message.
    fn absorb(&mut self, mut next: AssistantMessage) -> Option<AssistantMessage> {
        let continues_pending = match (self.pending.as_ref(), next.message.id.as_deref()) {
            (Some(pending), Some(id)) => pending.message.id.as_deref() == Some(id),
            _ => false,
        };

        if continues_pending {
            if let Some(pending) = self.pending.as_mut() {
                // Later lines repeat the same usage and stop_reason; the first
                // line's copy is already held.
                pending.message.content.extend(next.message.content);
            }
            return None;
        }

        // Becoming the pending message. If this id was already released once
        // this turn, the stream interleaved something between its lines — keep
        // the blocks, drop the repeated usage.
        if let Some(id) = next.message.id.clone() {
            if !self.usage_seen.insert(id) {
                next.message.usage = None;
            }
        }
        self.pending.replace(next)
    }

    /// Release the pending message so it is published before whatever follows.
    fn take(&mut self) -> Option<AssistantMessage> {
        self.pending.take()
    }

    /// Drop the per-turn id set at a turn boundary so it cannot grow unbounded
    /// across a long-lived session.
    fn end_turn(&mut self) {
        self.usage_seen.clear();
    }
}

/// Drain a message still held when a turn ends without a `Result`.
///
/// The cancel, stream-error and transport-closed paths never reach
/// [`process_claude_message`], so nothing else releases the buffer. Left
/// undrained the held message is lost from its own turn and then published
/// during the next one, attributed to the wrong run.
pub(super) async fn flush_pending_assistant(
    buffer: &mut AssistantBuffer,
    events: &GrpcEventPublisher,
    task_id: &str,
) {
    if let Some(complete) = buffer.take() {
        publish_assistant_to(events, task_id, complete).await;
    }
}

/// Publish a reassembled assistant message.
async fn publish_assistant(ctx: &mut TurnContext<'_>, assistant: AssistantMessage) {
    publish_assistant_to(&ctx.run_events(), ctx.task_id, assistant).await;
}

async fn publish_assistant_to(
    events: &GrpcEventPublisher,
    task_id: &str,
    mut assistant: AssistantMessage,
) {
    // The CLI's stdout stream carries no stop_reason (it is only written to the
    // transcript), but the API's contract is that a response containing
    // tool_use blocks stopped for `tool_use`. Turn reconciliation keys on this,
    // so derive what is unambiguous and leave the rest unset.
    if assistant.message.stop_reason.is_none()
        && assistant
            .message
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
    {
        assistant.message.stop_reason = Some("tool_use".to_string());
    }

    debug!(
        uuid = %assistant.uuid,
        blocks = assistant.message.content.len(),
        "Claude assistant message"
    );
    let payload = serde_json::to_value(&assistant).unwrap_or_default();
    events
        .publish_agent_event(task_id, AgentSubType::Message, payload)
        .await;
}

/// Construct a `UserMessage` for injection into a running Claude session.
pub(super) fn make_user_message(text: &str, session_id: &str) -> UserMessage {
    UserMessage {
        uuid: None,
        session_id: session_id.to_string(),
        message: ApiMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            usage: None,
            model: None,
            id: None,
            stop_reason: None,
            stop_sequence: None,
        },
        parent_tool_use_id: None,
        is_synthetic: None,
        tool_use_result: None,
        is_meta: None,
        is_compact_summary: None,
        is_visible_in_transcript_only: None,
        plan_content: None,
        thinking_metadata: None,
        todos: None,
        summarize_metadata: None,
        source_tool_assistant_uuid: None,
        source_tool_use_id: None,
        permission_mode: None,
        priority: MessagePriority::Next,
    }
}

/// Extract result info: (is_error, session_id, sub_type, payload).
///
/// When `is_error: true` comes back in a `ResultMessage::Success` (e.g. the CLI
/// rejects the message with `"Unknown command: ..."`), we route the event through
/// the `AgentSubType::Error` path so the backend treats it as an error and the
/// frontend renders the rejection text as a red error banner. We also inject
/// `error` into the payload so `handle_agent_error` can read it without having
/// to know the SDK-specific shape.
fn extract_result_info(
    result: &ResultMessage,
    task_id: &str,
    stop_reason: Option<&str>,
) -> (bool, String, AgentSubType, serde_json::Value) {
    // Inject the turn's last assistant stop_reason into a result payload object.
    let with_stop_reason = |mut payload: serde_json::Value| {
        if let (Some(reason), Some(obj)) = (stop_reason, payload.as_object_mut()) {
            obj.insert(
                "stop_reason".to_string(),
                serde_json::Value::String(reason.to_string()),
            );
        }
        payload
    };
    match result {
        ResultMessage::Success(success) => {
            if success.is_error {
                warn!(
                    task_id = task_id,
                    session_id = %success.session_id,
                    num_turns = success.num_turns,
                    duration_ms = success.duration_ms,
                    cost_usd = success.total_cost_usd,
                    output = %truncate(&success.result, 500),
                    "Agent completed with error output"
                );
                let mut payload = serde_json::to_value(success).unwrap_or_default();
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert(
                        "error".to_string(),
                        serde_json::Value::String(success.result.clone()),
                    );
                }
                return (
                    true,
                    success.session_id.clone(),
                    AgentSubType::Error,
                    with_stop_reason(payload),
                );
            }
            info!(
                task_id = task_id,
                session_id = %success.session_id,
                num_turns = success.num_turns,
                duration_ms = success.duration_ms,
                cost_usd = success.total_cost_usd,
                output = %truncate(&success.result, 500),
                "Agent completed successfully"
            );
            (
                false,
                success.session_id.clone(),
                AgentSubType::Completed,
                with_stop_reason(serde_json::to_value(success).unwrap_or_default()),
            )
        }
        ResultMessage::Error(error) => {
            warn!(
                task_id = task_id,
                session_id = %error.session_id,
                num_turns = error.num_turns,
                duration_ms = error.duration_ms,
                subtype = %error.subtype,
                errors = ?error.errors,
                "Agent completed with error"
            );
            (
                true,
                error.session_id.clone(),
                AgentSubType::Error,
                with_stop_reason(serde_json::to_value(error).unwrap_or_default()),
            )
        }
    }
}

/// Extract a human-readable error text from a result message.
fn extract_error_text(result: &ResultMessage) -> String {
    match result {
        ResultMessage::Success(success) => truncate(&success.result, 500).to_string(),
        ResultMessage::Error(error) => error
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "Unknown error".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use claude_agent_sdk::types::content::Usage as ClaudeUsage;
    use claude_agent_sdk::types::messages::{AssistantMessage, ResultError, ResultSuccess};

    fn make_result_error(subtype: &str, num_turns: u32, errors: Vec<String>) -> ResultMessage {
        ResultMessage::Error(ResultError {
            subtype: subtype.to_string(),
            uuid: "test-uuid".to_string(),
            session_id: "test-session".to_string(),
            duration_ms: 100,
            duration_api_ms: 50,
            is_error: true,
            num_turns,
            total_cost_usd: 0.0,
            usage: ClaudeUsage {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                service_tier: None,
                cache_creation: None,
                server_tool_use: None,
            },
            model_usage: HashMap::new(),
            permission_denials: vec![],
            errors,
        })
    }

    fn make_result_success(is_error: bool, num_turns: u32, result: &str) -> ResultMessage {
        ResultMessage::Success(ResultSuccess {
            uuid: "test-uuid".to_string(),
            session_id: "test-session".to_string(),
            duration_ms: 100,
            duration_api_ms: 50,
            is_error,
            num_turns,
            result: result.to_string(),
            total_cost_usd: 0.0,
            usage: ClaudeUsage {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                service_tier: None,
                cache_creation: None,
                server_tool_use: None,
            },
            model_usage: HashMap::new(),
            permission_denials: vec![],
            structured_output: None,
        })
    }

    // ── extract_wakeup_job ──────────────────────────────────────────

    #[test]
    fn extract_wakeup_job_returns_some_for_valid_tool_call() {
        let content = vec![ContentBlock::ToolUse {
            id: "toolu_123".to_string(),
            name: "ScheduleWakeup".to_string(),
            input: serde_json::json!({
                "delaySeconds": 120,
                "prompt": "check build status",
                "reason": "waiting for build to complete"
            }),
        }];
        let job = extract_wakeup_job(&content).expect("expected wakeup job");
        assert_eq!(job.id(), "toolu_123");
        assert_eq!(job.reason(), "waiting for build to complete");
    }

    #[test]
    fn extract_wakeup_job_returns_none_for_other_tool() {
        let content = vec![ContentBlock::ToolUse {
            id: "toolu_456".to_string(),
            name: "Bash".to_string(),
            input: serde_json::json!({ "command": "ls -la" }),
        }];
        assert!(extract_wakeup_job(&content).is_none());
    }

    #[test]
    fn extract_wakeup_job_returns_none_for_text_block() {
        let content = vec![ContentBlock::Text {
            text: "Some text".to_string(),
        }];
        assert!(extract_wakeup_job(&content).is_none());
    }

    #[test]
    fn extract_wakeup_job_returns_none_for_missing_fields() {
        let content = vec![ContentBlock::ToolUse {
            id: "toolu_789".to_string(),
            name: "ScheduleWakeup".to_string(),
            input: serde_json::json!({
                "delaySeconds": 60
                // missing prompt and reason
            }),
        }];
        assert!(extract_wakeup_job(&content).is_none());
    }

    #[test]
    fn extract_wakeup_job_finds_in_mixed_content() {
        let content = vec![
            ContentBlock::Text {
                text: "I'll schedule a wakeup".to_string(),
            },
            ContentBlock::ToolUse {
                id: "toolu_first".to_string(),
                name: "Bash".to_string(),
                input: serde_json::json!({ "command": "echo hi" }),
            },
            ContentBlock::ToolUse {
                id: "toolu_wakeup".to_string(),
                name: "ScheduleWakeup".to_string(),
                input: serde_json::json!({
                    "delaySeconds": 300,
                    "prompt": "resume work",
                    "reason": "cache cooldown"
                }),
            },
        ];
        let job = extract_wakeup_job(&content).expect("expected wakeup job");
        assert_eq!(job.id(), "toolu_wakeup");
        assert_eq!(job.reason(), "cache cooldown");
    }

    #[test]
    fn extract_wakeup_job_returns_none_for_empty_content() {
        let content: Vec<ContentBlock> = vec![];
        assert!(extract_wakeup_job(&content).is_none());
    }

    // ── extract_result_info ─────────────────────────────────────────

    #[test]
    fn extract_result_info_success_is_error_true_routes_to_error_subtype() {
        let result = make_result_success(true, 1, "Unknown command: /foo");
        let (is_error, session_id, sub_type, payload) =
            extract_result_info(&result, "task-1", None);
        assert!(is_error);
        assert_eq!(session_id, "test-session");
        assert_eq!(sub_type, AgentSubType::Error);
        assert_eq!(payload["error"], "Unknown command: /foo");
        assert_eq!(payload["result"], "Unknown command: /foo");
        assert_eq!(payload["is_error"], true);
    }

    #[test]
    fn extract_result_info_success_is_error_false_unchanged() {
        let result = make_result_success(false, 1, "All done");
        let (is_error, _, sub_type, payload) = extract_result_info(&result, "task-1", None);
        assert!(!is_error);
        assert_eq!(sub_type, AgentSubType::Completed);
        assert!(payload.get("error").is_none());
        assert_eq!(payload["result"], "All done");
        // No stop_reason was captured this turn → not injected.
        assert!(payload.get("stop_reason").is_none());
    }

    #[test]
    fn extract_result_info_injects_stop_reason_when_present() {
        let result = make_result_success(false, 1, "All done");
        let (_, _, sub_type, payload) = extract_result_info(&result, "task-1", Some("end_turn"));
        assert_eq!(sub_type, AgentSubType::Completed);
        assert_eq!(payload["stop_reason"], "end_turn");
    }

    #[test]
    fn extract_result_info_explicit_error_variant_unchanged() {
        let result = make_result_error("rate_limit", 1, vec!["429".to_string()]);
        let (is_error, _, sub_type, payload) = extract_result_info(&result, "task-1", None);
        assert!(is_error);
        assert_eq!(sub_type, AgentSubType::Error);
        assert_eq!(payload["subtype"], "rate_limit");
    }

    // ── AssistantBuffer ─────────────────────────────────────────────

    fn make_assistant(message_id: Option<&str>, with_usage: bool) -> AssistantMessage {
        let usage = with_usage.then_some(ClaudeUsage {
            input_tokens: 2,
            output_tokens: 3,
            cache_read_input_tokens: Some(238_041),
            cache_creation_input_tokens: Some(1854),
            service_tier: None,
            cache_creation: None,
            server_tool_use: None,
        });
        AssistantMessage {
            uuid: "row-uuid".to_string(),
            session_id: "sess".to_string(),
            message: ApiMessage {
                role: "assistant".to_string(),
                content: vec![],
                usage,
                model: Some("claude-opus-4-8".to_string()),
                id: message_id.map(ToString::to_string),
                stop_reason: None,
                stop_sequence: None,
            },
            parent_tool_use_id: None,
            error: None,
            is_api_error_message: None,
            api_error: None,
            request_id: None,
        }
    }

    /// Build an assistant line carrying a single content block, as the CLI
    /// emits them.
    fn block_line(message_id: Option<&str>, block: ContentBlock) -> AssistantMessage {
        let mut msg = make_assistant(message_id, true);
        msg.message.content = vec![block];
        msg
    }

    fn text_block(text: &str) -> ContentBlock {
        ContentBlock::Text {
            text: text.to_string(),
        }
    }

    fn tool_use_block(id: &str) -> ContentBlock {
        ContentBlock::ToolUse {
            id: id.to_string(),
            name: "Write".to_string(),
            input: serde_json::json!({}),
        }
    }

    #[test]
    fn buffer_merges_block_lines_sharing_a_message_id() {
        let mut buffer = AssistantBuffer::default();

        // The CLI splits one response across three lines; none completes it.
        assert!(buffer
            .absorb(block_line(
                Some("msg_1"),
                ContentBlock::Thinking {
                    thinking: "hmm".to_string(),
                    signature: "sig".to_string(),
                },
            ))
            .is_none());
        assert!(buffer
            .absorb(block_line(Some("msg_1"), text_block("working on it")))
            .is_none());
        assert!(buffer
            .absorb(block_line(Some("msg_1"), tool_use_block("toolu_1")))
            .is_none());

        let complete = buffer.take().expect("pending message");
        assert_eq!(complete.message.content.len(), 3);
        assert_eq!(complete.message.id.as_deref(), Some("msg_1"));
        // Usage is carried once, from the first line — not multiplied by block count.
        let usage = complete.message.usage.expect("usage");
        assert_eq!(usage.output_tokens, 3);
    }

    #[test]
    fn buffer_releases_previous_message_when_the_id_changes() {
        let mut buffer = AssistantBuffer::default();

        assert!(buffer
            .absorb(block_line(Some("msg_1"), text_block("first")))
            .is_none());

        let released = buffer
            .absorb(block_line(Some("msg_2"), text_block("second")))
            .expect("previous message released");
        assert_eq!(released.message.id.as_deref(), Some("msg_1"));

        let pending = buffer.take().expect("second message pending");
        assert_eq!(pending.message.id.as_deref(), Some("msg_2"));
    }

    #[test]
    fn buffer_does_not_correlate_lines_without_a_message_id() {
        let mut buffer = AssistantBuffer::default();

        assert!(buffer.absorb(block_line(None, text_block("a"))).is_none());
        // With no id to match on, the previous line is released rather than
        // merged — better a split message than one that swallows another.
        let released = buffer
            .absorb(block_line(None, text_block("b")))
            .expect("previous line released");
        assert_eq!(released.message.content.len(), 1);
    }

    #[test]
    fn interleaved_lines_split_the_message_but_carry_usage_once() {
        // A multi-tool response: the CLI emits each tool_use as its own line and
        // slots that tool's result in between, so the message cannot be held
        // whole without publishing a tool_use after its own result. It is
        // released instead — but only the first row keeps the usage.
        let mut buffer = AssistantBuffer::default();

        assert!(buffer
            .absorb(block_line(Some("msg_1"), tool_use_block("toolu_1")))
            .is_none());
        // ...tool_result for toolu_1 arrives, forcing the release.
        let first = buffer.take().expect("first half released");
        assert!(first.message.usage.is_some(), "first row keeps the usage");

        // Second line of the SAME api message, after the interleaved result.
        assert!(buffer
            .absorb(block_line(Some("msg_1"), tool_use_block("toolu_2")))
            .is_none());
        let second = buffer.take().expect("second half released");
        assert!(
            second.message.usage.is_none(),
            "repeat of an already-released message id must not re-report usage"
        );
    }

    #[test]
    fn usage_dedup_resets_at_a_turn_boundary() {
        let mut buffer = AssistantBuffer::default();
        assert!(buffer
            .absorb(block_line(Some("msg_1"), text_block("a")))
            .is_none());
        assert!(buffer.take().expect("released").message.usage.is_some());

        buffer.end_turn();

        // A fresh turn starts with a clean set — ids are globally unique, so
        // this only matters for keeping the set from growing without bound.
        assert!(buffer
            .absorb(block_line(Some("msg_1"), text_block("b")))
            .is_none());
        assert!(buffer.take().expect("released").message.usage.is_some());
    }

    #[test]
    fn buffer_does_not_carry_a_message_into_the_next_turn() {
        // A turn that ends abnormally (cancel / crash) never delivers a Result,
        // so the actor drains the buffer itself. Once drained, the next turn
        // starts clean — otherwise the previous turn's message would surface
        // under the new run id.
        let mut buffer = AssistantBuffer::default();
        assert!(buffer
            .absorb(block_line(Some("msg_turn_1"), tool_use_block("toolu_1")))
            .is_none());

        let stranded = buffer.take().expect("held message is recoverable");
        assert_eq!(stranded.message.id.as_deref(), Some("msg_turn_1"));

        // Next turn's first line must not be merged into the drained message.
        let released = buffer.absorb(block_line(Some("msg_turn_2"), text_block("next")));
        assert!(released.is_none(), "nothing pending from the previous turn");
        assert_eq!(
            buffer.take().and_then(|m| m.message.id).as_deref(),
            Some("msg_turn_2")
        );
    }

    #[test]
    fn buffer_take_is_empty_once_drained() {
        let mut buffer = AssistantBuffer::default();
        assert!(buffer.absorb(block_line(Some("msg_1"), text_block("x"))).is_none());
        assert!(buffer.take().is_some());
        assert!(buffer.take().is_none());
    }
}
