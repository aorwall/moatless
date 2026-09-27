//! Message sending orchestration — `send_message_flow` and
//! `send_tool_response_flow`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::conversation::projector::Projector;
use crate::inbox::{
    AdapterChannel, DeliveryOptions, ExternalSender, InboxContext, InboxItem, InboxOrigin,
    InboxService, NewInboxItem,
};
use crate::message::dao::MessageDao;
use crate::message::models::MessageType;
use crate::message::operations::Messages;
use crate::task::dao::TaskDao;
use crate::task::models::TaskPatch;
use crate::task::operations::TaskCommands;
use crate::task::schemas::{FileContextData, MessageContextInput};
use crate::task_activities::mapping::{ActivityStatus, ConversationWrite, ToolOutcome};
use crate::task_turn::dao::TaskTurnDao;
use moatless_core::file_storage::{FileInfo, Files};
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::AgentMode;

use super::contexts::message_context_input_to_row;
use super::error::MessageError;
use moatless_types::task::ExternalOrigin;

/// The message domain's one question of the adapter registry.
///
/// Given an inbound event: has it already been sent from here, and what ids
/// could it be stored under. Send flows read this port, never `Adapters`.
pub trait SelfEchoGuard: Send + Sync {
    /// In-memory race-window check — catches webhooks that land before
    /// `set_external_event_id` has committed.
    fn sent_recently(&self, task_id: &str, ext_id: &str) -> bool;

    /// Given an adapter name and an external id, the ids this event could be
    /// stored under — the adapter's inbound presentation aliases, or the bare
    /// id when no adapter is known.
    ///
    /// Tolerates both the prefixed format and legacy bare-int rows.
    fn candidate_ids(&self, adapter: Option<&str>, ext_id: &str) -> Vec<String>;
}

/// The ports the message-send flows read, held together so each flow is a
/// method on one value instead of a function threading `&AppState`.
pub struct MessageSendService {
    messages: Arc<dyn MessageDao>,
    tasks: Arc<dyn TaskDao>,
    task_turns: Arc<dyn TaskTurnDao>,
    files: Arc<Files>,
    self_echo: Arc<dyn SelfEchoGuard>,
    inbox: InboxService,
}

