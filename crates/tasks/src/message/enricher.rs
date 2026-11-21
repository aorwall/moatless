//! Message content enricher — construct agent-facing prompt text from user
//! input.

use std::collections::HashMap;
use std::fmt::Write;

use serde_json::Value;

use super::contexts::task_message_context_to_input;
use super::models::{TaskMessage, TaskMessageContext};
use crate::task::schemas::MessageContextInput;
use moatless_core::config::Config;
use moatless_core::config::DEFAULT_CODEX_MODEL;
use moatless_core::file_storage::Files;
use moatless_types::adapter::SCHEDULED_ORIGIN;
use moatless_types::agent::AgentProfile;

use crate::agent::catalog::{self, UNRESOLVED_MODEL_CHOICE};

/// Content block for agent request.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Value>,
}

/// One pending harness input assembled by the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDelivery {
    pub message_uuids: Vec<String>,
    pub prompt_text: String,
    pub file_ids: Vec<String>,
    pub model: Option<String>,
    pub agent_profile: Option<String>,
    pub agent_mode: Option<String>,
    /// Reasoning effort for the turn, in the agent's own vocabulary. `None`
    /// leaves the agent on its own default.
    pub effort: Option<String>,
    /// Skills to frontload into this turn, in the order to apply them. Names
    /// only — the sandbox resolves each one's `SKILL.md` on its own disk.
    pub skills: Vec<String>,
    pub resume_session_id: Option<String>,
    pub resume: bool,
    pub start_fresh: bool,
    /// A person's answer, where this delivery is one. See [`PromptResponse`].
    pub prompt_response: Option<PromptResponse>,
    /// The agent's memory is current once this is accepted: it starts a
    /// session, whose system prompt lists preloaded memory afresh, or it had
    /// room for the memory note.
    pub memory_current: bool,
}

/// A person's answer to an interactive tool call, as the sandbox resolves it.
///
/// Read off the row's own `tool_response` column, never parsed back out of the
/// prompt text — that text is what an agent who has forgotten asking is told,
/// not an answer per question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptResponse {
    pub tool_use_id: String,
    pub tool_name: String,
    /// One answer per question, keyed on the question's full text. Empty for a
    /// plan verdict, which answers with [`approved`](Self::approved) instead.
    pub answers: std::collections::BTreeMap<String, String>,
    pub approved: bool,
    pub feedback: Option<String>,
}

impl PromptResponse {
    /// The answer a row carries, or `None` where it carries none.
    fn read(row: &TaskMessage) -> Option<Self> {
        let stored = row.tool_response.as_ref()?;
        let field = |name: &str| stored.get(name).and_then(Value::as_str).map(str::to_string);
        Some(Self {
            tool_use_id: field("toolUseId")?,
            tool_name: field("toolName")?,
            answers: stored
                .get("answers")
                .and_then(Value::as_object)
                .map_or_default(|answers| {
                    answers
                        .iter()
                        .filter_map(|(question, given)| {
                            Some((question.clone(), given.as_str()?.to_string()))
                        })
                        .collect()
                }),
            approved: stored.get("approved").and_then(Value::as_bool) == Some(true),
            feedback: field("feedback").filter(|feedback| !feedback.trim().is_empty()),
        })
    }
}

/// Builds pending Message rows into the exact unit the sandbox submits.
///
/// Hides the transient prompt-assembly rules from the gRPC handler and sandbox:
/// callers get text, upload ids, model metadata, resume intent, and uuids to
/// acknowledge after harness acceptance.
pub struct AgentPromptBuilder<'a> {
    files: Option<&'a Files>,
    config: Option<&'a Config>,
    task_agent_profile: AgentProfile,
    task_model: Option<String>,
    task_resume_session_id: Option<String>,
    task_skills: Vec<String>,
    /// Logins already resolved for this fetch, keyed by [`sender_key`], for an
    /// external row's `<from>`.
    sender_logins: &'a HashMap<String, String>,
    memory_note: Option<String>,
}

impl<'a> AgentPromptBuilder<'a> {
    pub fn new(
        files: Option<&'a Files>,
        config: Option<&'a Config>,
        task_agent_profile: AgentProfile,
        task_model: Option<String>,
        task_resume_session_id: Option<String>,
        task_skills: Vec<String>,
        sender_logins: &'a HashMap<String, String>,
    ) -> Self {
        Self {
            files,
            config,
            task_agent_profile,
            task_model,
            task_resume_session_id,
            task_skills,
            sender_logins,
            memory_note: None,
        }
    }

    /// Text appended to a delivery that resumes its session, answers no parked
    /// prompt and starts with no slash command, which would take the note as
    /// its arguments. A fresh session reads the index from its system prompt.
    #[must_use]
    pub fn with_memory_note(mut self, note: Option<String>) -> Self {
        self.memory_note = note;
        self
    }

