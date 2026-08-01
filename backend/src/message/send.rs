//! Message sending orchestration — send_message_flow and send_tool_response_flow.

use super::models::{TaskMessage, TaskMessageContext};
use crate::message::models::{MessageRole, MessageType, TOOL_RESPONSE_USER_TYPE};
use crate::shared_models::AgentMode;
use crate::state::AppState;
use crate::task::dao::TaskPatch;
use crate::task::schemas::{FileContextData, MessageContextInput};
use crate::task_activities::mapping::{ActivityStatus, ConversationWrite, ToolOutcome};
use moatless_backend_core::scope::ScopeContext;

use super::contexts::message_context_input_to_row;
use super::error::MessageError;

/// Whether the task exists and is in `Closed` status. Fail-open on DB
/// errors: a DB blip must not silently drop a legitimate message.
async fn task_is_closed(state: &AppState, task_id: &str) -> bool {
    // reason: closed-task guard runs server-side without a user request; system ctx required.
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    match state.tasks.find_by_id(task_id, &sys_ctx).await {
        Ok(Some(task)) => task.status == crate::task::models::TaskStatus::Closed,
        Ok(None) => false,
        Err(e) => {
            tracing::warn!(
                task_id = %task_id,
                error = %e,
                "Failed to fetch task status for closed-task guard; proceeding"
            );
            false
        }
    }
}

