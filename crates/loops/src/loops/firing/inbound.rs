//! Inbound message processing — unified message → task flow.

use crate::adapters::models::LoopContext;
use moatless_core::shared_models::AgentType;
use moatless_tasks::task::creation::{
    FirstMessage, NewTaskRequest, TaskCreationError, TaskCreationOutcome, TaskCreationService,
};
use moatless_tasks::task::models::Visibility;
use moatless_tasks::task::relations::RelatedTask;

use crate::adapters::models::InboundMessage;

/// Assemble a task's firing-time `append_system_prompt` from the three sources
/// every Subscription firing path draws on, joined with `"\n\n"` in a fixed
/// order:
///
/// 1. `origin_prelude` — an optional origin-context line prepended ahead of
///    everything else.
/// 2. `prompt_template` — the loop/subscription's own prompt template.
/// 3. `reply_instructions` — the adapter-specific reply guidance (e.g. mention
///    syntax) the caller resolves from its own adapter. The universal
///    `moat messages reply --message <id>` guidance lives in the base
///    sandbox-tools prompt — no per-adapter copy needed.
///
/// Returns `None` when every source is absent, so a task with no configured
/// prompt keeps a `None` append (rather than an empty string). This is the one
/// firing-time prompt-assembly function shared by every Subscription firing
/// path; a path opts into a prelude purely via the `origin_prelude` argument.
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
/// differ between the chat, GitHub PR, and Linear firing paths. Everything else
/// on the resulting [`NewTaskRequest`] is derived uniformly from the
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
    /// Adapter-specific reply instructions appended to the firing system
    /// prompt (e.g. mention syntax). The caller resolves these from its own
    /// adapter and passes the value; `None` for adapters that supply none.
    pub reply_instructions: Option<String>,
    /// Optional peer relation recorded by the creation service.
    pub related_to: Option<RelatedTask>,
    /// `adapter`/source metadata attached to the `Open` task event.
    pub event_metadata: serde_json::Value,
}

/// Single task-creation entry point for every Subscription firing path.
///
/// Translates a [`LoopContext`] plus the per-path [`CreateTaskParams`] into a
/// [`NewTaskRequest`] and hands the whole create-then-provision sequence (short
/// id, preflight, insert, tags, relations, provision, task event) to
/// [`TaskCreationService`]. No first message is passed — every adapter delivers
/// its inbound prompt separately through the binding.
///
/// Surfaces the service's typed [`TaskCreationError`] verbatim so callers can
/// tell a permanent [`TaskCreationError::Preflight`] failure apart from a
/// transient one (`Db` / `ShortId` / `RepoNotFound`).
///
/// [`TaskCreationService`]: moatless_tasks::task::creation::TaskCreationService
pub async fn create_task_from_loop(
    task_creation: &TaskCreationService,
    config: &LoopContext,
    params: CreateTaskParams<'_>,
) -> Result<TaskCreationOutcome, TaskCreationError> {
    let append_system_prompt = assemble_firing_prompt(
        params.reply_instructions.as_deref(),
        config.prompt_template.as_deref(),
        params.origin_prelude,
    );

    let req = NewTaskRequest {
        repository_id: config.repository_id.clone(),
        agent_type: AgentType(config.agent_type.clone()),
        name: params.name,
        description: Some(params.description),
        branch: None, // Will use repo default
        model: None,
        created_by: Some(config.effective_user_id().to_string()),
        headless: true,
        visibility: params.visibility,
        scope: config.scope,
        owner_user_id: (config.scope == moatless_backend_core::scope::Scope::User)
            .then(|| config.effective_user_id().to_string()),
        loop_id: Some(config.id.clone()),
        workspace_id: None,
        template_id: None,
        vibe: None,
        append_system_prompt,
        ttl_minutes: moatless_tasks::task::models::DEFAULT_TTL_MINUTES,
        inactive_ttl_days: config
            .inactive_ttl_days
            .unwrap_or(moatless_tasks::task::models::DEFAULT_INACTIVE_TTL_DAYS),
        tag_ids: config.tag_ids.clone(),
        parent: None,
        related_to: params.related_to,
        // Bindings runtime / schedule runner deliver the first message themselves.
        first_message: None,
        task_id: None,
        provision: true,
        event_subtype: moatless_events::TaskSubType::Open,
        repository: None,
    };

    let metadata = params.event_metadata;
    let outcome = task_creation
        .create(
            req,
            move |_task| async move { metadata },
            |_task_id, _fm: FirstMessage| async move {},
        )
        .await?;

    Ok(outcome)
}

/// Create a new task from an external chat message (Slack / Telegram / Webhook)
/// or the schedule runner. Thin wrapper over [`create_task_from_loop`] that
/// fills in the chat-path shape: private visibility, no origin prelude, the
/// Loop's task name, a truncated message preview, and `{adapter, source_id}`
/// event metadata. `related_to` carries the optional peer relation the bindings
/// path computes; the schedule runner passes `None`.
pub(crate) async fn create_task_for_external_message(
    task_creation: &TaskCreationService,
    config: &LoopContext,
    msg: &InboundMessage,
    related_to: Option<RelatedTask>,
    reply_instructions: Option<String>,
) -> Result<TaskCreationOutcome, TaskCreationError> {
    create_task_from_loop(
        task_creation,
        config,
        CreateTaskParams {
            name: config.task_name.clone(),
            description: crate::bindings::outbound::truncate_text(&msg.text, 200),
            visibility: Visibility::Private,
            origin_prelude: None,
            related_to,
            reply_instructions,
            event_metadata: serde_json::json!({
                "adapter": config.adapter_kind,
                "source_id": config.source_id,
            }),
        },
    )
    .await
}