    /// Build the pending deliveries for this fetch.
    ///
    /// The dispatcher submits one harness input per fetch, so this returns at
    /// most one delivery: all rows before the first `start_fresh`, or that row
    /// alone.
    pub async fn build_pending_deliveries(
        &self,
        pending: &[TaskMessage],
        contexts_by_message: &HashMap<i32, Vec<TaskMessageContext>>,
    ) -> Vec<AgentDelivery> {
        let Some((rows, _deferred)) = delivery_slice(pending) else {
            return Vec::new();
        };

        let mut parts = Vec::new();
        let mut message_uuids = Vec::new();
        let mut file_ids = Vec::new();
        let mut model = None;
        let mut agent_mode = None;
        let mut effort = None;
        let start_fresh = rows.iter().any(|row| row.start_fresh);
        let resume = !start_fresh
            && self.task_resume_session_id.as_ref().is_some()
            && rows.last().is_some_and(|row| row.resume);

        // A fresh session has no transcript, so the task's own skills have to be
        // activated again; a resumed one still carries them. Per-message skills
        // ride on top either way, deduped and in the order they were asked for.
        let mut skills = if resume {
            Vec::new()
        } else {
            self.task_skills.clone()
        };

        for row in rows {
            let mut text = self
                .build_message_prompt(
                    row,
                    contexts_by_message
                        .get(&row.id)
                        .map_or(&[][..], Vec::as_slice),
                )
                .await;
            let header = build_message_header(row, self.sender_logins);
            if !header.is_empty() {
                text = format!("{header}{text}");
            }
            if !text.is_empty() {
                parts.push(text);
            }
            message_uuids.push(row.uuid.clone());
            file_ids.extend(row.file_ids());
            model = effective_model(
                row.model.clone(),
                self.task_model.clone(),
                self.task_agent_profile,
                self.config.map_or(DEFAULT_CODEX_MODEL, |config| {
                    config.sandbox.default_codex_model.as_str()
                }),
            )
            .or(model);
            if let Some(mode) = &row.agent_mode {
                agent_mode = Some(mode.as_str().to_string());
            }
            effort = row.effort.clone().or(effort);
            for name in &row.skills {
                if !skills.iter().any(|seen| seen == name) {
                    skills.push(name.clone());
                }
            }
        }

        // Skills alone are enough to open a turn: they frontload into a prompt on
        // the pod, so a start request that carries only skills still runs. Drop the
        // delivery only when there is neither text nor a skill to activate.
        if parts.is_empty() && skills.is_empty() {
            tracing::warn!("Pending delivery has no prompt text");
            return Vec::new();
        }

        // At most one answer per delivery: an answer is sent on its own row, and
        // the first is the one the run is parked on. A second in the same batch
        // would be an answer to a call the first has already released.
        let prompt_response = rows.iter().find_map(PromptResponse::read);
        let note_fits =
            prompt_response.is_none() && !parts.first().is_some_and(|text| text.starts_with('/'));
        if let Some(note) = self.memory_note.as_ref().filter(|_| resume && note_fits) {
            parts.push(note.clone());
        }

        vec![AgentDelivery {
            message_uuids,
            prompt_response,
            prompt_text: parts.join("\n"),
            file_ids,
            model,
            agent_profile: Some(self.task_agent_profile.to_string()),
            agent_mode,
            effort,
            skills,
            resume_session_id: if resume {
                self.task_resume_session_id.clone()
            } else {
                None
            },
            resume,
            start_fresh,
            memory_current: !resume || note_fits,
        }]
    }

    async fn build_message_prompt(
        &self,
        message: &TaskMessage,
        contexts: &[TaskMessageContext],
    ) -> String {
        if let (Some(files), Some(config)) = (self.files, self.config) {
            return build_pending_content_blocks(files, config, message, contexts)
                .await
                .into_iter()
                .filter_map(|block| match block.block_type.as_str() {
                    "text" => block.text,
                    _ => None,
                })
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
        }

        tracing::warn!(
            message_uuid = %message.uuid,
            "Delivering pending message without context enrichment because file store or config is unavailable"
        );
        message_text_with_inline_context(message, contexts)
    }
}

fn delivery_slice(pending: &[TaskMessage]) -> Option<(&[TaskMessage], usize)> {
    if pending.is_empty() {
        return None;
    }
    let idx = pending.iter().position(|row| row.start_fresh);
    let keep = match idx {
        Some(0) => 1,
        Some(idx) => idx,
        None => pending.len(),
    };
    Some((&pending[..keep], pending.len() - keep))
}