/// The main message flow — called by message handlers, task creation, and external inbound.
#[allow(clippy::too_many_arguments)]
pub async fn send_message_flow(
    state: &AppState,
    task_id: &str,
    message: &str,
    model: Option<&str>,
    agent_mode: Option<&AgentMode>,
    file_ids: &[String],
    contexts: &[MessageContextInput],
    resume: bool,
    external_event_id: Option<&str>,
    external_adapter: Option<&str>,
    external_subject_key: Option<&str>,
    external_source_name: Option<&str>,
    source_task_id: Option<&str>,
    start_fresh: bool,
    sent_by: Option<&str>,
    // The Message's uuid, when the caller owns it. The T3 compatibility
    // surface mints a message id before it sends and keys its own optimistic
    // copy on it, so a server-minted uuid would leave that copy on screen
    // forever. Every other caller passes `None` and gets a fresh uuid.
    //
    // An id that arrives from a client is checked against the whole table
    // below before anything is written — see the guard on it.
    message_uuid: Option<String>,
) -> Result<Option<String>, MessageError> {
    // A closed task is terminal (no reopen path) so no send may persist a
    // message or wake the agent. Inbound external messages are dropped
    // silently — backstop for the close-vs-comment race; steady state,
    // inactive bindings already keep these out of routing. Internal/API sends
    // surface a 409 so the caller knows nothing was queued. `Error` tasks
    // stay sendable to match `start_task` recovery semantics.
    if task_is_closed(state, task_id).await {
        if let Some(adapter) = external_adapter {
            tracing::debug!(
                task_id = %task_id,
                external_adapter = %adapter,
                "Dropping inbound external message for closed task"
            );
            return Ok(None);
        }
        return Err(MessageError::TaskClosed(task_id.to_string()));
    }

    // Dedup: if this external message was already persisted (or was just sent
    // outbound and is racing the webhook), skip the insert entirely. Prevents
    // duplicates from Slack retries, re-delivered webhooks, and `moat reply`
    // self-echoes.
    if let Some(ext_id) = external_event_id {
        // 1. In-memory race-window check — catches webhooks that land before
        //    `set_external_event_id` has committed.
        if state.recently_sent.contains(task_id, ext_id) {
            tracing::debug!(
                task_id = %task_id,
                external_event_id = %ext_id,
                "Filtered self-echo via recently_sent cache"
            );
            return Ok(None);
        }

        // 2. DB check — tolerates both new prefixed format and any legacy
        //    bare-int rows written before the prefix scheme.
        let candidates = external_adapter
            .and_then(|provider_name| {
                state
                    .adapters
                    .as_ref()
                    .and_then(|providers| providers.inbound_presentation_by_name(provider_name))
            })
            .map_or_else(
                || vec![ext_id.to_string()],
                |presentation| presentation.external_id_aliases(ext_id),
            );
        let exists = state
            .messages
            .has_any_external_event_id(task_id, &candidates)
            .await
            .map_err(MessageError::DedupCheck)?;
        if exists {
            tracing::debug!(task_id = %task_id, external_event_id = %ext_id, "Duplicate external message, skipping");
            return Ok(None);
        }
    }

    // A caller-owned id addresses a Message across the whole table, not within
    // this Task, and the insert below upserts on that column: it is the merge
    // path that assembles one agent Message from its blocks. So an id already
    // in use is one of two things, and they have to be told apart before the
    // write rather than after it.
    //
    // This Task's own user row means the frame arrived twice — a reconnecting
    // T3 client replaying what it could not confirm — and must read as sent, or
    // the person sees their message fail and sends it again. Anything else names
    // a Message the sender has no claim on: writing through would rewrite that
    // conversation's history from a Task the sender does own, leaving the
    // sender's own conversation empty and the victim's carrying text nobody
    // with access to it wrote.
    //
    // The replay case is deliberately not narrowed to identical text. A client
    // reusing its own id for a different message is a bug on its side, and
    // answering it the way a true replay is answered writes nothing either way.
    if let Some(uuid) = message_uuid.as_deref() {
        let existing = state
            .messages
            .find_by_uuid(uuid)
            .await
            .map_err(MessageError::DedupCheck)?;
        if let Some(existing) = existing {
            if existing.task_id == task_id && existing.message_type == MessageType::User {
                tracing::debug!(task_id = %task_id, uuid = %uuid, "Message already persisted; treating as sent");
                return Ok(None);
            }
            tracing::warn!(
                task_id = %task_id,
                uuid = %uuid,
                addressed = %existing.task_id,
                "Refusing a send whose message id is already in use"
            );
            return Err(MessageError::BadRequest(
                "That message id is already in use.".to_string(),
            ));
        }
    }

    let effective_message = message.to_string();

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = message_uuid.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Persist the attached file IDs so the sandbox can fetch them via gRPC
    // before invoking the agent.
    let attachments = if file_ids.is_empty() {
        None
    } else {
        Some(serde_json::json!({ "file_ids": file_ids }))
    };

    let msg_row = TaskMessage {
        id: 0, // will be assigned by DB
        task_id: task_id.to_string(),
        uuid: msg_uuid,
        message_type: MessageType::User,
        user_type: "human".to_string(),
        parent_uuid: None,
        is_sidechain: None,
        cwd: None,
        session_id: None,
        // Queued user rows carry no turn until the sandbox acknowledges them:
        // the run that consumes them decides which turn they belong to.
        turn_number: None,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: now.clone(),
        message_role: MessageRole::User,
        // What the person wrote, and only that. The preview URL, the selected
        // element, the attribution and the attachments are rebuilt from their
        // own rows on the way to the agent
        // (`enricher::build_pending_content_blocks`).
        message_text: Some(effective_message),
        assistant_message_id: None,
        assistant_message_type: None,
        assistant_message_model: None,
        assistant_stop_reason: None,
        assistant_stop_sequence: None,
        assistant_usage_input_tokens: None,
        assistant_usage_output_tokens: None,
        assistant_usage_cache_creation_input_tokens: None,
        assistant_usage_cache_read_input_tokens: None,
        assistant_usage: None,
        tool_use_result: None,
        tool_response: None,
        sdk_metadata: None,
        attachments,
        agent_session_id: None, // Not yet acknowledged
        will_not_resume: None,
        run_metrics: None,
        agent_mode: agent_mode.cloned(),
        model: model.map(std::string::ToString::to_string),
        resume,
        sent_at: None,
        created_at: None,
        external_event_id: external_event_id.map(std::string::ToString::to_string),
        external_adapter: external_adapter.map(std::string::ToString::to_string),
        external_subject_key: external_subject_key.map(std::string::ToString::to_string),
        external_source_name: external_source_name.map(std::string::ToString::to_string),
        source_task_id: source_task_id.map(std::string::ToString::to_string),
        start_fresh,
        sent_by: sent_by.map(std::string::ToString::to_string),
    };

    // Persist contexts: sender-supplied (preview/element) + synthesised
    // file contexts so the read path can surface uploaded files alongside the
    // other context items. The `attachments` JSON column is still written
    // above for the sandbox gRPC pre-fetch path.
    let mut all_contexts: Vec<MessageContextInput> = contexts.to_vec();
    for file_id in file_ids {
        let data = match state.files.find_by_id(file_id).await {
            Ok(Some(meta)) => FileContextData {
                filename: meta.filename,
                mime_type: meta.mime_type,
                size: meta.size,
            },
            Ok(None) => {
                tracing::warn!(
                    file_id,
                    "File metadata not found when synthesising File context"
                );
                FileContextData::default()
            }
            Err(e) => {
                tracing::warn!(file_id, error = %e, "Failed to load file metadata for File context");
                FileContextData::default()
            }
        };
        all_contexts.push(MessageContextInput::File {
            reference_id: file_id.clone(),
            data,
        });
    }

    let context_rows: Vec<TaskMessageContext> = all_contexts
        .iter()
        .map(message_context_input_to_row)
        .collect();

    state
        .messages
        .append(&msg_row, &context_rows, &ConversationWrite::default())
        .await
        .map_err(MessageError::PersistMessage)?;

    tracing::info!(task_id = %task_id, "Message persisted");

    // Advance the conversation clock. The persisted pending row is the
    // reconciler's durable wake input after idle/user stops; sandbox intent is
    // written only by lifecycle (through the ensure-and-notify start path),
    // never here.
    // reason: conversation-clock bookkeeping; the send itself was already
    // read-authorized (public tasks accept messages from non-owners).
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    state
        .tasks
        .update_task(
            task_id,
            &TaskPatch {
                last_message_at: Some(now),
                model: model.map(std::string::ToString::to_string),
                ..Default::default()
            },
            &sys_ctx,
        )
        .await
        .map_err(MessageError::AdvanceTaskClock)?;

    Ok(Some(msg_row.uuid))
}

