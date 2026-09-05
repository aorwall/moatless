//! Message sending orchestration — `send_message_flow` and `send_tool_response_flow`.

use std::sync::Arc;

use super::models::{TaskMessage, TaskMessageContext};
use crate::message::dao::MessageDao;
use crate::message::models::{MessageRole, MessageType, TOOL_RESPONSE_USER_TYPE};
use crate::task::dao::{TaskDao, TaskPatch};
use crate::task::schemas::{FileContextData, MessageContextInput};
use crate::task_activities::mapping::{ActivityStatus, ConversationWrite, ToolOutcome};
use crate::task_turn::dao::TaskTurnDao;
use moatless_core::file_storage::dao::FileDao;
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::AgentMode;

use super::contexts::message_context_input_to_row;
use super::error::MessageError;
use moatless_types::task::ExternalOrigin;

/// The message domain's one question of the adapter registry: given an inbound
/// event, has it already been sent from here, and what ids could it be stored
/// under. Send flows read this port, never `Adapters`. The filter holding the
/// in-memory race-window cache and the registry lives up-band in
/// `adapters/self_echo.rs`.
pub trait SelfEchoGuard: Send + Sync {
    /// In-memory race-window check — catches webhooks that land before
    /// `set_external_event_id` has committed.
    fn sent_recently(&self, task_id: &str, ext_id: &str) -> bool;

    /// Given an adapter name and an external id, the ids this event could be
    /// stored under — the adapter's inbound presentation aliases, or the bare
    /// id when no adapter is known. Tolerates both the prefixed format and any
    /// legacy bare-int rows.
    fn candidate_ids(&self, adapter: Option<&str>, ext_id: &str) -> Vec<String>;
}

/// The ports the message-send flows read, held together so each flow is a
/// method on one value instead of a function threading `&AppState`.
pub struct MessageSendService {
    messages: Arc<dyn MessageDao>,
    tasks: Arc<dyn TaskDao>,
    task_turns: Arc<dyn TaskTurnDao>,
    files: Arc<dyn FileDao>,
    self_echo: Arc<dyn SelfEchoGuard>,
}

impl MessageSendService {
    pub fn new(
        messages: Arc<dyn MessageDao>,
        tasks: Arc<dyn TaskDao>,
        task_turns: Arc<dyn TaskTurnDao>,
        files: Arc<dyn FileDao>,
        self_echo: Arc<dyn SelfEchoGuard>,
    ) -> Self {
        Self {
            messages,
            tasks,
            task_turns,
            files,
            self_echo,
        }
    }

    /// Whether the task exists and is in `Closed` status. Fail-open on DB
    /// errors: a DB blip must not silently drop a legitimate message.
    async fn task_is_closed(&self, task_id: &str) -> bool {
        // reason: closed-task guard runs server-side without a user request; system ctx required.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        match self.tasks.find_by_id(task_id, &sys_ctx).await {
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
}

/// Fixed namespace for agent→task send idempotency uuids. Arbitrary but stable —
/// changing it would stop old rows from deduping against new resends.
const AGENT_SEND_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x9b1e_5c0e_4a7d_4f21_8f3a_2c6b_0d5e_7a91);

/// Deterministic uuid for an agent→task send, so an identical resend from the
/// same source task dedups on the `message_uuid` guard. Keyed on source +
/// destination + text (unit-separated so the fields can't run together), which
/// makes it unique to this (child, parent, message) triple and unable to collide
/// with any other task's row.
fn agent_send_idempotency_uuid(source_task_id: &str, task_id: &str, text: &str) -> String {
    let name = format!("{source_task_id}\u{1f}{task_id}\u{1f}{text}");
    uuid::Uuid::new_v5(&AGENT_SEND_NAMESPACE, name.as_bytes()).to_string()
}

fn attached_file_ids(file_ids: &[String], contexts: &[MessageContextInput]) -> Vec<String> {
    let mut attached = Vec::new();

    for file_id in file_ids {
        if !file_id.is_empty() && !attached.contains(file_id) {
            attached.push(file_id.clone());
        }
    }

    for context in contexts {
        match context {
            MessageContextInput::File { reference_id, .. }
                if !reference_id.is_empty() && !attached.contains(reference_id) =>
            {
                attached.push(reference_id.clone());
            }
            _ => {}
        }
    }

    attached
}