/// Resolve the model to deliver for a pending message.
///
/// An explicit per-message model wins. Otherwise `claude-code-fable` gets its
/// pinned model and Codex `default_codex_model`, since the sandbox refuses a
/// Codex turn without one; Claude passes [`UNRESOLVED_MODEL_CHOICE`] through.
pub fn effective_model(
    row_model: Option<String>,
    task_model: Option<String>,
    agent_profile: AgentProfile,
    default_codex_model: &str,
) -> Option<String> {
    let chosen = row_model.or(task_model);
    match agent_profile {
        AgentProfile::ClaudeCode | AgentProfile::ClaudeCodeTui | AgentProfile::ClaudeCodeCli => {
            chosen
        }
        AgentProfile::ClaudeCodeFable => {
            chosen.or_else(|| Some(catalog::CLAUDE_CODE_FABLE.default_model.to_string()))
        }
        AgentProfile::Antigravity => {
            chosen.or_else(|| Some(catalog::ANTIGRAVITY.default_model.to_string()))
        }
        AgentProfile::MiniSwe => {
            chosen.or_else(|| Some(catalog::MINI_SWE.default_model.to_string()))
        }
        AgentProfile::Pi => chosen.or_else(|| Some(catalog::PI.default_model.to_string())),
        AgentProfile::Codex => Some(
            chosen
                .filter(|model| model != UNRESOLVED_MODEL_CHOICE)
                .unwrap_or_else(|| default_codex_model.to_string()),
        ),
    }
}

/// The key a resolved sender is cached under. Two Adapters could hand out the
/// same id string, so the Adapter is part of the identity.
pub fn sender_key(adapter: &str, sender_external_id: &str) -> String {
    format!("{adapter}:{sender_external_id}")
}

/// Who wrote an external row, in terms the agent can act on.
///
/// Resolved from the writer's own external id, never `sent_by` — an unmapped
/// writer is attributed to the trigger owner, so `sent_by` would misname them
/// in a multi-participant thread. Falls back to the raw handle, claiming no
/// login, when there's no mapping.
fn sender_label(row: &TaskMessage, sender_logins: &HashMap<String, String>) -> Option<String> {
    let adapter = row.external_adapter.as_deref().unwrap_or("external");
    let handle = row.external_author.as_deref();
    let login = row
        .external_sender_id
        .as_deref()
        .and_then(|id| sender_logins.get(&sender_key(adapter, id)))
        .map(String::as_str);

    match (login, handle) {
        // The handle repeats the login: one name is the whole answer.
        (Some(login), Some(handle)) if login == handle => Some(login.to_string()),
        (Some(login), Some(handle)) => Some(format!("{login} ({adapter}: {handle})")),
        (Some(login), None) => Some(login.to_string()),
        // No mapping. Name the writer in the Adapter's own terms and claim
        // nothing about who they are in Moatless.
        (None, Some(handle)) => Some(format!("{adapter}: {handle}")),
        // Not even a handle — the raw id still tells two writers apart, which
        // is the least this has to do.
        (None, None) => row
            .external_sender_id
            .as_deref()
            .map(|id| format!("{adapter}: {id}")),
    }
}

pub(crate) fn build_message_header(
    row: &TaskMessage,
    login_cache: &HashMap<String, String>,
) -> String {
    // A schedule firing stores an `external_event_id` for dedup only; see
    // [`SCHEDULED_ORIGIN`] for why such a row carries no header.
    let external = row.external_event_id.is_some()
        && row.external_adapter.as_deref() != Some(SCHEDULED_ORIGIN);

    if external {
        let mut header = format!("<message_id>{}</message_id>\n", row.uuid);
        if let Some(source) = row.external_adapter.as_deref() {
            let _ = writeln!(header, "<source>{source}</source>");
        }
        if let Some(thread) = row.external_subject_key.as_deref() {
            let _ = writeln!(header, "<thread>{thread}</thread>");
        }
        if let Some(from) = sender_label(row, login_cache) {
            let _ = writeln!(header, "<from>{from}</from>");
        }
        header.push('\n');
        return header;
    }

    // An internal row goes out bare, deliberately: a leading `/compact` or
    // other slash command only runs when it is the first thing in the prompt.
    String::new()
}

/// Build content blocks from user input + contexts + attachments.
///
/// The primary text block has `show_in_ui = true`; context and file blocks have
/// `show_in_ui = false` (agent-request only).
pub async fn build_content_blocks(
    files: &Files,
    config: &Config,
    text: &str,
    contexts: &[MessageContextInput],
    file_ids: &[String],
) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();

    // Primary text block (visible in UI)
    blocks.push(ContentBlock {
        block_type: "text".to_string(),
        text: Some(text.to_string()),
        source: None,
    });

    // Context blocks (hidden from UI)
    for ctx in contexts {
        match ctx {
            MessageContextInput::PreviewUrl { reference_id, data } => {
                let url = data.url.as_deref().unwrap_or(reference_id);
                blocks.push(ContentBlock {
                    block_type: "text".to_string(),
                    text: Some(format!("User is currently viewing: {url}")),
                    source: None,
                });
            }
            MessageContextInput::Element { data, .. } => {
                let element_text = serde_json::to_string_pretty(&data).unwrap_or_default();
                blocks.push(ContentBlock {
                    block_type: "text".to_string(),
                    text: Some(format!("Selected UI element context:\n{element_text}")),
                    source: None,
                });
            }
            // File contexts are persisted separately and rendered into the
            // agent prompt via `file_ids` below; nothing to do here.
            MessageContextInput::File { .. } => {}
        }
    }

    // File attachment blocks (hidden from UI)
    for file_id in file_ids {
        if let Some(block) =
            build_file_attachment_block(files, file_id, &config.sandbox.file_mount_path).await
        {
            blocks.push(block);
        }
    }

    blocks
}