/// Persist an error result message visible in the chat.
///
/// Used when the message was saved but a subsequent operation (e.g. sandbox start) failed.
/// The error text is prefixed to make it clear to both the UI and the agent that this
/// is a system/harness error, not user input.
/// Fire-and-forget: logs on failure but never propagates errors.
pub async fn persist_error_result(state: &AppState, task_id: &str, error_text: &str) {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = uuid::Uuid::new_v4().to_string();

    let turn_number = match state.task_turns.latest_turn_number(task_id).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(task_id, error = %e, "Failed to read current turn number for error result");
            None
        }
    };

    let msg_row = TaskMessage {
        id: 0,
        task_id: task_id.to_string(),
        uuid: msg_uuid,
        message_type: MessageType::Error,
        user_type: "system".to_string(),
        parent_uuid: None,
        is_sidechain: None,
        cwd: None,
        session_id: None,
        turn_number,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: now,
        message_role: MessageRole::Assistant,
        message_text: Some(error_text.to_string()),
        assistant_message_id: None,
        assistant_message_type: None,
        assistant_message_model: None,
        assistant_stop_reason: None,
        assistant_stop_sequence: None,
        assistant_usage_input_tokens: None,
        assistant_usage_output_tokens: None,
        assistant_usage_cache_creation_input_tokens: None,
        assistant_usage_cache_read_input_tokens: None,
        assistant_usage: None,
        tool_use_result: None,
        tool_response: None,
        sdk_metadata: None,
        attachments: None,
        agent_session_id: None,
        will_not_resume: Some(false),
        run_metrics: None,
        agent_mode: None,
        model: None,
        resume: true,
        sent_at: None,
        created_at: None,
        external_event_id: None,
        external_adapter: None,
        external_subject_key: None,
        external_source_name: None,
        source_task_id: None,
        start_fresh: false,
        sent_by: None,
    };

    if let Err(e) = state
        .messages
        .append(&msg_row, &[], &ConversationWrite::default())
        .await
    {
        tracing::error!(task_id = %task_id, error = %e, "Failed to persist error result message");
    }
}

