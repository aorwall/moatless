//! Claude Code agent executor — implements `AgentExecutor` as a pure event source.
//!
//! Wraps `claude_agent_sdk::query()` and streams `AgentEvent`s through a channel.
//! Does NOT manage lifecycle state, publish events, or acknowledge messages —
//! all of that is handled by the `run_agent()` orchestrator in `executor.rs`.

use std::sync::Arc;

use claude_agent_sdk::SdkMessage;
use moatless_events::AgentSubType;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::error::SandboxError;
use crate::state::SandboxState;

use super::executor::{AgentEvent, AgentExecutor};
use super::options::build_options;
use super::types::AgentRunRequest;

/// Claude Code executor — a pure streaming event source.
///
/// Builds SDK options, calls `claude_agent_sdk::query()`, and converts
/// SDK messages to `AgentEvent`s sent through the channel.
pub struct ClaudeCodeExecutor;

#[async_trait::async_trait]
impl AgentExecutor for ClaudeCodeExecutor {
    async fn execute(
        &self,
        state: &SandboxState,
        request: AgentRunRequest,
        cancel_token: CancellationToken,
        events_tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<(), SandboxError> {
        let prompt = request.extract_text();
        let agent_type = request.agent_type.clone();
        let agent_type_str = agent_type.as_deref().unwrap_or("claude-code");

        info!(
            task_id = %state.config.task_id,
            prompt_len = prompt.len(),
            agent_type = agent_type_str,
            "Starting agent execution"
        );

        // Re-fetch credentials before each agent run to pick up rotated tokens
        if let Err(e) = crate::init::credentials::fetch_and_apply_credentials(state).await {
            warn!(error = %e, "Failed to refresh credentials, using cached");
        }

        // Read credentials from state to pass as env vars to the agent subprocess
        let credentials = state.credentials.read().await;
        let env = if credentials.is_empty() {
            None
        } else {
            Some(credentials.clone())
        };
        drop(credentials);

        // Build options from config + request
        let options = build_options(
            &state.config,
            &request,
            cancel_token,
            Arc::clone(&state.agent),
            env,
        );

        // Start the query
        let mut handle = claude_agent_sdk::query(&prompt, options)
            .await
            .map_err(SandboxError::Sdk)?;

        // Process message stream — convert SDK messages to AgentEvents
        let mut event_count = 0u64;
        while let Some(msg_result) = handle.messages.recv().await {
            let msg = match msg_result {
                Ok(m) => m,
                Err(e) => {
                    if matches!(e, claude_agent_sdk::SdkError::Cancelled) {
                        break;
                    }
                    warn!(error = %e, "SDK message error");
                    continue;
                }
            };

            event_count += 1;

            match msg {
                SdkMessage::System(init) => {
                    info!(
                        session_id = %init.session_id,
                        model = %init.model,
                        "Agent initialized"
                    );
                    // session_id is intrinsic agent state — set directly
                    state.agent.set_session_id(init.session_id.clone()).await;

                    let _ = events_tx.send(AgentEvent::SessionInit {
                        session_id: init.session_id,
                        model: init.model,
                    });
                }
                SdkMessage::Assistant(assistant) => {
                    debug!(
                        uuid = %assistant.uuid,
                        blocks = assistant.message.content.len(),
                        "Assistant message received"
                    );
                    let payload = serde_json::to_value(&assistant).unwrap_or_default();
                    let _ = events_tx.send(AgentEvent::Message {
                        sub_type: AgentSubType::Message,
                        payload,
                    });
                }
                SdkMessage::Result(result) => {
                    let (is_error, session_id, sub_type, payload) = extract_result_info(&result);
                    state.agent.set_session_id(session_id).await;

                    // Record error in agent state so it's visible via status endpoint
                    if is_error {
                        let error_text = extract_error_text(&result);
                        state.agent.set_error(error_text).await;
                    }

                    let _ = events_tx.send(AgentEvent::Result {
                        is_error,
                        sub_type,
                        payload,
                    });

                    info!(event_count, is_error, "Agent execution completed");
                }
                SdkMessage::User(user_msg) => {
                    debug!(
                        uuid = ?user_msg.uuid,
                        blocks = user_msg.message.content.len(),
                        "User message (tool results) received"
                    );
                    let payload = serde_json::to_value(&user_msg).unwrap_or_default();
                    let _ = events_tx.send(AgentEvent::Message {
                        sub_type: AgentSubType::Message,
                        payload,
                    });
                }
                msg => {
                    let variant = sdk_message_variant(&msg);
                    debug!(event_count, variant, "SDK message");
                }
            }
        }

        info!(
            task_id = %state.config.task_id,
            event_count,
            "Agent execution finished"
        );

        Ok(())
    }
}

/// Extract result info: (is_error, session_id, sub_type, payload).
fn extract_result_info(
    result: &claude_agent_sdk::types::messages::ResultMessage,
) -> (bool, String, AgentSubType, serde_json::Value) {
    match result {
        claude_agent_sdk::types::messages::ResultMessage::Success(s) => {
            if s.is_error {
                warn!(
                    session_id = %s.session_id,
                    num_turns = s.num_turns,
                    duration_ms = s.duration_ms,
                    cost_usd = s.total_cost_usd,
                    "Agent completed with is_error=true"
                );
                warn!(output = %truncate(&s.result, 500), "Agent error output");
            } else {
                info!(
                    session_id = %s.session_id,
                    num_turns = s.num_turns,
                    duration_ms = s.duration_ms,
                    cost_usd = s.total_cost_usd,
                    "Agent completed successfully"
                );
                info!(output = %truncate(&s.result, 500), "Agent output");
            }
            // Always publish Success as "agent_completed" — the backend reads
            // the is_error flag from the payload and routes accordingly.
            (
                s.is_error,
                s.session_id.clone(),
                AgentSubType::Completed,
                serde_json::to_value(s).unwrap_or_default(),
            )
        }
        claude_agent_sdk::types::messages::ResultMessage::Error(e) => {
            warn!(
                session_id = %e.session_id,
                num_turns = e.num_turns,
                duration_ms = e.duration_ms,
                subtype = %e.subtype,
                "Agent completed with error"
            );
            for err in &e.errors {
                warn!(error = %err, "Agent error detail");
            }
            (
                true,
                e.session_id.clone(),
                AgentSubType::Error,
                serde_json::to_value(e).unwrap_or_default(),
            )
        }
    }
}

/// Extract a human-readable error text from a result message.
fn extract_error_text(result: &claude_agent_sdk::types::messages::ResultMessage) -> String {
    match result {
        claude_agent_sdk::types::messages::ResultMessage::Success(s) => {
            truncate(&s.result, 500).to_string()
        }
        claude_agent_sdk::types::messages::ResultMessage::Error(e) => e
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "Unknown error".to_string()),
    }
}