impl MessageSendService {
    /// The main message flow — called by message handlers, task creation, and external inbound.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_message_flow(
        &self,
        task_id: &str,
        message: &str,
        model: Option<&str>,
        agent_mode: Option<&AgentMode>,
        effort: Option<&str>,
        file_ids: &[String],
        contexts: &[MessageContextInput],
        resume: bool,
        origin: ExternalOrigin<'_>,
        start_fresh: bool,
        sent_by: Option<&str>,
        // Skills the sandbox activates for this turn. Names only; the SKILL.md
        // body is resolved from disk in the pod.
        skills: &[String],
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
        if self.task_is_closed(task_id).await {
            if let Some(adapter) = origin.adapter {
                tracing::debug!(
                    task_id = %task_id,
                    external_adapter = %adapter,
                    "Dropping inbound external message for closed task"
                );
                return Ok(None);
            }
            return Err(MessageError::TaskClosed(task_id.to_string()));
        }

        // Idempotency for agent→task sends. Any `moat send` from inside a sandbox
        // (a child reporting to its parent, most notably) carries a source_task_id.
        // Give it a deterministic uuid keyed on (source, destination, text) so an
        // identical resend — a report the agent retried because the first looked
        // like it failed — collapses onto the first row via the message_uuid guard
        // below instead of landing twice. Human sends carry no source_task_id and
        // keep a fresh random uuid; an explicit caller-owned uuid (the T3 surface)
        // always wins over this derivation.
        let message_uuid = message_uuid.or_else(|| {
            origin
                .source_task_id
                .map(|source| agent_send_idempotency_uuid(source, task_id, message))
        });

