//! Inbound message processing — unified message → task flow.

use moatless_ports::task::{NewTask, TaskOpenError, TaskOpened, TaskOpener};
use moatless_types::loops::LoopContext;
use moatless_types::task::RelatedTask;
use moatless_types::task::Visibility;

use moatless_types::adapter::InboundMessage;

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
/// on the resulting [`NewTask`] is derived uniformly from the
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
/// [`NewTask`] and hands the whole create-then-provision sequence (short id,
/// preflight, insert, tags, relations, provision, task event) to the task band.
/// No first turn is passed — every adapter delivers its inbound prompt
/// separately through the binding.
///
/// A Loop is shared and may be global-scoped; the task it fires is not. The
/// task band owns that rule and derives scope from `created_by`.
///
/// Surfaces the port's typed [`TaskOpenError`] verbatim so callers can tell a
/// permanent [`TaskOpenError::Refused`] apart from a transient store failure.
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

    let outcome = task_creation
        .open(NewTask {
            workspace_id: config.workspace_id.clone(),
            agent_type: config.agent_type.clone(),
            model: config.model.clone(),
            name: params.name,
            description: Some(params.description),
            created_by: Some(config.effective_user_id().to_string()),
            visibility: params.visibility,
            loop_id: config.id.clone(),
            append_system_prompt,
            ttl_minutes: moatless_tasks::task::models::DEFAULT_TTL_MINUTES,
            inactive_ttl_days: config
                .inactive_ttl_days
                .unwrap_or(moatless_tasks::task::models::DEFAULT_INACTIVE_TTL_DAYS),
            tag_ids: config.tag_ids.clone(),
            skills: config.skills.clone(),
            related_to: params.related_to,
            event_metadata: params.event_metadata,
        })
        .await?;

    Ok(outcome)
}

/// Create a new task from a chat message (Slack / Telegram / Webhook) or a
/// schedule firing. Thin wrapper over [`create_task_from_loop`] that fills in
/// the shape both share: private visibility, the Loop's task name, a truncated
/// message preview, and `{adapter, source_id}` event metadata. `related_to`
/// carries the optional peer relation the bindings path computes.
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
        },
    )
    .await
}