impl MessageSendService {
    pub(crate) fn new(
        messages: Arc<dyn MessageDao>,
        tasks: Arc<dyn TaskDao>,
        task_turns: Arc<dyn TaskTurnDao>,
        files: Arc<Files>,
        self_echo: Arc<dyn SelfEchoGuard>,
        inbox: InboxService,
    ) -> Self {
        Self {
            messages,
            tasks,
            task_turns,
            files,
            self_echo,
            inbox,
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

/// Fixed namespace for agent→task send idempotency uuids. Arbitrary but stable
/// — changing it would stop old rows from deduping against new resends.
const AGENT_SEND_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x9b1e_5c0e_4a7d_4f21_8f3a_2c6b_0d5e_7a91);

/// Deterministic uuid for an agent→task send, so an identical resend from the
/// same source task dedups on the `message_uuid` guard.
///
/// Keyed on source + destination + text (unit-separated so fields can't run
/// together), unique to this (child, parent, message) triple.
fn agent_send_idempotency_uuid(source_task_id: &str, task_id: &str, text: &str) -> String {
    let name = format!("{source_task_id}\u{1f}{task_id}\u{1f}{text}");
    uuid::Uuid::new_v5(&AGENT_SEND_NAMESPACE, name.as_bytes()).to_string()
}

/// Submits `item` and projects its Message in one transaction, so no send
/// path writes a Message itself. `None` when an item with its id exists.
pub(crate) async fn queue(
    inbox: &InboxService,
    item: NewInboxItem,
) -> Result<Option<InboxItem>, sqlx::Error> {
    let mut tx = inbox.begin().await?;
    let submitted = inbox.submit(&mut tx, item).await?;
    if let Some(item) = &submitted {
        Projector.on_inbox(&mut tx, item).await?;
    }
    tx.commit().await?;
    Ok(submitted)
}

/// Who sent a message, from what its send path knows.
fn inbox_origin(origin: &ExternalOrigin<'_>, sent_by: Option<&str>) -> InboxOrigin {
    let sent_by_owned = sent_by.map(str::to_string);
    if let Some(source_task_id) = origin.source_task_id {
        return InboxOrigin::Task {
            source_task_id: source_task_id.to_string(),
            sent_by: sent_by_owned,
        };
    }
    if let (Some(adapter), Some(event_id)) = (origin.adapter, origin.event_id) {
        return InboxOrigin::Adapter {
            channel: AdapterChannel {
                adapter: adapter.to_string(),
                external_message_id: event_id.to_string(),
                subject_key: origin.subject_key.map(str::to_string),
                source_name: origin.source_name.map(str::to_string),
            },
            sender: ExternalSender {
                id: origin.sender_id.map(str::to_string),
                author: origin.author.map(str::to_string),
            },
            sent_by: sent_by_owned,
        };
    }
    match sent_by_owned {
        Some(sent_by) => InboxOrigin::User { sent_by },
        None => InboxOrigin::System,
    }
}

/// A client-owned message id, which is the inbox item's id.
fn parse_message_id(uuid: &str) -> Result<uuid::Uuid, MessageError> {
    uuid.parse()
        .map_err(|_| MessageError::BadRequest("A message id must be a UUID.".to_string()))
}

/// Queue a user message a sandbox wrote for its own Task, so an interrupted
/// turn recovers — `agent_handle.send` alone leaves no row for
/// `find_interrupted_turn` to see.
///
/// `idempotency_key` collapses a retried call onto its first row; `None` keys
/// on `text`, which swallows repeat callers like `/loop` unless they pass their
/// own key. `Ok(None)` means already queued, or the Task is closed.
pub async fn enqueue_sandbox_message(
    messages: &Messages,
    tasks: &TaskCommands,
    task_id: &str,
    text: &str,
    idempotency_key: Option<&str>,
) -> Result<Option<String>, MessageError> {
    // reason: the pod authenticated as itself over gRPC; there is no user request behind this.
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();

    // A closed Task is terminal, and the wake would open a turn on a
    // conversation nobody will read.
    match tasks.find_by_id(task_id, &sys_ctx).await {
        Ok(Some(task)) if task.status == crate::task::models::TaskStatus::Closed => {
            tracing::debug!(task_id, "Dropping a sandbox message for a closed task");
            return Ok(None);
        }
        Ok(_) => {}
        Err(e) => return Err(MessageError::Db(e)),
    }

    let uuid = agent_send_idempotency_uuid(task_id, task_id, idempotency_key.unwrap_or(text));
    let item = NewInboxItem {
        id: parse_message_id(&uuid)?,
        task_id: task_id.to_string(),
        text: text.to_string(),
        file_ids: Vec::new(),
        // `resume: false`, deliberately: this is the last item of its batch,
        // and `enricher::build_pending_deliveries` reads the last one's `resume`
        // to decide whether the recovered turn rejoins the previous session.
        options: DeliveryOptions {
            resume: false,
            ..DeliveryOptions::default()
        },
        answers_tool_use_id: None,
        origin: InboxOrigin::Sandbox,
    };
    let Some(queued) = queue(messages.inbox(), item)
        .await
        .map_err(MessageError::PersistMessage)?
    else {
        tracing::debug!(task_id, uuid, "Sandbox message already queued");
        return Ok(None);
    };

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    tasks
        .update_task(
            task_id,
            &TaskPatch {
                last_message_at: Some(now),
                ..Default::default()
            },
            &sys_ctx,
        )
        .await
        .map_err(MessageError::AdvanceTaskClock)?;

    Ok(Some(queued.id.to_string()))
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

/// Pair each attached id with what the store holds for it, refusing the whole
/// send when it holds nothing for one: a message that reached the sandbox with
/// an id short of its bytes would name an attachment nothing can open.
fn attachments_or_refusal(
    attached: Vec<String>,
    known: &HashMap<String, FileInfo>,
) -> Result<Vec<(String, FileContextData)>, MessageError> {
    let missing: Vec<&str> = attached
        .iter()
        .filter(|id| !known.contains_key(*id))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(MessageError::BadRequest(format!(
            "No uploaded file for {}. An attachment is an upload id, not a path: upload \
             the file first, then attach the id the upload returns.",
            missing.join(", ")
        )));
    }

    Ok(attached
        .into_iter()
        .filter_map(|id| {
            let meta = known.get(&id)?;
            let data = FileContextData {
                filename: meta.filename.clone(),
                mime_type: meta.mime_type.clone(),
                size: meta.size,
            };
            Some((id, data))
        })
        .collect())
}

impl MessageSendService {
    /// What each attached file is, in the order the send attaches them, or the
    /// refusal that names the ids the file store does not hold.
    ///
    /// The sandbox fetches every id over gRPC before the agent runs, so an id
    /// with no upload behind it arrives there as `File metadata not found` with
    /// nothing written — after the call that took it reported a send.
    pub async fn attachment_metadata(
        &self,
        file_ids: &[String],
        contexts: &[MessageContextInput],
    ) -> Result<Vec<(String, FileContextData)>, MessageError> {
        let attached = attached_file_ids(file_ids, contexts);
        if attached.is_empty() {
            return Ok(Vec::new());
        }
        let known = self
            .files
            .metadata_for(&attached)
            .await
            .map_err(MessageError::AttachmentMetadata)?;
        attachments_or_refusal(attached, &known)
    }

    /// The main message flow — called by message handlers, task creation, and
    /// external inbound.
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
            let id = parse_message_id(uuid)?;
            let queued = self
                .inbox
                .find(id)
                .await
                .map_err(MessageError::DedupCheck)?
                .map(|item| item.task_id);
            let addressed = match queued {
                Some(task) => Some(task),
                // A Message sent before the inbox, or any other Message: an
                // agent's reply shares this id space.
                None => self
                    .messages
                    .find_by_uuid(uuid)
                    .await
                    .map_err(MessageError::DedupCheck)?
                    .map(|existing| {
                        if existing.message_type == MessageType::User {
                            existing.task_id
                        } else {
                            String::new()
                        }
                    }),
            };
            if let Some(addressed) = addressed {
                if addressed == task_id {
                    tracing::debug!(task_id = %task_id, uuid = %uuid, "Message already persisted; treating as sent");
                    return Ok(None);
                }
                tracing::warn!(
                    task_id = %task_id,
                    uuid = %uuid,
                    addressed = %addressed,
                    "Refusing a send whose message id is already in use"
                );
                return Err(MessageError::BadRequest(
                    "That message id is already in use.".to_string(),
                ));
            }
        }

        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let id = match message_uuid.as_deref() {
            Some(uuid) => parse_message_id(uuid)?,
            None => uuid::Uuid::new_v4(),
        };
        let attachments_meta = self.attachment_metadata(file_ids, contexts).await?;
        let normalized_file_ids: Vec<String> =
            attachments_meta.iter().map(|(id, _)| id.clone()).collect();

        // Sender-supplied contexts (preview, element) plus one per attached
        // file, so the read path surfaces uploads beside the other items.
        let mut all_contexts: Vec<MessageContextInput> = contexts
            .iter()
            .filter(|context| !matches!(context, MessageContextInput::File { .. }))
            .cloned()
            .collect();
        for (file_id, data) in attachments_meta {
            all_contexts.push(MessageContextInput::File {
                reference_id: file_id,
                data,
            });
        }

        // `text` is what the person wrote, and only that. The preview URL, the
        // selected element, the attribution and the attachments are rebuilt
        // from their own fields on the way to the agent
        // (`enricher::build_pending_content_blocks`).
        let item = NewInboxItem {
            id,
            task_id: task_id.to_string(),
            text: message.to_string(),
            file_ids: normalized_file_ids,
            options: DeliveryOptions {
                model: model.map(str::to_string),
                agent_mode: agent_mode.map(|m| m.as_str().to_string()),
                effort: effort.map(str::to_string),
                skills: skills.to_vec(),
                resume,
                start_fresh,
                contexts: all_contexts
                    .iter()
                    .map(|context| InboxContext::from(&message_context_input_to_row(context)))
                    .collect(),
                tool_response: None,
            },
            answers_tool_use_id: None,
            origin: inbox_origin(&origin, sent_by),
        };
        let Some(queued) = queue(&self.inbox, item)
            .await
            .map_err(MessageError::PersistMessage)?
        else {
            tracing::debug!(task_id = %task_id, %id, "Message already queued");
            return Ok(None);
        };

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

        Ok(Some(queued.id.to_string()))
    }

