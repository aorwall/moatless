//! Claude SDK message processing: turning `SdkMessage`s from the active
//! session into agent events and state transitions.

use claude_agent_sdk::types::content::{ApiMessage, ContentBlock};
use claude_agent_sdk::types::messages::{MessagePriority, ResultMessage, UserMessage};
use claude_agent_sdk::SdkMessage;
use moatless_events::AgentSubType;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::agent::wakeup_job::WakeupJob;

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
    usage_seen_message_ids: &mut std::collections::HashSet<String>,
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
        SdkMessage::Assistant(mut assistant) => {
            // Remember this turn's latest stop_reason — the `result` message has
            // none, so this is the only place it's available for agent/completed.
            if assistant.message.stop_reason.is_some() {
                *ctx.last_stop_reason = assistant.message.stop_reason.clone();
            }

            // The CLI splits one assistant API response into one `assistant` SDK
            // message per content block (thinking / text / tool_use), each
            // repeating the same Anthropic `message.id` and the same `usage`.
            // Persisting usage on every block row multiplies the token counts by
            // the block count, so keep usage on the first row seen for a message
            // id and strip it from the rest.
            dedup_assistant_usage(&mut assistant, usage_seen_message_ids);

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

            debug!(
                uuid = %assistant.uuid,
                blocks = assistant.message.content.len(),
                "Claude assistant message"
            );
            let payload = serde_json::to_value(&assistant).unwrap_or_default();
            ctx.run_events()
                .publish_agent_event(ctx.task_id, AgentSubType::Message, payload)
                .await;
        }
        SdkMessage::Result(result) => {
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
            // Message ids are globally unique, but clear the per-turn dedup set
            // so it can't grow without bound across a long-lived session.
            usage_seen_message_ids.clear();
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

/// Strip duplicate `usage` from a repeated assistant block row.
///
/// The CLI emits one `assistant` SDK message per content block (thinking / text
/// / tool_use), each carrying the same Anthropic `message.id` and the same
/// `usage`. Keeping usage on every row would multiply token counts by the block
/// count downstream, so usage is retained only on the first row seen for a given
/// message id and cleared on the rest. Rows without a `message.id` (or without
/// usage) are left untouched.
fn dedup_assistant_usage(
    assistant: &mut claude_agent_sdk::types::messages::AssistantMessage,
    seen_message_ids: &mut std::collections::HashSet<String>,
) {
    if assistant.message.usage.is_none() {
        return;
    }
    if let Some(message_id) = assistant.message.id.clone() {
        // `insert` returns false when the id was already present this turn.
        if !seen_message_ids.insert(message_id) {
            assistant.message.usage = None;
        }
    }
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

    // ── dedup_assistant_usage ───────────────────────────────────────

    fn make_assistant(message_id: Option<&str>, with_usage: bool) -> AssistantMessage {
        let usage = with_usage.then(|| ClaudeUsage {
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

    #[test]
    fn dedup_keeps_usage_on_first_block_and_strips_repeats() {
        let mut seen = std::collections::HashSet::new();

        // Three content-block rows share one message id — the CLI repeats the
        // same usage on each. Only the first (thinking) should keep it.
        let mut thinking = make_assistant(Some("msg_1"), true);
        let mut text = make_assistant(Some("msg_1"), true);
        let mut tool_use = make_assistant(Some("msg_1"), true);
        dedup_assistant_usage(&mut thinking, &mut seen);
        dedup_assistant_usage(&mut text, &mut seen);
        dedup_assistant_usage(&mut tool_use, &mut seen);

        assert!(thinking.message.usage.is_some());
        assert!(text.message.usage.is_none());
        assert!(tool_use.message.usage.is_none());

        // A distinct message id keeps its own usage.
        let mut other = make_assistant(Some("msg_2"), true);
        dedup_assistant_usage(&mut other, &mut seen);
        assert!(other.message.usage.is_some());

        // After a turn boundary clears the set, the same id counts again.
        seen.clear();
        let mut next_turn = make_assistant(Some("msg_1"), true);
        dedup_assistant_usage(&mut next_turn, &mut seen);
        assert!(next_turn.message.usage.is_some());
    }

    #[test]
    fn dedup_leaves_rows_without_message_id_untouched() {
        let mut seen = std::collections::HashSet::new();
        let mut a = make_assistant(None, true);
        let mut b = make_assistant(None, true);
        dedup_assistant_usage(&mut a, &mut seen);
        dedup_assistant_usage(&mut b, &mut seen);
        // No id to dedupe against, so usage is preserved on both.
        assert!(a.message.usage.is_some());
        assert!(b.message.usage.is_some());
        assert!(seen.is_empty());
    }
}