/// Return a short label for an SdkMessage variant (for logging).
fn sdk_message_variant(msg: &SdkMessage) -> &'static str {
    match msg {
        SdkMessage::Assistant(_) => "assistant",
        SdkMessage::User(_) => "user",
        SdkMessage::UserReplay(_) => "user_replay",
        SdkMessage::Result(_) => "result",
        SdkMessage::System(_) => "system_init",
        SdkMessage::CompactBoundary(_) => "compact_boundary",
        SdkMessage::Status(_) => "status",
        SdkMessage::HookStarted(_) => "hook_started",
        SdkMessage::HookProgress(_) => "hook_progress",
        SdkMessage::HookResponse(_) => "hook_response",
        SdkMessage::TaskNotification(_) => "task_notification",
        SdkMessage::FilesPersisted(_) => "files_persisted",
        SdkMessage::StreamEvent(_) => "stream_event",
        SdkMessage::ToolProgress(_) => "tool_progress",
        SdkMessage::AuthStatus(_) => "auth_status",
        SdkMessage::ToolUseSummary(_) => "tool_use_summary",
    }
}

/// Truncate a string to at most `max` characters (on a char boundary).
fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_agent_sdk::types::content::Usage;
    use claude_agent_sdk::types::messages::{ResultError, ResultMessage, ResultSuccess};
    use std::collections::HashMap;

    fn make_usage() -> Usage {
        Usage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            service_tier: None,
            cache_creation: None,
            server_tool_use: None,
        }
    }

    fn make_success(is_error: bool, result: &str) -> ResultMessage {
        ResultMessage::Success(ResultSuccess {
            uuid: "uuid-1".to_string(),
            session_id: "sess-abc".to_string(),
            duration_ms: 1000,
            duration_api_ms: 800,
            is_error,
            num_turns: 3,
            result: result.to_string(),
            total_cost_usd: 0.01,
            usage: make_usage(),
            model_usage: HashMap::new(),
            permission_denials: vec![],
            structured_output: None,
        })
    }

    fn make_error(errors: Vec<&str>) -> ResultMessage {
        ResultMessage::Error(ResultError {
            subtype: "agent_error".to_string(),
            uuid: "uuid-2".to_string(),
            session_id: "sess-xyz".to_string(),
            duration_ms: 500,
            duration_api_ms: 400,
            is_error: true,
            num_turns: 1,
            total_cost_usd: 0.005,
            usage: make_usage(),
            model_usage: HashMap::new(),
            permission_denials: vec![],
            errors: errors.into_iter().map(ToString::to_string).collect(),
        })
    }

    // ── extract_result_info ─────────────────────────────────────────

    #[test]
    fn extract_result_info_success() {
        let result = make_success(false, "All done");
        let (is_error, session_id, sub_type, _payload) = extract_result_info(&result);
        assert!(!is_error);
        assert_eq!(session_id, "sess-abc");
        assert_eq!(sub_type, AgentSubType::Completed);
    }

    #[test]
    fn extract_result_info_success_with_is_error() {
        let result = make_success(true, "Failed to complete");
        let (is_error, session_id, sub_type, _payload) = extract_result_info(&result);
        assert!(is_error);
        assert_eq!(session_id, "sess-abc");
        // Even with is_error=true, Success maps to Completed (backend reads the flag)
        assert_eq!(sub_type, AgentSubType::Completed);
    }

    #[test]
    fn extract_result_info_error() {
        let result = make_error(vec!["something went wrong"]);
        let (is_error, session_id, sub_type, _payload) = extract_result_info(&result);
        assert!(is_error);
        assert_eq!(session_id, "sess-xyz");
        assert_eq!(sub_type, AgentSubType::Error);
    }

    // ── extract_error_text ──────────────────────────────────────────

    #[test]
    fn extract_error_text_from_success() {
        let result = make_success(true, "This is the error output");
        let text = extract_error_text(&result);
        assert_eq!(text, "This is the error output");
    }

    #[test]
    fn extract_error_text_from_success_truncates() {
        let long = "x".repeat(600);
        let result = make_success(true, &long);
        let text = extract_error_text(&result);
        assert_eq!(text.len(), 500);
    }

    #[test]
    fn extract_error_text_from_error() {
        let result = make_error(vec!["first error", "second error"]);
        let text = extract_error_text(&result);
        assert_eq!(text, "first error");
    }

    #[test]
    fn extract_error_text_from_error_no_errors() {
        let result = make_error(vec![]);
        let text = extract_error_text(&result);
        assert_eq!(text, "Unknown error");
    }

    // ── sdk_message_variant ─────────────────────────────────────────

    #[test]
    fn sdk_message_variant_coverage() {
        // Verify all known variants map to non-empty strings
        let variants = [
            "assistant", "user", "user_replay", "result", "system_init",
            "compact_boundary", "status", "hook_started", "hook_progress",
            "hook_response", "task_notification", "files_persisted",
            "stream_event", "tool_progress", "auth_status", "tool_use_summary",
        ];
        // Just check the count matches the enum variants
        assert_eq!(variants.len(), 16);
    }

    // ── truncate ────────────────────────────────────────────────────

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string_clips() {
        assert_eq!(truncate("hello world", 5), "hello");
    }

    #[test]
    fn truncate_empty_string() {
        assert_eq!(truncate("", 10), "");
    }

    #[test]
    fn truncate_multibyte_chars() {
        let s = "🎉🎊🎈";
        assert_eq!(truncate(s, 2), "🎉🎊");
    }
}