/// The question and the answer, as one thing to say to an agent that has
/// forgotten asking.
///
/// The question comes off the call's own stored input; `asked` is `None` where
/// that row is gone, and then only the answer survives — still better than
/// nothing, and the only case where the restatement is partial.
fn restate(
    tool_name: &str,
    asked: Option<&serde_json::Value>,
    answer: &serde_json::Value,
) -> String {
    use std::fmt::Write as _;

    let mut text = String::new();
    if tool_name == crate::task_activities::mapping::PLAN_TOOL {
        let approved = answer.get("approved").and_then(serde_json::Value::as_bool) == Some(true);
        if let Some(plan) = asked
            .and_then(|input| input.get("plan"))
            .and_then(serde_json::Value::as_str)
        {
            let _ = writeln!(text, "You proposed this plan:\n\n{plan}\n");
        }
        if approved {
            text.push_str("The plan is approved. Carry it out.");
            return text;
        }
        text.push_str("The plan is rejected.");
        if let Some(feedback) = answer
            .get("feedback")
            .and_then(serde_json::Value::as_str)
            .filter(|feedback| !feedback.trim().is_empty())
        {
            let _ = write!(text, " Feedback: {feedback}");
        }
        return text;
    }

    let answers = answer.get("answers").and_then(serde_json::Value::as_object);
    let asked_questions = asked
        .and_then(|input| input.get("questions"))
        .and_then(serde_json::Value::as_array);
    text.push_str("You asked, and the answer is:\n");
    match answers {
        Some(answers) if !answers.is_empty() => {
            for (question, given) in answers {
                let _ = writeln!(
                    text,
                    "\n- {question}\n  {}",
                    given.as_str().unwrap_or_default()
                );
            }
        }
        // Nothing keyed by question. Say what was asked so the agent can ask
        // again rather than proceed on an answer it cannot read.
        _ => {
            for question in asked_questions.into_iter().flatten() {
                if let Some(text_of) = question.get("question").and_then(serde_json::Value::as_str)
                {
                    let _ = writeln!(text, "\n- {text_of}\n  (no answer given)");
                }
            }
        }
    }
    text
}

