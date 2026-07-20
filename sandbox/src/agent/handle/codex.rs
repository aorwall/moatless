//! Codex CLI event processing: turning `CodexExecEvent`s from the active
//! turn into agent events and state transitions.

use moatless_events::AgentSubType;
use tracing::{debug, info, warn};

use crate::agent::codex_exec::{CodexExecEvent, ThreadItemDetails};

use super::actor::TurnContext;
use super::claude::publish_session_init;
use super::codex_payloads::{
    codex_message_payload, codex_reasoning_payload, codex_result_payload,
    publish_codex_collab_call, publish_codex_edit_call, publish_codex_error_item,
    publish_codex_mcp_call, publish_codex_shell_call, publish_codex_todo_list_call,
    publish_codex_web_search_call,
};
use super::{append_turn_output, drain_pending_acks};

/// Process a Codex CLI event from the active turn.
pub(super) async fn process_codex_message(
    msg: CodexExecEvent,
    requested_model: Option<&str>,
    ctx: &mut TurnContext<'_>,
) {
    match msg {
        CodexExecEvent::ThreadStarted(event) => {
            info!(
                thread_id = %event.thread_id,
                model = ?requested_model,
                "Codex session initialized"
            );
            // Refresh the per-spawn UUID nonce. Codex CLI's `item_id` sequence
            // restarts at `item_0` on every `--resume`, so we mix this random
            // 8-char tag into every emitted UUID to keep turns disjoint.
            *ctx.spawn_nonce = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
            publish_session_init(
                ctx,
                event.thread_id,
                requested_model.map(std::string::ToString::to_string),
            )
            .await;
        }
        CodexExecEvent::TurnStarted(_) => {
            ctx.codex_turn_output.clear();
        }
        CodexExecEvent::ItemStarted(event) | CodexExecEvent::ItemUpdated(event) => {
            match &event.item.details {
                ThreadItemDetails::TodoList(todo) => {
                    publish_codex_todo_list_call(
                        &ctx.run_events(),
                        ctx.task_id,
                        ctx.session_id,
                        ctx.spawn_nonce,
                        &event.item.id,
                        todo,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::Other { type_name, .. } => {
                    debug!(type_name, "Codex item event (unknown kind)");
                }
                _ => {
                    debug!("Codex item event");
                }
            }
        }
        CodexExecEvent::ItemCompleted(event) => {
            let item_id = &event.item.id;
            match &event.item.details {
                ThreadItemDetails::AgentMessage(agent_message) => {
                    append_turn_output(ctx.codex_turn_output, &agent_message.text);

                    // Codex item ids are short and only unique within a single
                    // codex spawn (item_0, item_1, ...) — and the sequence
                    // restarts at item_0 on every `codex exec --resume`. The
                    // task_messages table uses a global ON CONFLICT (uuid), so
                    // we qualify the id with both the session (cross-task
                    // separation) and the per-spawn nonce (cross-turn
                    // separation within the same session).
                    let uuid = format!("{}_{}_{item_id}", ctx.session_id, ctx.spawn_nonce);
                    let payload = codex_message_payload(
                        &uuid,
                        ctx.session_id,
                        &agent_message.text,
                        requested_model,
                    );
                    ctx.run_events()
                        .publish_agent_event(ctx.task_id, AgentSubType::Message, payload)
                        .await;
                }
                ThreadItemDetails::Reasoning(reasoning) => {
                    let uuid = format!("{}_{}_{item_id}", ctx.session_id, ctx.spawn_nonce);
                    let payload = codex_reasoning_payload(
                        &uuid,
                        ctx.session_id,
                        &reasoning.text,
                        requested_model,
                    );
                    ctx.run_events()
                        .publish_agent_event(ctx.task_id, AgentSubType::Message, payload)
                        .await;
                }
                ThreadItemDetails::CommandExecution(cmd) => {
                    publish_codex_shell_call(
                        &ctx.run_events(),
                        ctx.task_id,
                        ctx.session_id,
                        ctx.spawn_nonce,
                        item_id,
                        cmd,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::FileChange(edit) => {
                    publish_codex_edit_call(
                        &ctx.run_events(),
                        ctx.task_id,
                        ctx.session_id,
                        ctx.spawn_nonce,
                        item_id,
                        edit,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::TodoList(todo) => {
                    publish_codex_todo_list_call(
                        &ctx.run_events(),
                        ctx.task_id,
                        ctx.session_id,
                        ctx.spawn_nonce,
                        item_id,
                        todo,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::McpToolCall(mcp) => {
                    publish_codex_mcp_call(
                        &ctx.run_events(),
                        ctx.task_id,
                        ctx.session_id,
                        ctx.spawn_nonce,
                        item_id,
                        mcp,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::CollabToolCall(collab) => {
                    publish_codex_collab_call(
                        &ctx.run_events(),
                        ctx.task_id,
                        ctx.session_id,
                        ctx.spawn_nonce,
                        item_id,
                        collab,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::WebSearch(web) => {
                    publish_codex_web_search_call(
                        &ctx.run_events(),
                        ctx.task_id,
                        ctx.session_id,
                        ctx.spawn_nonce,
                        item_id,
                        web,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::Error(err) => {
                    append_turn_output(ctx.codex_turn_output, &err.message);
                    publish_codex_error_item(
                        &ctx.run_events(),
                        ctx.task_id,
                        ctx.session_id,
                        ctx.spawn_nonce,
                        item_id,
                        &err.message,
                        requested_model,
                    )
                    .await;
                }
                ThreadItemDetails::Other { type_name, raw } => {
                    warn!(
                        item_id,
                        type_name,
                        raw = %raw,
                        "Unknown codex item type — skipped"
                    );
                }
            }
        }
        CodexExecEvent::TurnCompleted(event) => {
            let result_text = ctx.codex_turn_output.trim().to_string();
            // Codex can complete a turn without emitting any agent message
            // (`last_agent_message: null`). Tag that on the payload — the analog
            // of the Claude `stop_reason` — so the UI renders "stopped: empty
            // turn" instead of a silent, contentless completion.
            let empty_turn = result_text.is_empty();
            let mut payload = codex_result_payload(
                Some(ctx.session_id.as_str()),
                false,
                "completed",
                result_text,
                Some(&event.usage),
                requested_model,
            );
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("empty_turn".to_string(), serde_json::json!(empty_turn));
            }

            ctx.agent_state.set_idle().await;
            *ctx.consecutive_errors = 0;

            ctx.run_events()
                .publish_agent_event(ctx.task_id, AgentSubType::Completed, payload)
                .await;

            ctx.codex_turn_output.clear();
            *ctx.handle = None;
        }
        CodexExecEvent::TurnFailed(event) => {
            drain_pending_acks(ctx.pending_acks);
            ctx.agent_state.set_error(event.error.message.clone()).await;
            *ctx.consecutive_errors += 1;

            let payload = codex_result_payload(
                (!ctx.session_id.is_empty()).then_some(ctx.session_id.as_str()),
                true,
                "failed",
                event.error.message,
                None,
                requested_model,
            );

            ctx.run_events()
                .publish_agent_event(ctx.task_id, AgentSubType::Error, payload)
                .await;

            ctx.codex_turn_output.clear();
            *ctx.handle = None;
        }
        CodexExecEvent::Error(event) => {
            drain_pending_acks(ctx.pending_acks);
            ctx.agent_state.set_error(event.message.clone()).await;
            *ctx.consecutive_errors += 1;

            let payload = codex_result_payload(
                (!ctx.session_id.is_empty()).then_some(ctx.session_id.as_str()),
                true,
                "error",
                event.message,
                None,
                requested_model,
            );

            ctx.run_events()
                .publish_agent_event(ctx.task_id, AgentSubType::Error, payload)
                .await;

            ctx.codex_turn_output.clear();
            *ctx.handle = None;
        }
    }
}

pub(super) async fn handle_codex_stream_error(
    error: &str,
    requested_model: Option<&str>,
    ctx: &mut TurnContext<'_>,
) {
    warn!(error, "Codex stream error");
    drain_pending_acks(ctx.pending_acks);
    ctx.agent_state.set_error(error.to_string()).await;
    *ctx.consecutive_errors += 1;

    let payload = codex_result_payload(
        (!ctx.session_id.is_empty()).then_some(ctx.session_id.as_str()),
        true,
        "error",
        error.to_string(),
        None,
        requested_model,
    );
    ctx.run_events()
        .publish_agent_event(ctx.task_id, AgentSubType::Error, payload)
        .await;

    ctx.codex_turn_output.clear();
    *ctx.handle = None;
}