    /// Persist an error result message visible in the chat.
    ///
    /// Used when the message was saved but a subsequent operation (e.g. sandbox
    /// start) failed. Fire-and-forget: logs failure but never propagates it.
    pub async fn persist_error_result(&self, task_id: &str, error_text: &str) {
        let turn_number = match self.task_turns.latest_turn_number(task_id).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(task_id, error = %e, "Failed to read current turn number for error result");
                None
            }
        };
        if let Err(e) = self
            .messages
            .append_error_result(task_id, turn_number, error_text)
            .await
        {
            tracing::error!(task_id = %task_id, error = %e, "Failed to persist error result message");
        }
    }
}

/// The question and the answer, as one thing to say to an agent that has
/// forgotten asking.
///
/// `asked` is `None` when the call's stored input row is gone; only the answer
/// survives then — partial, but better than nothing.
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
    /// Writes the answer twice: structured on `tool_response` for a run still
    /// parked on the call, and as restated `message_text` for a resumed session
    /// with no memory of asking — the CLI drops an unanswered call (and its
    /// text) on resume, so a bare "Option A" would answer nothing.
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

        // The Projector settles the call this answers, from `tool_response`.
        let item = NewInboxItem {
            id: uuid::Uuid::new_v4(),
            task_id: task_id.to_string(),
            text: response_text,
            file_ids: Vec::new(),
            options: DeliveryOptions {
                tool_response: Some(serde_json::Value::Object(structured)),
                ..DeliveryOptions::default()
            },
            answers_tool_use_id: Some(tool_use_id.to_string()),
            origin: match sent_by {
                Some(sent_by) => InboxOrigin::User {
                    sent_by: sent_by.to_string(),
                },
                None => InboxOrigin::System,
            },
        };
        queue(&self.inbox, item)
            .await
            .map_err(MessageError::PersistToolResponse)?;

        tracing::info!(task_id = %task_id, tool_use_id = %tool_use_id, "Tool response sent");

        Ok(())
    }
}