/// Attribution for a message one Task sent to another.
///
/// Applied on the way out and never stored, so the reply instruction is
/// regenerated from the current `source_task_id` rather than frozen into a row
/// at send time — the agent needs to know where it came from; the person
/// reading the transcript already sees the sending Task.
fn attributed_text(text: &str, source_task_id: Option<&str>) -> String {
    match source_task_id {
        Some(id) => format!(
            "Message from task `{id}`. An AI agent wrote this and sent it here. \
             No person typed it and no person is waiting on an answer.\n\n\
             {text}\n\n\
             Reply only to answer a question it asked or to unblock a decision it named:\n\n  \
             `moat send --task {id} --text \"<your reply>\"`\n\n\
             A message that only reports or signs off needs no reply. Keep a reply to the \
             answer and anything the other agent cannot look up itself; send no \
             acknowledgement, no thanks, no progress update, no confirmation of receipt."
        ),
        None => text.to_string(),
    }
}

/// Rebuild the agent-facing content of a queued Message, at delivery time.
///
/// A Message stores only what the person wrote; everything else (preview URL,
/// selected element, attribution, attached files) is rebuilt from its own
/// persisted row rather than cached, so nothing can disagree. Read-side twin of
/// [`build_content_blocks`].
pub async fn build_pending_content_blocks(
    files: &Files,
    config: &Config,
    message: &TaskMessage,
    contexts: &[TaskMessageContext],
) -> Vec<ContentBlock> {
    let text = message_text_with_inline_context(message, contexts);

    // A row whose context type this build does not know is dropped, exactly as
    // the transform layer drops it — a forward-incompatible context must not
    // stop the message it decorates from reaching the agent.
    let inputs: Vec<MessageContextInput> = contexts
        .iter()
        .filter_map(task_message_context_to_input)
        .collect();

    build_content_blocks(files, config, &text, &inputs, &message.file_ids()).await
}