        // Dedup: if this external message was already persisted (or was just sent
        // outbound and is racing the webhook), skip the insert entirely. Prevents
        // duplicates from Slack retries, re-delivered webhooks, and `moat reply`
        // self-echoes.
        if let Some(ext_id) = origin.event_id {
            // 1. In-memory race-window check — catches webhooks that land before
            //    `set_external_event_id` has committed.
            if self.self_echo.sent_recently(task_id, ext_id) {
                tracing::debug!(
                    task_id = %task_id,
                    external_event_id = %ext_id,
                    "Filtered self-echo via recently_sent cache"
                );
                return Ok(None);
            }

            // 2. DB check — tolerates both new prefixed format and any legacy
            //    bare-int rows written before the prefix scheme.
            let candidates = self.self_echo.candidate_ids(origin.adapter, ext_id);
            let exists = self
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
            let existing = self
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
        let normalized_file_ids = attached_file_ids(file_ids, contexts);

        // Persist the attached file IDs so the sandbox can fetch them via gRPC
        // before invoking the agent.
        let attachments = if normalized_file_ids.is_empty() {
            None
        } else {
            Some(serde_json::json!({ "file_ids": &normalized_file_ids }))
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
            agent_mode_raw: agent_mode.map(|m| m.as_str().to_string()),
            model: model.map(std::string::ToString::to_string),
            effort: effort.map(std::string::ToString::to_string),
            skills: skills.to_vec(),
            resume,
            sent_at: None,
            created_at: None,
            external_event_id: origin.event_id.map(std::string::ToString::to_string),
            external_adapter: origin.adapter.map(std::string::ToString::to_string),
            external_subject_key: origin.subject_key.map(std::string::ToString::to_string),
            external_source_name: origin.source_name.map(std::string::ToString::to_string),
            external_author: origin.author.map(std::string::ToString::to_string),
            external_sender_id: origin.sender_id.map(std::string::ToString::to_string),
            source_task_id: origin.source_task_id.map(std::string::ToString::to_string),
            start_fresh,
            sent_by: sent_by.map(std::string::ToString::to_string),
        };

        // Persist contexts: sender-supplied (preview/element) + synthesised
        // file contexts so the read path can surface uploaded files alongside the
        // other context items. The `attachments` JSON column is still written
        // above for the sandbox gRPC pre-fetch path.
        let mut all_contexts: Vec<MessageContextInput> = contexts
            .iter()
            .filter(|context| !matches!(context, MessageContextInput::File { .. }))
            .cloned()
            .collect();
        for file_id in &normalized_file_ids {
            let data = match self.files.find_by_id(file_id).await {
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

        self.messages
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
        self.tasks
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
    pub async fn persist_error_result(&self, task_id: &str, error_text: &str) {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let msg_uuid = uuid::Uuid::new_v4().to_string();

        let turn_number = match self.task_turns.latest_turn_number(task_id).await {
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
            agent_mode_raw: None,
            model: None,
            effort: None,
            skills: Vec::new(),
            resume: true,
            sent_at: None,
            created_at: None,
            external_event_id: None,
            external_adapter: None,
            external_subject_key: None,
            external_source_name: None,
            external_author: None,
            external_sender_id: None,
            source_task_id: None,
            start_fresh: false,
            sent_by: None,
        };

        if let Err(e) = self
            .messages
            .append(&msg_row, &[], &ConversationWrite::default())
            .await
        {
            tracing::error!(task_id = %task_id, error = %e, "Failed to persist error result message");
        }
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

impl MessageSendService {
    /// Send a tool response (`AskUserQuestion` / `ExitPlanMode`).
    ///
    /// The answer leaves here twice, in two shapes, because two paths read it. The
    /// structured one on `tool_response` is what the sandbox delivers to a run still
    /// parked on the call — an answer per question, a rejection as a rejection. The
    /// text in `message_text` is what the agent is *told* when no run is parked:
    /// the question restated with the answer, because a resumed session has no
    /// memory of having asked. The CLI drops an unanswered call on resume, and the
    /// text around it with it, so a bare "Option A" would answer nothing.
    pub async fn send_tool_response_flow(
        &self,
        task_id: &str,
        tool_use_id: &str,
        tool_name: &str,
        tool_response: Option<&serde_json::Value>,
        sent_by: Option<&str>,
    ) -> Result<(), MessageError> {
        // Same closed-task guard as `send_message_flow`. Tool responses only
        // arrive via the HTTP handler, so a closed task always surfaces the 409.
        if self.task_is_closed(task_id).await {
            return Err(MessageError::TaskClosed(task_id.to_string()));
        }

        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let msg_uuid = uuid::Uuid::new_v4().to_string();

        let answer = tool_response.cloned().unwrap_or(serde_json::Value::Null);
        // The question as it was asked, off the call's own stored input.
        let asked = self
            .messages
            .find_tool_call(task_id, tool_use_id)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(task_id, tool_use_id, %error, "Could not read the call being answered");
                None
            })
            .map(|call| call.tool_input);
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
            agent_mode_raw: None,
            model: None,
            effort: None,
            skills: Vec::new(),
            resume: true,
            sent_at: None,
            created_at: None,
            external_event_id: None,
            external_adapter: None,
            external_subject_key: None,
            external_source_name: None,
            external_author: None,
            external_sender_id: None,
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
        self.messages
            .append(&msg_row, &contexts, &conversation)
            .await
            .map_err(MessageError::PersistToolResponse)?;

        tracing::info!(task_id = %task_id, tool_use_id = %tool_use_id, "Tool response sent");

        Ok(())
    }
}

/// Settle the interactive call that an ordinary message answered.
///
/// Writing in the chat box answers a parked question as much as the panel does,
/// but that message carries no tool response, so nothing on its way in settles
/// the call: the outcome and the resolved row beside it are otherwise written
/// only by `send_tool_response_flow`. Without them the call stays `in_progress`
/// and the panel keeps asking a question that has been answered.
///
/// The answer recorded is what the run was told — a question hears the text as
/// its freeform response, a proposed plan hears a rejection carrying it. Both
/// rows are keyed on the message that answered, so a redelivered acknowledgement
/// rewrites the same rows.
pub async fn settle_answered_prompt(
    messages: &dyn crate::message::dao::MessageDao,
    task_id: &str,
    tool_use_id: &str,
    carrier_uuid: &str,
) {
    use crate::task_activities::mapping::{PLAN_TOOL, QUESTION_TOOL, user_input_resolved};

    let call = match messages.find_tool_call(task_id, tool_use_id).await {
        Ok(Some(call)) => call,
        Ok(None) => {
            // The block carrying the call has not reached the backend. An
            // outcome would settle nothing, and the panel it would close does
            // not exist either.
            tracing::warn!(task_id, tool_use_id, "Answered call has no stored row");
            return;
        }
        Err(error) => {
            tracing::warn!(task_id, tool_use_id, %error, "Could not read the answered call");
            return;
        }
    };
    if call.is_error.is_some() {
        return;
    }

    let text = match messages.find_by_uuid(carrier_uuid).await {
        Ok(Some(msg)) => msg.message_text.unwrap_or_default(),
        Ok(None) => String::new(),
        Err(error) => {
            tracing::warn!(task_id, carrier_uuid, %error, "Could not read the answer");
            String::new()
        }
    };

    let answer = match call.tool_name.as_str() {
        QUESTION_TOOL => serde_json::json!({ "response": text }),
        PLAN_TOOL => serde_json::json!({ "approved": false, "feedback": text }),
        other => {
            tracing::warn!(
                task_id,
                tool_use_id,
                other,
                "Not a call a message can answer"
            );
            return;
        }
    };

    let conversation = ConversationWrite {
        outcomes: vec![ToolOutcome {
            task_id: task_id.to_string(),
            tool_use_id: tool_use_id.to_string(),
            status: ActivityStatus::Completed,
            result: answer.clone(),
            typed_result: None,
            is_error: false,
        }],
        activities: vec![user_input_resolved(
            task_id,
            carrier_uuid,
            tool_use_id,
            &answer,
        )],
        ..Default::default()
    };
    if let Err(error) = messages
        .append_activities(task_id, carrier_uuid, &conversation)
        .await
    {
        tracing::warn!(task_id, tool_use_id, %error, "Could not settle the answered call");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn idempotency_uuid_is_stable_for_identical_agent_send() {
        // The same child reporting the same text to the same parent twice must
        // produce the same uuid, so the resend dedups onto the first row.
        let a = agent_send_idempotency_uuid("child", "parent", "phase 1 done");
        let b = agent_send_idempotency_uuid("child", "parent", "phase 1 done");
        assert_eq!(a, b);
        // And it is a well-formed uuid.
        assert!(uuid::Uuid::parse_str(&a).is_ok());
    }

    #[test]
    fn idempotency_uuid_differs_on_text_source_or_destination() {
        let base = agent_send_idempotency_uuid("child", "parent", "phase 1 done");
        // A reworded resend (e.g. corrupted vs clean) is a distinct message and
        // must not collapse onto the first.
        assert_ne!(
            base,
            agent_send_idempotency_uuid("child", "parent", "phase 1 done.")
        );
        // Different sender or recipient can never collide.
        assert_ne!(
            base,
            agent_send_idempotency_uuid("other", "parent", "phase 1 done")
        );
        assert_ne!(
            base,
            agent_send_idempotency_uuid("child", "elsewhere", "phase 1 done")
        );
        // The unit separator keeps field boundaries unambiguous.
        assert_ne!(
            agent_send_idempotency_uuid("a", "bc", "d"),
            agent_send_idempotency_uuid("ab", "c", "d")
        );
    }

    #[test]
    fn attached_file_ids_include_file_context_references() {
        let contexts = vec![MessageContextInput::File {
            reference_id: "ctx-file".to_string(),
            data: FileContextData {
                filename: "screenshot.png".to_string(),
                mime_type: "image/png".to_string(),
                size: 128,
            },
        }];

        assert_eq!(attached_file_ids(&[], &contexts), vec!["ctx-file"]);
    }

    #[test]
    fn attached_file_ids_dedupe_explicit_and_context_references() {
        let explicit = vec!["explicit-file".to_string(), "ctx-file".to_string()];
        let contexts = vec![
            MessageContextInput::File {
                reference_id: "ctx-file".to_string(),
                data: FileContextData::default(),
            },
            MessageContextInput::PreviewUrl {
                reference_id: "preview".to_string(),
                data: crate::task::schemas::PreviewUrlContextData {
                    path: "/".to_string(),
                    url: None,
                },
            },
        ];

        assert_eq!(
            attached_file_ids(&explicit, &contexts),
            vec!["explicit-file", "ctx-file"]
        );
    }

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