/// Send a tool response (AskUserQuestion / ExitPlanMode).
///
/// The answer leaves here twice, in two shapes, because two paths read it. The
/// structured one on `tool_response` is what the sandbox delivers to a run still
/// parked on the call — an answer per question, a rejection as a rejection. The
/// text in `message_text` is what the agent is *told* when no run is parked:
/// the question restated with the answer, because a resumed session has no
/// memory of having asked. The CLI drops an unanswered call on resume, and the
/// text around it with it, so a bare "Option A" would answer nothing.
pub async fn send_tool_response_flow(
    state: &AppState,
    task_id: &str,
    tool_use_id: &str,
    tool_name: &str,
    tool_response: Option<&serde_json::Value>,
    sent_by: Option<&str>,
) -> Result<(), MessageError> {
    // Same closed-task guard as `send_message_flow`. Tool responses only
    // arrive via the HTTP handler, so a closed task always surfaces the 409.
    if task_is_closed(state, task_id).await {
        return Err(MessageError::TaskClosed(task_id.to_string()));
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let msg_uuid = uuid::Uuid::new_v4().to_string();

    let answer = tool_response.cloned().unwrap_or(serde_json::Value::Null);
    // The question as it was asked, off the call's own stored input.
    let asked = crate::task_activities::dao::find_tool_call(&state.db, task_id, tool_use_id)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(task_id, tool_use_id, %error, "Could not read the call being answered");
            None
        })
        .map(|call| call.tool_input().clone());
    let response_text = restate(tool_name, asked.as_ref(), &answer);
    let mut structured = serde_json::Map::new();
    structured.insert("toolUseId".into(), tool_use_id.into());
    structured.insert("toolName".into(), tool_name.into());
    for (key, value) in answer.as_object().into_iter().flatten() {
        structured.insert(key.clone(), value.clone());
    }

    let msg_row = TaskMessage {
        id: 0,
        task_id: task_id.to_string(),
        uuid: msg_uuid,
        message_type: MessageType::User,
        // Not `human`, even though a person typed it. This row exists to hand a
        // tool response back to the agent; the answer itself is already on
        // screen as the outcome of the call that asked, and rendering the
        // carrier too would show it twice.
        user_type: TOOL_RESPONSE_USER_TYPE.to_string(),
        parent_uuid: None,
        is_sidechain: None,
        cwd: None,
        session_id: None,
        // Queued user rows carry no turn until the sandbox acknowledges them:
        // the run that consumes them decides which turn they belong to.
        turn_number: None,
        version: None,
        git_branch: None,
        request_id: None,
        timestamp: now,
        message_role: MessageRole::User,
        message_text: Some(response_text.clone()),
        assistant_message_id: None,
        assistant_message_type: None,
        assistant_message_model: None,
        assistant_stop_reason: None,
        assistant_stop_sequence: None,
        assistant_usage_input_tokens: None,
        assistant_usage_output_tokens: None,
        assistant_usage_cache_creation_input_tokens: None,
        assistant_usage_cache_read_input_tokens: None,
        assistant_usage: None,
        tool_use_result: None,
        tool_response: Some(serde_json::Value::Object(structured)),
        sdk_metadata: None,
        attachments: None,
        agent_session_id: None,
        will_not_resume: None,
        run_metrics: None,
        agent_mode: None,
        model: None,
        resume: true,
        sent_at: None,
        created_at: None,
        external_event_id: None,
        external_adapter: None,
        external_subject_key: None,
        external_source_name: None,
        source_task_id: None,
        start_fresh: false,
        sent_by: sent_by.map(std::string::ToString::to_string),
    };

    // The person's answer settles the call that asked for it. Without this the
    // `AskUserQuestion` activity stays `in_progress` forever and the question
    // keeps rendering as unanswered — the agent's own tool result never arrives
    // for these, because this row *is* the result.
    //
    // The resolved row beside it is what closes the panel: the client keeps a
    // question open until it sees one naming the same `requestId`.
    let conversation = ConversationWrite {
        outcomes: vec![ToolOutcome {
            task_id: task_id.to_string(),
            tool_use_id: tool_use_id.to_string(),
            status: ActivityStatus::Completed,
            result: if answer.is_null() {
                serde_json::Value::String(response_text.clone())
            } else {
                answer.clone()
            },
            typed_result: None,
            is_error: false,
        }],
        activities: vec![crate::task_activities::mapping::user_input_resolved(
            task_id,
            &msg_row.uuid,
            tool_use_id,
            &answer,
        )],
        ..Default::default()
    };

    let contexts: Vec<TaskMessageContext> = vec![];
    state
        .messages
        .append(&msg_row, &contexts, &conversation)
        .await
        .map_err(MessageError::PersistToolResponse)?;

    tracing::info!(task_id = %task_id, tool_use_id = %tool_use_id, "Tool response sent");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A resumed session has no memory of having asked: the CLI drops an
    /// unanswered call on resume and the text around it with it. So the answer
    /// travels with the question restated around it.
    #[test]
    fn an_answer_carries_the_question_it_answers() {
        let asked = json!({ "questions": [{ "question": "Which store?" }] });
        let text = restate(
            crate::task_activities::mapping::QUESTION_TOOL,
            Some(&asked),
            &json!({ "answers": { "Which store?": "Postgres" } }),
        );
        assert!(text.contains("Which store?"), "{text}");
        assert!(text.contains("Postgres"), "{text}");
    }

    /// The call's row is gone. Only the answer survives, which is still better
    /// than nothing said at all.
    #[test]
    fn an_answer_to_a_call_no_longer_stored_still_says_what_was_answered() {
        let text = restate(
            crate::task_activities::mapping::QUESTION_TOOL,
            None,
            &json!({ "answers": { "Which store?": "Postgres" } }),
        );
        assert!(text.contains("Which store?"), "{text}");
        assert!(text.contains("Postgres"), "{text}");
    }

    /// An answer nothing can be read out of asks again rather than letting the
    /// agent proceed on one it cannot read.
    #[test]
    fn an_unreadable_answer_restates_the_question_unanswered() {
        let asked = json!({ "questions": [{ "question": "Which store?" }] });
        let text = restate(
            crate::task_activities::mapping::QUESTION_TOOL,
            Some(&asked),
            &json!({}),
        );
        assert!(text.contains("Which store?"), "{text}");
        assert!(text.contains("no answer given"), "{text}");
    }

    #[test]
    fn an_approved_plan_is_restated_and_approved() {
        let asked = json!({ "plan": "1. Do the thing" });
        let text = restate(
            crate::task_activities::mapping::PLAN_TOOL,
            Some(&asked),
            &json!({ "approved": true }),
        );
        assert!(text.contains("1. Do the thing"), "{text}");
        assert!(text.contains("approved"), "{text}");
    }

    /// "No, use the second approach" is a rejection *and* the reason for it —
    /// the agent needs both to plan again.
    #[test]
    fn a_rejected_plan_carries_the_feedback_that_rejected_it() {
        let text = restate(
            crate::task_activities::mapping::PLAN_TOOL,
            Some(&json!({ "plan": "1. Do the thing" })),
            &json!({ "approved": false, "feedback": "use the second approach" }),
        );
        assert!(text.contains("rejected"), "{text}");
        assert!(text.contains("use the second approach"), "{text}");
    }

    /// Absent `approved` is not approval.
    #[test]
    fn a_verdict_that_says_nothing_is_not_an_approval() {
        let text = restate(
            crate::task_activities::mapping::PLAN_TOOL,
            None,
            &serde_json::Value::Null,
        );
        assert!(text.contains("rejected"), "{text}");
    }
}