fn message_text_with_inline_context(
    message: &TaskMessage,
    contexts: &[TaskMessageContext],
) -> String {
    contexts
        .iter()
        .filter(|context| matches!(context.context_type.as_str(), "mergeBack" | "sideChat"))
        .filter_map(|context| context.data.get("text").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .chain(std::iter::once(attributed_text(
            message.message_text.as_deref().unwrap_or_default(),
            message.source_task_id.as_deref(),
        )))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Build a content block from a file attachment reference.
///
/// `file_mount_path` is where the sandbox writes uploads before invoking the
/// agent; the prompt advertises that on-disk path so the agent reads the file
/// directly, and the sandbox gets the same value as `UPLOAD_DIR` so the two
/// cannot disagree.
async fn build_file_attachment_block(
    files: &Files,
    file_id: &str,
    file_mount_path: &str,
) -> Option<ContentBlock> {
    let row = match files.metadata(file_id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            tracing::warn!(file_id, "File metadata not found for attachment block");
            return None;
        }
        Err(e) => {
            tracing::warn!(file_id, error = %e, "Failed to load file metadata");
            return None;
        }
    };

    let mount = file_mount_path.trim_end_matches('/');
    Some(ContentBlock {
        block_type: "text".to_string(),
        text: Some(format!(
            "Attached file: {} ({}, {} bytes)\nSaved to: {}/{}/{}",
            row.filename, row.mime_type, row.size, mount, file_id, row.filename,
        )),
        source: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_core::file_storage::FileInfo;
    use moatless_core::file_storage::test_support::files_from;
    use serde_json::json;

    fn pending_message(id: i32, uuid: &str, text: &str) -> TaskMessage {
        let mut message = TaskMessage::new_user_message(
            "task-1".to_string(),
            uuid.to_string(),
            text.to_string(),
            "2026-08-01T00:00:00Z".to_string(),
        );
        message.id = id;
        message
    }

    fn prompt_builder(
        sender_logins: &HashMap<String, String>,
        resume_session_id: Option<String>,
    ) -> AgentPromptBuilder<'_> {
        prompt_builder_with_task_skills(sender_logins, resume_session_id, Vec::new())
    }

    fn prompt_builder_with_task_skills(
        sender_logins: &HashMap<String, String>,
        resume_session_id: Option<String>,
        task_skills: Vec<String>,
    ) -> AgentPromptBuilder<'_> {
        AgentPromptBuilder::new(
            None,
            None,
            AgentProfile::ClaudeCode,
            None,
            resume_session_id,
            task_skills,
            sender_logins,
        )
    }

    /// The text is what an agent that has forgotten asking is told; the answer
    /// per question travels structurally beside it.
    #[test]
    fn an_answer_is_read_off_its_own_column_not_out_of_the_text() {
        let mut row = pending_message(1, "msg-1", "You asked, and the answer is: Postgres");
        row.tool_response = Some(json!({
            "toolUseId": "tu-1",
            "toolName": "AskUserQuestion",
            "answers": { "Which store?": "Postgres" }
        }));
        let answer = PromptResponse::read(&row).expect("the row carries an answer");
        assert_eq!(answer.tool_use_id, "tu-1");
        assert_eq!(answer.tool_name, "AskUserQuestion");
        assert_eq!(answer.answers["Which store?"], "Postgres");
        assert!(!answer.approved);
    }

    #[test]
    fn a_plan_verdict_carries_its_feedback_and_no_answers() {
        let mut row = pending_message(1, "msg-1", "The plan is rejected.");
        row.tool_response = Some(json!({
            "toolUseId": "tu-1",
            "toolName": "ExitPlanMode",
            "approved": false,
            "feedback": "use the second approach"
        }));
        let answer = PromptResponse::read(&row).expect("the row carries an answer");
        assert!(answer.answers.is_empty());
        assert!(!answer.approved);
        assert_eq!(answer.feedback.as_deref(), Some("use the second approach"));
    }

    /// An ordinary message carries no answer, and must not be read as one.
    #[test]
    fn an_ordinary_message_carries_no_answer() {
        assert_eq!(
            PromptResponse::read(&pending_message(1, "msg-1", "have another look")),
            None
        );
    }

    /// A delivery batching an answer with the messages around it still names
    /// the answer, so the parked call is released by the row that answers it.
    #[tokio::test]
    async fn a_batched_delivery_still_names_the_answer_in_it() {
        let mut answered = pending_message(2, "msg-2", "The plan is approved. Carry it out.");
        answered.tool_response = Some(json!({
            "toolUseId": "tu-1",
            "toolName": "ExitPlanMode",
            "approved": true
        }));
        let rows = vec![pending_message(1, "msg-1", "and one more thing"), answered];

        let logins = HashMap::new();
        let contexts = HashMap::new();
        let deliveries = prompt_builder(&logins, None)
            .build_pending_deliveries(&rows, &contexts)
            .await;
        let answer = deliveries[0]
            .prompt_response
            .as_ref()
            .expect("the batch names its answer");
        assert_eq!(answer.tool_use_id, "tu-1");
        assert!(answer.approved);
    }

    /// A batch runs as one Turn, so it runs at one effort: the newest row that
    /// named one, the same rule the model follows.
    #[tokio::test]
    async fn a_batched_delivery_runs_at_the_newest_effort_named_in_it() {
        let mut first = pending_message(1, "msg-1", "start on this");
        first.effort = Some("low".to_string());
        let mut second = pending_message(2, "msg-2", "actually, think harder");
        second.effort = Some("max".to_string());
        let rows = vec![first, second, pending_message(3, "msg-3", "and one more")];

        let logins = HashMap::new();
        let contexts = HashMap::new();
        let deliveries = prompt_builder(&logins, None)
            .build_pending_deliveries(&rows, &contexts)
            .await;
        assert_eq!(deliveries[0].effort.as_deref(), Some("max"));
    }

    #[tokio::test]
    async fn a_delivery_nobody_picked_an_effort_for_carries_none() {
        let rows = vec![pending_message(1, "msg-1", "have another look")];
        let logins = HashMap::new();
        let contexts = HashMap::new();
        let deliveries = prompt_builder(&logins, None)
            .build_pending_deliveries(&rows, &contexts)
            .await;
        assert_eq!(deliveries[0].effort, None);
    }

    /// A fresh session carries no transcript, so the task's skills have to be
    /// activated again.
    #[tokio::test]
    async fn task_skills_ride_a_delivery_that_starts_a_fresh_session() {
        let rows = vec![pending_message(1, "msg-1", "start")];
        let logins = HashMap::new();
        let contexts = HashMap::new();
        let deliveries =
            prompt_builder_with_task_skills(&logins, None, vec!["e2e-testing".to_string()])
                .build_pending_deliveries(&rows, &contexts)
                .await;
        assert_eq!(deliveries[0].skills, vec!["e2e-testing".to_string()]);
    }

    /// A resumed session still has them, so re-sending the body would be waste.
    #[tokio::test]
    async fn task_skills_are_left_off_a_resumed_delivery() {
        let rows = vec![pending_message(1, "msg-1", "carry on")];
        let logins = HashMap::new();
        let contexts = HashMap::new();
        let deliveries = prompt_builder_with_task_skills(
            &logins,
            Some("session-1".to_string()),
            vec!["e2e-testing".to_string()],
        )
        .build_pending_deliveries(&rows, &contexts)
        .await;
        assert!(deliveries[0].resume);
        assert!(deliveries[0].skills.is_empty());
    }

    #[tokio::test]
    async fn the_memory_note_follows_a_resumed_delivery_only() {
        let rows = vec![pending_message(1, "msg-1", "carry on")];
        let logins = HashMap::new();
        let contexts = HashMap::new();
        let note = Some("## Memory updated since your last turn".to_string());
        let resumed = prompt_builder(&logins, Some("session-1".to_string()))
            .with_memory_note(note.clone())
            .build_pending_deliveries(&rows, &contexts)
            .await;
        assert!(resumed[0].prompt_text.starts_with("carry on"));
        assert!(
            resumed[0]
                .prompt_text
                .ends_with("## Memory updated since your last turn")
        );
        assert!(resumed[0].memory_current);

        let fresh = prompt_builder(&logins, None)
            .with_memory_note(note)
            .build_pending_deliveries(&rows, &contexts)
            .await;
        assert!(!fresh[0].prompt_text.contains("Memory updated"));
        assert!(fresh[0].memory_current);
    }

    /// `/compact <text>` takes the text as compaction instructions, so the
    /// note waits, and `memory_current` keeps its changes for the next one.
    #[tokio::test]
    async fn a_slash_command_takes_no_memory_note() {
        let rows = vec![pending_message(1, "msg-1", "/compact")];
        let logins = HashMap::new();
        let contexts = HashMap::new();
        let deliveries = prompt_builder(&logins, Some("session-1".to_string()))
            .with_memory_note(Some("## Memory updated since your last turn".to_string()))
            .build_pending_deliveries(&rows, &contexts)
            .await;
        assert_eq!(deliveries[0].prompt_text, "/compact");
        assert!(!deliveries[0].memory_current);
    }

    /// `start_fresh` cuts the session, which is exactly when they are needed.
    #[tokio::test]
    async fn task_skills_come_back_on_a_start_fresh_delivery() {
        let mut fresh = pending_message(1, "msg-1", "from the top");
        fresh.start_fresh = true;
        let logins = HashMap::new();
        let contexts = HashMap::new();
        let deliveries = prompt_builder_with_task_skills(
            &logins,
            Some("session-1".to_string()),
            vec!["e2e-testing".to_string()],
        )
        .build_pending_deliveries(&[fresh], &contexts)
        .await;
        assert_eq!(deliveries[0].skills, vec!["e2e-testing".to_string()]);
    }

    #[tokio::test]
    async fn message_skills_ride_even_a_resumed_delivery() {
        let mut row = pending_message(1, "msg-1", "use this");
        row.skills = vec!["rust-dev-guidelines".to_string()];
        let logins = HashMap::new();
        let contexts = HashMap::new();
        let deliveries = prompt_builder(&logins, Some("session-1".to_string()))
            .build_pending_deliveries(&[row], &contexts)
            .await;
        assert!(deliveries[0].resume);
        assert_eq!(
            deliveries[0].skills,
            vec!["rust-dev-guidelines".to_string()]
        );
    }

    /// Task skills lead, message skills follow, and a name asked for twice is
    /// frontloaded once.
    #[tokio::test]
    async fn a_batched_delivery_unions_its_skills_in_order() {
        let mut first = pending_message(1, "msg-1", "start on this");
        first.skills = vec!["e2e-testing".to_string(), "moat-cli".to_string()];
        let mut second = pending_message(2, "msg-2", "and this");
        second.skills = vec!["moat-cli".to_string(), "shadow-dev".to_string()];

        let logins = HashMap::new();
        let contexts = HashMap::new();
        let deliveries =
            prompt_builder_with_task_skills(&logins, None, vec!["e2e-testing".to_string()])
                .build_pending_deliveries(&[first, second], &contexts)
                .await;
        assert_eq!(
            deliveries[0].skills,
            vec![
                "e2e-testing".to_string(),
                "moat-cli".to_string(),
                "shadow-dev".to_string()
            ]
        );
    }

    /// A start request can carry skills and no message text. The skills alone
    /// open a turn — they frontload into a prompt on the pod — so the delivery
    /// must survive with an empty prompt rather than being dropped.
    #[tokio::test]
    async fn a_skills_only_delivery_survives_with_no_prompt_text() {
        let rows = vec![pending_message(1, "msg-1", "")];
        let logins = HashMap::new();
        let contexts = HashMap::new();
        let deliveries =
            prompt_builder_with_task_skills(&logins, None, vec!["e2e-testing".to_string()])
                .build_pending_deliveries(&rows, &contexts)
                .await;
        assert_eq!(deliveries.len(), 1);
        assert!(deliveries[0].prompt_text.is_empty());
        assert_eq!(deliveries[0].skills, vec!["e2e-testing".to_string()]);
    }

    /// With neither text nor a skill there is nothing to open a turn with, so
    /// the delivery is still dropped.
    #[tokio::test]
    async fn a_delivery_with_no_text_and_no_skills_is_dropped() {
        let rows = vec![pending_message(1, "msg-1", "")];
        let logins = HashMap::new();
        let contexts = HashMap::new();
        let deliveries = prompt_builder(&logins, None)
            .build_pending_deliveries(&rows, &contexts)
            .await;
        assert!(deliveries.is_empty());
    }

    #[test]
    fn effective_model_forces_fable_when_unset() {
        assert_eq!(
            effective_model(None, None, AgentProfile::ClaudeCodeFable, "gpt-test"),
            Some("fable".to_string())
        );
    }

    #[test]
    fn effective_model_uses_task_model_when_message_omits_one() {
        assert_eq!(
            effective_model(
                None,
                Some("claude-sonnet-4-6".to_string()),
                AgentProfile::ClaudeCode,
                "gpt-test",
            ),
            Some("claude-sonnet-4-6".to_string())
        );
    }

    #[test]
    fn effective_model_preserves_explicit_model() {
        assert_eq!(
            effective_model(
                Some("claude-opus-4-6".to_string()),
                Some("fable".to_string()),
                AgentProfile::ClaudeCodeFable,
                "gpt-test",
            ),
            Some("claude-opus-4-6".to_string())
        );
    }

    #[test]
    fn effective_model_leaves_claude_unset() {
        let claude = AgentProfile::ClaudeCode;
        assert_eq!(effective_model(None, None, claude, "gpt-test"), None);
    }

    #[test]
    fn effective_model_resolves_the_codex_default() {
        let codex = AgentProfile::Codex;
        let resolved = Some("gpt-test".to_string());
        assert_eq!(effective_model(None, None, codex, "gpt-test"), resolved);
        let unresolved = Some(UNRESOLVED_MODEL_CHOICE.to_string());
        assert_eq!(
            effective_model(unresolved, None, codex, "gpt-test"),
            resolved
        );
        let pinned = Some("gpt-5.5".to_string());
        assert_eq!(
            effective_model(pinned.clone(), None, codex, "gpt-test"),
            pinned
        );
    }

    #[tokio::test]
    async fn builder_batches_rows_before_first_start_fresh() {
        let cache = HashMap::new();
        let mut first = pending_message(1, "msg-1", "hello");
        first.attachments = Some(json!({ "file_ids": ["file-1"] }));
        let second = pending_message(2, "msg-2", "world");
        let mut fresh = pending_message(3, "msg-3", "fresh");
        fresh.start_fresh = true;

        let deliveries = prompt_builder(&cache, Some("session-1".to_string()))
            .build_pending_deliveries(&[first, second, fresh], &HashMap::new())
            .await;

        assert_eq!(deliveries.len(), 1);
        let delivery = &deliveries[0];
        assert_eq!(delivery.message_uuids, vec!["msg-1", "msg-2"]);
        assert_eq!(delivery.prompt_text, "hello\nworld");
        assert_eq!(delivery.file_ids, vec!["file-1"]);
        assert_eq!(delivery.resume_session_id.as_deref(), Some("session-1"));
        assert!(delivery.resume);
        assert!(!delivery.start_fresh);
    }

    #[tokio::test]
    async fn builder_dispatches_leading_start_fresh_alone_without_resume() {
        let cache = HashMap::new();
        let mut fresh = pending_message(1, "msg-1", "fresh");
        fresh.start_fresh = true;
        let second = pending_message(2, "msg-2", "later");

        let deliveries = prompt_builder(&cache, Some("session-1".to_string()))
            .build_pending_deliveries(&[fresh, second], &HashMap::new())
            .await;

        assert_eq!(deliveries.len(), 1);
        let delivery = &deliveries[0];
        assert_eq!(delivery.message_uuids, vec!["msg-1"]);
        assert_eq!(delivery.prompt_text, "fresh");
        assert_eq!(delivery.resume_session_id, None);
        assert!(!delivery.resume);
        assert!(delivery.start_fresh);
    }

    #[tokio::test]
    async fn external_row_header_is_in_prompt_text() {
        let mut cache = HashMap::new();
        cache.insert("u1".to_string(), "alice".to_string());
        let mut message = pending_message(1, "msg_abc", "hello");
        message.external_event_id = Some("slack:ts:1700".to_string());
        message.external_adapter = Some("slack".to_string());
        message.external_subject_key = Some("T06:C0A:1781593153.718929".to_string());
        message.sent_by = Some("u1".to_string());

        let deliveries = prompt_builder(&cache, None)
            .build_pending_deliveries(&[message], &HashMap::new())
            .await;

        assert_eq!(
            deliveries[0].prompt_text,
            "<message_id>msg_abc</message_id>\n<source>slack</source>\n\
             <thread>T06:C0A:1781593153.718929</thread>\n\nhello",
            "a row with no sender id names nobody rather than naming sent_by"
        );
    }

    #[tokio::test]
    async fn a_scheduled_row_carries_no_header_and_no_reply_target() {
        let mut message = pending_message(1, "msg_abc", "/docs-improver");
        message.external_event_id = Some("scheduled:sch-1:1780000000".to_string());
        message.external_adapter = Some(SCHEDULED_ORIGIN.to_string());
        message.external_subject_key = Some("scheduled:sch-1".to_string());

        let deliveries = prompt_builder(&HashMap::new(), None)
            .build_pending_deliveries(&[message], &HashMap::new())
            .await;

        assert_eq!(deliveries[0].prompt_text, "/docs-improver");
    }

    /// An external row that names a writer.
    fn slack_row(sender_id: Option<&str>, handle: Option<&str>) -> TaskMessage {
        let mut message = pending_message(1, "msg_abc", "hello");
        message.external_event_id = Some("slack:ts:1700".to_string());
        message.external_adapter = Some("slack".to_string());
        message.external_sender_id = sender_id.map(str::to_string);
        message.external_author = handle.map(str::to_string);
        // The trigger owner an unmapped writer is attributed to. The label must
        // never reach for it.
        message.sent_by = Some("u1".to_string());
        message
    }

    async fn from_line(row: TaskMessage, cache: &HashMap<String, String>) -> String {
        let deliveries = prompt_builder(cache, None)
            .build_pending_deliveries(&[row], &HashMap::new())
            .await;
        deliveries[0]
            .prompt_text
            .lines()
            .find(|line| line.starts_with("<from>"))
            .unwrap_or("")
            .to_string()
    }

    /// The whole point of resolving from the writer's own id: `sent_by` here
    /// resolves to `alice`, the trigger owner, and the label must say
    /// `aorwall`.
    #[tokio::test]
    async fn a_mapped_writer_is_named_by_login_not_by_the_trigger_owner() {
        let mut cache = HashMap::new();
        cache.insert("u1".to_string(), "alice".to_string());
        cache.insert(sender_key("slack", "U0AAQ"), "aorwall".to_string());

        assert_eq!(
            from_line(slack_row(Some("U0AAQ"), Some("albertornelius")), &cache).await,
            "<from>aorwall (slack: albertornelius)</from>"
        );
    }

    #[tokio::test]
    async fn a_handle_that_repeats_the_login_is_not_said_twice() {
        let mut cache = HashMap::new();
        cache.insert(sender_key("slack", "U0AAQ"), "aorwall".to_string());

        assert_eq!(
            from_line(slack_row(Some("U0AAQ"), Some("aorwall")), &cache).await,
            "<from>aorwall</from>"
        );
    }

    /// No mapping, so no login is claimed — the handle is the only true thing.
    #[tokio::test]
    async fn an_unmapped_writer_is_named_by_handle_and_claims_no_login() {
        assert_eq!(
            from_line(
                slack_row(Some("U0MARTIN"), Some("martin.k")),
                &HashMap::new()
            )
            .await,
            "<from>slack: martin.k</from>"
        );
    }

    /// The `users.info` lookup failed. Two writers must still read as two.
    #[tokio::test]
    async fn a_writer_with_no_handle_is_still_told_apart_by_id() {
        assert_eq!(
            from_line(slack_row(Some("U0MARTIN"), None), &HashMap::new()).await,
            "<from>slack: U0MARTIN</from>"
        );
    }

    /// Written before the column existed: nothing to resolve, and the handle
    /// carries it.
    #[tokio::test]
    async fn a_pre_migration_row_falls_back_to_its_handle() {
        assert_eq!(
            from_line(slack_row(None, Some("martin.k")), &HashMap::new()).await,
            "<from>slack: martin.k</from>"
        );
    }

    #[test]
    fn a_task_message_names_its_sender_as_an_agent_and_gates_the_reply() {
        let text = attributed_text("hello", Some("task-abc"));

        assert!(text.contains("An AI agent wrote this"));
        assert!(text.contains("No person typed it"));
        assert!(text.contains("Reply only to answer a question it asked"));
        assert!(text.contains("only reports or signs off needs no reply"));
        assert!(text.contains("`moat send --task task-abc --text \"<your reply>\"`"));
    }

    #[tokio::test]
    async fn an_internal_row_is_delivered_without_a_header() {
        let mut message = pending_message(1, "msg_abc", "/compact");
        message.sent_by = Some("u1".to_string());

        let deliveries = prompt_builder(&HashMap::new(), None)
            .build_pending_deliveries(&[message], &HashMap::new())
            .await;

        assert_eq!(deliveries[0].prompt_text, "/compact");
    }

    #[tokio::test]
    async fn file_attachment_block_renders_on_disk_path() {
        let files = files_from(vec![FileInfo {
            file_id: "abc-123".to_string(),
            filename: "report.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            size: 1234,
        }]);

        let block =
            build_file_attachment_block(&files, "abc-123", "/opt/moatless/workspace/uploads")
                .await
                .expect("expected attachment block");

        let text = block.text.expect("expected text");
        assert!(
            text.contains("Saved to: /opt/moatless/workspace/uploads/abc-123/report.pdf"),
            "unexpected rendered text: {text}"
        );
        assert!(text.contains("Attached file: report.pdf"));
        assert!(text.contains("(application/pdf, 1234 bytes)"));
        assert!(
            !text.contains("Available at:"),
            "old URL hint should not appear: {text}"
        );
    }

    #[tokio::test]
    async fn file_attachment_block_trims_trailing_mount_slash() {
        let files = files_from(vec![FileInfo {
            file_id: "id".to_string(),
            filename: "x.txt".to_string(),
            mime_type: "text/plain".to_string(),
            size: 1,
        }]);

        let block = build_file_attachment_block(&files, "id", "/opt/moatless/workspace/uploads/")
            .await
            .expect("expected attachment block");

        let text = block.text.expect("expected text");
        assert!(text.contains("/opt/moatless/workspace/uploads/id/x.txt"));
        assert!(!text.contains("/opt/moatless/workspace/uploads//"));
    }
}
