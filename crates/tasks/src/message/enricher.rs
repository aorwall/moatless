//! Message content enricher — construct agent-facing prompt text from user input.

use std::collections::HashMap;
use std::fmt::Write;

use serde_json::Value;

use super::contexts::task_message_context_to_input;
use super::models::{TaskMessage, TaskMessageContext};
use moatless_core::config::Config;
use crate::file_storage::dao::FileDao;
use crate::task::schemas::MessageContextInput;

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
    pub agent_type: Option<String>,
    pub agent_mode: Option<String>,
    /// Reasoning effort for the turn, in the agent's own vocabulary. `None`
    /// leaves the agent on its own default.
    pub effort: Option<String>,
    pub resume_session_id: Option<String>,
    pub resume: bool,
    pub start_fresh: bool,
    /// A person's answer, where this delivery is one. See [`PromptResponse`].
    pub prompt_response: Option<PromptResponse>,
}

/// A person's answer to an interactive tool call, as the sandbox resolves it.
///
/// Read off the answering row's own `tool_response` column rather than parsed
/// back out of the prompt text: the text is what an agent that has forgotten
/// asking is told, and text is not an answer per question.
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
                .map(|answers| {
                    answers
                        .iter()
                        .filter_map(|(question, given)| {
                            Some((question.clone(), given.as_str()?.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            approved: stored.get("approved").and_then(Value::as_bool) == Some(true),
            feedback: field("feedback").filter(|feedback| !feedback.trim().is_empty()),
        })
    }
}

/// Builds pending Message rows into the exact unit the sandbox submits.
///
/// The interface deliberately hides the transient prompt assembly rules from
/// the gRPC handler and sandbox: callers get text, upload ids, model metadata,
/// resume intent, and the uuids to acknowledge after harness acceptance.
pub struct AgentPromptBuilder<'a> {
    files: Option<&'a dyn FileDao>,
    config: Option<&'a Config>,
    task_agent_type: String,
    task_model: Option<String>,
    task_resume_session_id: Option<String>,
    sender_logins: &'a HashMap<String, String>,
}

impl<'a> AgentPromptBuilder<'a> {
    pub fn new(
        files: Option<&'a dyn FileDao>,
        config: Option<&'a Config>,
        task_agent_type: String,
        task_model: Option<String>,
        task_resume_session_id: Option<String>,
        sender_logins: &'a HashMap<String, String>,
    ) -> Self {
        Self {
            files,
            config,
            task_agent_type,
            task_model,
            task_resume_session_id,
            sender_logins,
        }
    }

    /// Build the pending deliveries for this fetch.
    ///
    /// Today the dispatcher submits one harness input per fetch. To preserve
    /// that invariant while moving batching ownership into the backend, this
    /// returns at most one delivery: all rows before the first `start_fresh`, or
    /// the leading `start_fresh` row alone.
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
                &self.task_agent_type,
            )
            .or(model);
            if row.agent_mode.is_some() {
                agent_mode = row.agent_mode.as_ref().map(|mode| mode.0.clone());
            }
            effort = row.effort.clone().or(effort);
        }

        if parts.is_empty() {
            tracing::warn!("Pending delivery has no prompt text");
            return Vec::new();
        }

        // At most one answer per delivery: an answer is sent on its own row, and
        // the first is the one the run is parked on. A second in the same batch
        // would be an answer to a call the first has already released.
        let prompt_response = rows.iter().find_map(PromptResponse::read);

        vec![AgentDelivery {
            message_uuids,
            prompt_response,
            prompt_text: parts.join("\n"),
            file_ids,
            model,
            agent_type: Some(self.task_agent_type.clone()),
            agent_mode,
            effort,
            resume_session_id: if resume {
                self.task_resume_session_id.clone()
            } else {
                None
            },
            resume,
            start_fresh,
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
        attributed_text(
            message.message_text.as_deref().unwrap_or_default(),
            message.source_task_id.as_deref(),
        )
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
/// An explicit per-message model is always preserved. When the message carries
/// no model, agents that pin a model (e.g. `claude-code-fable` -> `fable`) get it
/// forced here, since the catalog `default_model` is only a UI hint and is not
/// otherwise applied at runtime.
pub fn effective_model(
    row_model: Option<String>,
    task_model: Option<String>,
    agent_type: &str,
) -> Option<String> {
    row_model.or(task_model).or_else(|| match agent_type {
        "claude-code-fable" => Some("fable".to_string()),
        _ => None,
    })
}

fn build_message_header(row: &TaskMessage, login_cache: &HashMap<String, String>) -> String {
    if row.external_event_id.is_some() {
        let mut header = format!("<message_id>{}</message_id>\n", row.uuid);
        if let Some(source) = row.external_adapter.as_deref() {
            let _ = writeln!(header, "<source>{source}</source>");
        }
        if let Some(thread) = row.external_subject_key.as_deref() {
            let _ = writeln!(header, "<thread>{thread}</thread>");
        }
        header.push('\n');
        return header;
    }

    if let Some(uid) = row.sent_by.as_deref() {
        if let Some(login) = login_cache.get(uid) {
            return format!("<sent_by>{login}</sent_by>\n\n");
        }
    }
    String::new()
}

/// Build content blocks from user input + contexts + attachments.
///
/// The primary text block has `show_in_ui = true`; all context and file
/// blocks have `show_in_ui = false` (they appear only in the agent request).
pub async fn build_content_blocks(
    files: &dyn FileDao,
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
/// The agent has to be told where the message came from and how to answer it;
/// the person reading the transcript is looking at the Task that sent it and
/// does not. So this is applied on the way out and never stored — which also
/// means the reply instruction is regenerated from the current `source_task_id`
/// rather than frozen into a row at send time.
fn attributed_text(text: &str, source_task_id: Option<&str>) -> String {
    match source_task_id {
        Some(id) => format!(
            "Message from task `{id}`:\n\n{text}\n\n\
             To reply, run: `moat send --task {id} --text \"<your reply>\"`"
        ),
        None => text.to_string(),
    }
}

/// Rebuild the agent-facing content of a queued Message, at delivery time.
///
/// A user Message stores what the person wrote and nothing else. Everything
/// wrapped around it for the agent — the preview URL, the selected element, the
/// attribution to a sending Task, the attached files — is already persisted as
/// its own context row or column, so keeping a second, pre-rendered copy on the
/// Message only creates something that can disagree with them. This rebuilds it
/// from those rows on the way to the sandbox.
///
/// It is the read-side twin of [`build_content_blocks`], which the send path
/// used to call before writing the result down.
pub async fn build_pending_content_blocks(
    files: &dyn FileDao,
    config: &Config,
    message: &TaskMessage,
    contexts: &[TaskMessageContext],
) -> Vec<ContentBlock> {
    let text = attributed_text(
        message.message_text.as_deref().unwrap_or_default(),
        message.source_task_id.as_deref(),
    );

    // A row whose context type this build does not know is dropped, exactly as
    // the transform layer drops it — a forward-incompatible context must not
    // stop the message it decorates from reaching the agent.
    let inputs: Vec<MessageContextInput> = contexts
        .iter()
        .filter_map(task_message_context_to_input)
        .collect();

    build_content_blocks(files, config, &text, &inputs, &message.file_ids()).await
}

/// Build a content block from a file attachment reference.
///
/// `file_mount_path` is the on-disk directory the sandbox writes uploads to
/// before invoking the agent (defaults to `/mnt/uploads`). The prompt
/// advertises the resulting on-disk path so the agent can read the file
/// directly with no network round-trip.
async fn build_file_attachment_block(
    files: &dyn FileDao,
    file_id: &str,
    file_mount_path: &str,
) -> Option<ContentBlock> {
    let row = match files.find_by_id(file_id).await {
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
    use crate::file_storage::dao::MockFileDao;
    use crate::file_storage::models::FileMetadata;
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
        AgentPromptBuilder::new(
            None,
            None,
            "claude-code".to_string(),
            None,
            resume_session_id,
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

    #[test]
    fn effective_model_forces_fable_when_unset() {
        assert_eq!(
            effective_model(None, None, "claude-code-fable"),
            Some("fable".to_string())
        );
    }

    #[test]
    fn effective_model_uses_task_model_when_message_omits_one() {
        assert_eq!(
            effective_model(None, Some("claude-sonnet-4-6".to_string()), "claude-code"),
            Some("claude-sonnet-4-6".to_string())
        );
    }

    #[test]
    fn effective_model_preserves_explicit_model() {
        assert_eq!(
            effective_model(
                Some("claude-opus-4-6".to_string()),
                Some("fable".to_string()),
                "claude-code-fable",
            ),
            Some("claude-opus-4-6".to_string())
        );
    }

    #[test]
    fn effective_model_leaves_other_agents_unset() {
        assert_eq!(effective_model(None, None, "claude-code"), None);
        assert_eq!(effective_model(None, None, "codex"), None);
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
             <thread>T06:C0A:1781593153.718929</thread>\n\nhello"
        );
    }

    #[tokio::test]
    async fn internal_row_header_uses_resolved_sender() {
        let mut cache = HashMap::new();
        cache.insert("u1".to_string(), "alice".to_string());
        let mut message = pending_message(1, "msg_abc", "hello");
        message.sent_by = Some("u1".to_string());

        let deliveries = prompt_builder(&cache, None)
            .build_pending_deliveries(&[message], &HashMap::new())
            .await;

        assert_eq!(
            deliveries[0].prompt_text,
            "<sent_by>alice</sent_by>\n\nhello"
        );
    }

    #[tokio::test]
    async fn file_attachment_block_renders_on_disk_path() {
        let mut mock = MockFileDao::new();
        mock.expect_find_by_id().returning(|file_id| {
            Ok(Some(FileMetadata {
                file_id: file_id.to_string(),
                filename: "report.pdf".to_string(),
                mime_type: "application/pdf".to_string(),
                size: 1234,
                scope: moatless_backend_core::scope::Scope::User,
                owner_user_id: Some("user-1".to_string()),
                created_by: Some("user-1".to_string()),
                uploaded_at: chrono::Utc::now(),
            }))
        });

        let block = build_file_attachment_block(&mock, "abc-123", "/mnt/uploads")
            .await
            .expect("expected attachment block");

        let text = block.text.expect("expected text");
        assert!(
            text.contains("Saved to: /mnt/uploads/abc-123/report.pdf"),
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
        let mut mock = MockFileDao::new();
        mock.expect_find_by_id().returning(|file_id| {
            Ok(Some(FileMetadata {
                file_id: file_id.to_string(),
                filename: "x.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size: 1,
                scope: moatless_backend_core::scope::Scope::User,
                owner_user_id: Some("user-1".to_string()),
                created_by: Some("user-1".to_string()),
                uploaded_at: chrono::Utc::now(),
            }))
        });

        let block = build_file_attachment_block(&mock, "id", "/mnt/uploads/")
            .await
            .expect("expected attachment block");

        let text = block.text.expect("expected text");
        assert!(text.contains("/mnt/uploads/id/x.txt"));
        assert!(!text.contains("/mnt/uploads//"));
    }
}