/// Settle the interactive call that an ordinary chat message answered.
///
/// A plain message carries no tool response, so nothing on its way in settles
/// the call like `send_tool_response_flow` does, leaving it `in_progress`
/// forever; this writes the same outcome — text as a freeform answer, or a
/// rejection for a plan — keyed on the answering message, so a redelivered
/// acknowledgement is idempotent.
pub async fn settle_answered_prompt(
    messages: &crate::message::operations::Messages,
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

    fn upload(file_id: &str, filename: &str) -> (String, FileInfo) {
        (
            file_id.to_string(),
            FileInfo {
                file_id: file_id.to_string(),
                filename: filename.to_string(),
                mime_type: "text/markdown".to_string(),
                size: 42,
            },
        )
    }

    #[test]
    fn an_attachment_is_described_by_what_the_store_holds_for_it() {
        let known = HashMap::from([upload("file-1", "brief.md")]);

        let described = attachments_or_refusal(vec!["file-1".to_string()], &known)
            .expect("the store holds this one");

        assert_eq!(described.len(), 1);
        assert_eq!(described[0].0, "file-1");
        assert_eq!(described[0].1.filename, "brief.md");
        assert_eq!(described[0].1.size, 42);
    }

    /// `--context-file` passed a repo path as an attachment id, and the send
    /// took it: the child task reported `File metadata not found` and got
    /// nothing, long after the create that accepted the flag reported success.
    #[test]
    fn an_attachment_with_no_upload_behind_it_is_refused_by_name() {
        let known = HashMap::from([upload("file-1", "brief.md")]);

        let error = attachments_or_refusal(
            vec![
                "file-1".to_string(),
                "docs/implementation-context.md".to_string(),
            ],
            &known,
        )
        .expect_err("a path is not an upload id");

        match error {
            MessageError::BadRequest(reason) => {
                assert!(
                    reason.contains("docs/implementation-context.md"),
                    "{reason}"
                );
                assert!(!reason.contains("file-1"), "{reason}");
            }
            other => panic!("expected a refusal the caller can act on, got {other:?}"),
        }
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

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::*;
    use crate::task::test_support::seed_bare_task;
    use sqlx::PgPool;

    /// The loss this closes: a pod that wakes its agent through the agent
    /// handle alone leaves no row, so a replacement never sees the turn it
    /// opened.
    ///
    /// Acknowledging the queued row stamps the session and turn
    /// `find_interrupted_turn` selects on.
    #[sqlx::test]
    async fn a_queued_wake_becomes_a_recoverable_interrupted_turn(pool: PgPool) {
        seed_bare_task(&pool, "t1", "open").await;
        let graph = crate::compose::task_graph(pool.clone());
        let messages = graph.messages();
        let tasks = graph.tasks();

        let uuid =
            enqueue_sandbox_message(&messages, &tasks, "t1", "<sandbox-restart-note>hi", None)
                .await
                .expect("enqueue")
                .expect("a fresh wake is queued");
        let item = messages
            .inbox()
            .find(uuid.parse().expect("an item id"))
            .await
            .expect("query")
            .expect("the wake's item");
        assert_eq!(item.origin, crate::inbox::InboxOrigin::Sandbox);
        assert!(
            messages.find_by_uuid(&uuid).await.expect("query").is_none(),
            "nobody said the wake, so no Message shows it"
        );

        // Resending the same text is the retry of a call the pod could not
        // confirm; it must collapse onto the first row, not open a second turn.
        assert!(
            enqueue_sandbox_message(&messages, &tasks, "t1", "<sandbox-restart-note>hi", None)
                .await
                .expect("enqueue")
                .is_none()
        );

        messages
            .acknowledge_messages(
                "t1",
                &[uuid],
                &crate::inbox::RunStamp {
                    run_id: Some("run-7".to_string()),
                    session_id: "sess-1".to_string(),
                    turn_number: 7,
                },
            )
            .await
            .expect("acknowledge");
        // The acknowledging run allocated the turn before the ack stamped it.
        sqlx::query(
            "INSERT INTO task_turns (task_id, turn_number, status) VALUES ('t1', 7, 'active')",
        )
        .execute(&pool)
        .await
        .expect("allocate the turn");

        let turn = messages
            .find_interrupted_turn("t1")
            .await
            .expect("query")
            .expect("the wake's turn is recoverable");
        assert_eq!(turn.turn_number, 7);
    }

    /// A closed Task is terminal, so the wake would open a turn on a
    /// conversation nobody will read.
    #[sqlx::test]
    async fn a_closed_task_refuses_the_wake(pool: PgPool) {
        seed_bare_task(&pool, "t1", "closed").await;
        let graph = crate::compose::task_graph(pool.clone());
        let queued = enqueue_sandbox_message(&graph.messages(), &graph.tasks(), "t1", "hi", None)
            .await
            .expect("enqueue");
        assert!(queued.is_none());
    }

    /// A `/loop` tick repeats its prompt verbatim, so keying the dedup on the
    /// text would queue the first tick and silently swallow every one after it.
    #[sqlx::test]
    async fn identical_texts_under_distinct_keys_each_open_a_turn(pool: PgPool) {
        seed_bare_task(&pool, "t1", "open").await;
        let graph = crate::compose::task_graph(pool.clone());
        let (messages, tasks) = (graph.messages(), graph.tasks());

        let first = enqueue_sandbox_message(&messages, &tasks, "t1", "tick", Some("job-1"))
            .await
            .expect("enqueue");
        let second = enqueue_sandbox_message(&messages, &tasks, "t1", "tick", Some("job-2"))
            .await
            .expect("enqueue");
        assert!(first.is_some() && second.is_some() && first != second);

        // The same key twice is one job's retry, and still collapses.
        assert!(
            enqueue_sandbox_message(&messages, &tasks, "t1", "tick", Some("job-1"))
                .await
                .expect("enqueue")
                .is_none()
        );
    }
}
