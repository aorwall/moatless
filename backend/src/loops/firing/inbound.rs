//! Inbound message processing — unified message → task flow.

use crate::adapters::models::LoopContext;
use crate::shared_models::AgentType;
use crate::state::AppState;
use crate::task::creation::{
    FirstMessage, NewTaskRequest, TaskCreationError, TaskCreationOutcome,
};
use crate::task::models::Visibility;
use crate::task::relations::RelatedTask;

use crate::adapters::models::InboundMessage;

/// Create a new task from an external message.
///
/// Translates the trigger context into a [`NewTaskRequest`] and hands the whole
/// create-then-provision sequence (short id, preflight, insert, tags, relations,
/// provision, task event) to [`TaskCreationService`]. No first message is passed
/// — the bindings runtime and schedule runner deliver it separately. `related_to`
/// carries the optional peer relation the bindings path computes; the schedule
/// runner passes `None`.
///
/// Surfaces the service's typed [`TaskCreationError`] verbatim so callers can
/// tell a permanent [`TaskCreationError::Preflight`] failure (a bad GitHub
/// token — retrying just inserts more errored rows) apart from a transient one
/// (`Db` / `ShortId` / `RepoNotFound`) worth retrying.
///
/// [`TaskCreationService`]: crate::task::creation::TaskCreationService
pub(crate) async fn create_task_for_external_message(
    state: &AppState,
    config: &LoopContext,
    msg: &InboundMessage,
    related_to: Option<RelatedTask>,
) -> Result<TaskCreationOutcome, TaskCreationError> {
    let append_system_prompt = {
        let mut parts = Vec::new();
        if let Some(ref tpl) = config.prompt_template {
            parts.push(tpl.to_string());
        }
        // Append provider-specific reply instructions (e.g. Slack mention syntax).
        // The universal `moat messages reply --message <id>` guidance lives in
        // the base sandbox-tools prompt — no per-adapter copy needed.
        if let Some(providers) = state.adapters.as_ref() {
            if let Some(provider) = providers.get_by_name(&config.provider) {
                if let Some(extra) = provider.reply_instructions() {
                    parts.push(extra);
                }
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        }
    };

    let req = NewTaskRequest {
        repository_id: config.repository_id.clone(),
        agent_type: AgentType(config.agent_type.clone()),
        name: config.task_name.clone(),
        description: Some(crate::bindings::outbound::truncate_text(&msg.text, 200)),
        branch: None, // Will use repo default
        created_by: Some(config.effective_user_id().to_string()),
        headless: true,
        visibility: Visibility::Private,
        scope: config.scope,
        team_id: config.team_id.clone(),
        owner_user_id: (config.scope == moatless_backend_core::scope::Scope::User)
            .then(|| config.effective_user_id().to_string()),
        trigger_id: Some(config.id.clone()),
        workspace_id: None,
        template_id: None,
        vibe: None,
        append_system_prompt,
        ttl_minutes: crate::task::models::DEFAULT_TTL_MINUTES,
        inactive_ttl_days: config
            .inactive_ttl_days
            .unwrap_or(crate::task::models::DEFAULT_INACTIVE_TTL_DAYS),
        tag_ids: config.tag_ids.clone(),
        parent: None,
        related_to,
        // Bindings runtime / schedule runner deliver the first message themselves.
        first_message: None,
        event_subtype: moatless_events::TaskSubType::Open,
        repository: None,
    };

    let provider = config.provider.clone();
    let source_id = config.source_id.clone();
    let outcome = state
        .task_creation
        .create(
            req,
            move |_task| async move {
                serde_json::json!({ "adapter": provider, "source_id": source_id })
            },
            |_task_id, _fm: FirstMessage| async move {},
        )
        .await?;

    Ok(outcome)
}

