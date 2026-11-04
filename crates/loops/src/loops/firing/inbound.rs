//! Inbound message processing — unified message → task flow.

use moatless_ports::task::{NewTask, TaskOpenError, TaskOpened, TaskOpener};
use moatless_types::loops::LoopContext;
use moatless_types::task::RelatedTask;
use moatless_types::task::Visibility;

use moatless_types::adapter::InboundMessage;

/// Join `origin_prelude`, `prompt_template`, and `reply_instructions` with
/// `"\n\n"`, in that order, into the firing-time `append_system_prompt`.
///
/// `None` when all three are absent, so an unconfigured task keeps a `None`
/// append rather than an empty string; shared by every Subscription firing
/// path.
pub(crate) fn assemble_firing_prompt(
    reply_instructions: Option<&str>,
    prompt_template: Option<&str>,
    origin_prelude: Option<&str>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(prelude) = origin_prelude {
        parts.push(prelude.to_string());
    }
    if let Some(tpl) = prompt_template {
        parts.push(tpl.to_string());
    }
    if let Some(extra) = reply_instructions {
        parts.push(extra.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// Per-path inputs to [`create_task_from_loop`] — the fields that genuinely
/// differ between the chat, GitHub PR, and Linear firing paths.
///
/// Everything else on the resulting [`NewTask`] is derived uniformly from the
/// [`LoopContext`].
pub struct CreateTaskParams<'a> {
    /// Task name (chat/GitHub/Linear each compute their own, e.g. Linear falls
    /// back to `"{identifier} — {title}"`).
    pub name: Option<String>,
    /// Task description — already truncated by the caller.
    pub description: String,
    /// Visibility (chat = `Private`; GitHub PR / Linear = `Public`).
    pub visibility: Visibility,
    /// Optional origin-context prelude for the system prompt.
    pub origin_prelude: Option<&'a str>,
    /// Adapter-specific reply instructions appended to the firing system prompt
    /// (e.g. mention syntax). The caller resolves these from its own adapter
    /// and passes the value; `None` for adapters that supply none.
    pub reply_instructions: Option<String>,
    /// Optional peer relation recorded by the creation service.
    pub related_to: Option<RelatedTask>,
    /// `adapter`/source metadata attached to the `Open` task event.
    pub event_metadata: serde_json::Value,
    /// The verified user requesting the work, recorded as the Task's reporter.
    pub requested_by: Option<String>,
}

/// Single task-creation entry point for every Subscription firing path.
///
/// Hands create-then-provision to the task band with no first turn (every
/// adapter delivers its inbound prompt via the binding instead); a shared,
/// possibly global-scoped Loop always fires a scoped task, and callers see
/// [`TaskOpenError`] verbatim, including [`TaskOpenError::Refused`], which is
/// also counted as `moatless_loop_task_refused_total`.
pub async fn create_task_from_loop(
    task_creation: &dyn TaskOpener,
    config: &LoopContext,
    params: CreateTaskParams<'_>,
) -> Result<TaskOpened, TaskOpenError> {
    let append_system_prompt = assemble_firing_prompt(
        params.reply_instructions.as_deref(),
        config.prompt_template.as_deref(),
        params.origin_prelude,
    );

    let run_as = config.effective_user_id();
    // Pass the run-as even when it owns the Task: unpinned, a person who later
    // writes in Moatless becomes the driver and the Task acts as them.
    let reporter_user_id = params.requested_by.filter(|user| !user.is_empty());
    let run_as_user_id = Some(run_as.to_string());
    let outcome = task_creation
        .open(NewTask {
            workspace_id: config.workspace_id.clone(),
            agent_profile: config.agent_profile,
            model: config.model.clone(),
            name: params.name,
            description: Some(params.description),
            created_by: Some(run_as.to_string()),
            reporter_user_id,
            run_as_user_id,
            visibility: params.visibility,
            loop_id: config.id.clone(),
            append_system_prompt,
            ttl_minutes: moatless_tasks::task::models::DEFAULT_TTL_MINUTES,
            close_after_settled_days: config
                .close_after_settled_days
                .unwrap_or(moatless_tasks::task::models::DEFAULT_CLOSE_AFTER_SETTLED_DAYS),
            settle_after_hours: Some(
                config
                    .settle_after_hours
                    .unwrap_or(moatless_tasks::task::models::DEFAULT_LOOP_SETTLE_AFTER_HOURS),
            ),
            node_selector: config.node_selector.clone(),
            stop_when_done: config.stop_when_done,
            tag_ids: config.tag_ids.clone(),
            skills: config.skills.clone(),
            memory_preload: config.memory_preload.clone(),
            related_to: params.related_to,
            event_metadata: params.event_metadata,
            room: config.room.clone(),
        })
        .await
        .inspect_err(|e| {
            if matches!(e, TaskOpenError::Refused(_)) {
                super::metrics::record_task_refused(&config.id, &config.adapter_kind);
            }
        })?;

    Ok(outcome)
}

/// Create a task from a chat message or schedule firing: private, named by
/// the Loop, with a truncated message preview and `{adapter, source_id}`
/// event metadata. Its reporter is [`InboundMessage::requester`], never
/// `msg.user_id`.
pub(crate) async fn create_task_for_external_message(
    task_creation: &dyn TaskOpener,
    config: &LoopContext,
    msg: &InboundMessage,
    related_to: Option<RelatedTask>,
    reply_instructions: Option<String>,
    origin_prelude: Option<&str>,
) -> Result<TaskOpened, TaskOpenError> {
    create_task_from_loop(
        task_creation,
        config,
        CreateTaskParams {
            name: config.task_name.clone(),
            description: crate::bindings::outbound::truncate_text(&msg.text, 200),
            visibility: Visibility::Private,
            origin_prelude,
            related_to,
            reply_instructions,
            event_metadata: serde_json::json!({
                "adapter": config.adapter_kind,
                "source_id": config.source_id,
            }),
            requested_by: msg.requester.clone(),
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::Utc;
    use moatless_ports::task::SandboxProvision;
    use moatless_types::access_control::Scope;
    use moatless_types::agent::AgentProfile;
    use moatless_types::loops::RoutingMode;

    use super::*;

    #[derive(Default)]
    struct CapturingOpener(Mutex<Option<NewTask>>);

    #[async_trait::async_trait]
    impl TaskOpener for CapturingOpener {
        async fn open(&self, req: NewTask) -> Result<TaskOpened, TaskOpenError> {
            *self.0.lock().unwrap() = Some(req);
            Ok(TaskOpened {
                task_id: "task-1".to_string(),
                provision: SandboxProvision::Started,
            })
        }
    }

    fn loop_running_as(run_as: &str) -> LoopContext {
        let now = Utc::now();
        LoopContext {
            id: "loop-1".to_string(),
            adapter_kind: "linear".to_string(),
            source_id: "team-1".to_string(),
            source_name: None,
            workspace_id: "ws-1".to_string(),
            routing_mode: RoutingMode::BySubject,
            prompt_template: None,
            active: true,
            allow_direct_send: false,
            adapter_config: None,
            ongoing_task_id: None,
            scope: Scope::User,
            task_name: None,
            tag_ids: vec![],
            memory_preload: Vec::new(),
            skills: vec![],
            run_as_user_id: Some(run_as.to_string()),
            agent_profile: AgentProfile::ClaudeCode,
            model: None,
            close_after_settled_days: None,
            settle_after_hours: None,
            created_by: "admin".to_string(),
            created_at: now,
            updated_at: now,
            node_selector: None,
            stop_when_done: false,
            room: None,
        }
    }

    async fn opened_for(requested_by: Option<&str>) -> NewTask {
        let opener = CapturingOpener::default();
        create_task_from_loop(
            &opener,
            &loop_running_as("bot"),
            CreateTaskParams {
                name: None,
                description: String::new(),
                visibility: Visibility::Public,
                origin_prelude: None,
                reply_instructions: None,
                related_to: None,
                event_metadata: serde_json::json!({}),
                requested_by: requested_by.map(str::to_string),
            },
        )
        .await
        .expect("the opener accepts every task");
        let mut captured = opener.0.lock().unwrap();
        captured.take().expect("a task was opened")
    }

    #[tokio::test]
    async fn a_loop_task_records_its_reporter_separately_from_its_creator() {
        let task = opened_for(Some("ada")).await;
        assert_eq!(task.created_by.as_deref(), Some("bot"));
        assert_eq!(task.run_as_user_id.as_deref(), Some("bot"));
        assert_eq!(task.reporter_user_id.as_deref(), Some("ada"));
    }

    async fn opened_for_message(user_id: &str, requester: Option<&str>) -> NewTask {
        let opener = CapturingOpener::default();
        let msg = InboundMessage {
            adapter_kind: moatless_types::adapter::AdapterKind::Slack,
            source_id: "C1".to_string(),
            thread_id: None,
            message_thread_id: "1700000000.000100".to_string(),
            user_id: user_id.to_string(),
            author: None,
            sender_external_id: None,
            requester: requester.map(str::to_string),
            text: "fix it".to_string(),
            file_ids: vec![],
            attachments: Vec::new(),
            thread_properties: serde_json::json!({}),
            adapter_context: serde_json::json!({}),
            external_event_id: None,
            source_task_id: None,
            audience: moatless_types::adapter::Audience::Everyone,
            replies_to: vec![],
        };
        create_task_for_external_message(&opener, &loop_running_as("bot"), &msg, None, None, None)
            .await
            .expect("the opener accepts every task");
        let mut captured = opener.0.lock().unwrap();
        captured.take().expect("a task was opened")
    }

    #[tokio::test]
    async fn a_chat_messages_verified_requester_is_its_reporter() {
        let task = opened_for_message("ada", Some("ada")).await;
        assert_eq!(task.created_by.as_deref(), Some("bot"));
        assert_eq!(task.reporter_user_id.as_deref(), Some("ada"));
    }

    #[tokio::test]
    async fn a_chat_messages_user_id_alone_never_becomes_its_reporter() {
        let task = opened_for_message("ada", None).await;
        assert_eq!(task.created_by.as_deref(), Some("bot"));
        assert_eq!(task.reporter_user_id, None);
    }

    #[tokio::test]
    async fn a_scheduled_or_unmapped_firing_has_no_reporter() {
        for requested_by in [None, Some("")] {
            let task = opened_for(requested_by).await;
            assert_eq!(task.created_by.as_deref(), Some("bot"));
            assert_eq!(task.run_as_user_id.as_deref(), Some("bot"));
            assert_eq!(task.reporter_user_id, None);
        }
    }
}
