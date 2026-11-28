//! One firing of a Loop with no external thread: open (or message) its Task
//! and deliver the first turn. A schedule tick and `POST /loops/{id}/run` both
//! fire through [`FiringPorts::fire`].

use std::sync::Arc;

use tracing::warn;

use crate::loops::dao::LoopDao;
use crate::loops::firing::metrics::record_schedule_firing;
use crate::loops::models::LoopSpawnConfig;
use moatless_adapter_model::registry::Adapters;
use moatless_ports::sandbox::SandboxWaker;
use moatless_ports::task::TaskReader;
use moatless_ports::task::{
    InboundTurn, MessageAcceptor, SandboxProvision, TaskOpenError, TaskOpener, TurnOrigin,
};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::{AdapterKind, Audience, InboundMessage, SCHEDULED_ORIGIN};
use moatless_types::loops::{LoopContext, RoutingMode};
use moatless_types::task::TaskStatus;

/// Always prepended to the `append_system_prompt` of a task a schedule or a
/// run-now fires, so a Loop prompt does not have to restate that the run is
/// unattended.
///
/// Without it a scheduled agent stops to ask a question nobody answers until
/// someone opens the task.
const SCHEDULE_ORIGIN_CONTEXT: &str = "\
# Autonomous Run

A schedule, or an administrator running the Loop by hand, started this run, and
no person is waiting on it. A question you ask here reaches somebody only when
they open the task later.

Do not ask for approval, for confirmation, or for a choice between options. Run
the instructions that follow to their end, including every step that creates,
changes, or assigns something.

When the instructions cover the case in front of you, follow them. When they do
not, take the most conservative action they allow. Say in your final message
what you did and what you left undone.

When a person does message this task, answer them and continue.";

/// What differs between a schedule tick and a run-now of the same Loop.
pub(crate) struct Firing {
    /// The Loop id; a schedule's id is its Loop's.
    pub loop_id: String,
    pub text: String,
    pub run_as_user_id: String,
    /// The person who asked for the run, recorded as the Task's reporter.
    pub requester: Option<String>,
    pub active: bool,
    pub adapter_config: serde_json::Value,
    /// Dedup key for the turn; unique per firing.
    pub event_id: String,
}

/// The ports a firing opens and messages a Task through.
pub struct FiringPorts {
    pub tasks: Arc<dyn TaskReader>,
    pub message_send: Arc<dyn MessageAcceptor>,
    pub adapters: Option<Arc<Adapters>>,
    pub task_creation: Arc<dyn TaskOpener>,
    pub sandbox_waker: Arc<dyn SandboxWaker>,
}

impl FiringPorts {
    /// Fire `route` once and return the Task that took the turn: the Loop's
    /// open Ongoing Task when it has one, else a new Task with `loop_id` set.
    pub(crate) async fn fire(
        &self,
        route: &LoopSpawnConfig,
        firing: &Firing,
        loops: &LoopDao,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let config = route_to_config(route, firing);
        let msg = InboundMessage {
            adapter_kind: AdapterKind::Webhook,
            source_id: firing.loop_id.clone(),
            thread_id: None,
            message_thread_id: format!("scheduled:{}", firing.loop_id),
            user_id: firing.run_as_user_id.clone(),
            // No external person authored the firing.
            author: None,
            sender_external_id: None,
            requester: firing.requester.clone(),
            text: firing.text.clone(),
            file_ids: vec![],
            attachments: Vec::new(),
            thread_properties: serde_json::json!({"schedule_id": firing.loop_id}),
            adapter_context: serde_json::json!({"schedule_id": firing.loop_id}),
            external_event_id: Some(firing.event_id.clone()),
            source_task_id: None,
            audience: Audience::Everyone,
            replies_to: Vec::new(),
            event: None,
        };

        let ongoing = match route.ongoing_task_id.as_deref() {
            Some(task_id) if route.routing_mode == RoutingMode::Ongoing => {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "the Ongoing Task belongs to the Loop, not to whoever fired it"
                )]
                let sys = ScopeContext::system();
                match self.tasks.describe(task_id, &sys).await {
                    Ok(Some(task)) if task.status == TaskStatus::Open => Some(task.task_id),
                    _ => None,
                }
            }
            _ => None,
        };
        let is_new_task = ongoing.is_none();
        let task_id = match ongoing {
            Some(task_id) => task_id,
            None => {
                open_task(
                    self.task_creation.as_ref(),
                    self.adapters.as_deref(),
                    &config,
                    &msg,
                )
                .await?
            }
        };

        if is_new_task && route.routing_mode == RoutingMode::Ongoing {
            loops.set_ongoing_task(&route.id, &task_id).await?;
        }

        self.message_send
            .accept(InboundTurn {
                task_id: task_id.clone(),
                text: firing.text.clone(),
                file_ids: Vec::new(),
                resume: true,
                origin: TurnOrigin {
                    event_id: msg.external_event_id.clone(),
                    adapter: Some(SCHEDULED_ORIGIN.to_string()),
                    source_name: Some(firing.loop_id.clone()),
                    ..TurnOrigin::default()
                },
                sent_by: None,
                skills: config.skills.clone(),
            })
            .await?;

        if !is_new_task {
            self.sandbox_waker.wake(&task_id).await?;
        }

        Ok(task_id)
    }
}

/// Free rather than a method on [`FiringPorts`]: it reads two of the five
/// ports, and a test drives it with a `TaskOpener` double alone.
async fn open_task(
    task_creation: &dyn TaskOpener,
    adapters: Option<&Adapters>,
    config: &LoopContext,
    msg: &InboundMessage,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Provisioning happens inside the creation service. Preserve the
    // `provision_failed` metric using the provision outcome it reports back.
    // Resolve the bound adapter's reply instructions from the registry and
    // pass them to the firing entry point as a value.
    let presentation =
        adapters.and_then(|providers| providers.inbound_presentation_by_name(&config.adapter_kind));
    let reply_instructions = match presentation {
        Some(p) => p.reply_instructions().await,
        None => None,
    };
    let outcome = match crate::loops::firing::inbound::create_task_for_external_message(
        task_creation,
        config,
        msg,
        None,
        reply_instructions,
        Some(SCHEDULE_ORIGIN_CONTEXT),
    )
    .await
    {
        Ok(outcome) => outcome,
        // Permanent failure (e.g. bad GitHub token): already persisted as
        // errored, with a `SandboxStatus::Error` event published. Record a
        // distinct metric so a broken token is separable from a transient
        // blip in the generic `execution_failed` counter; the per-tick cron
        // retry is inherent, left to Loop auto-disable.
        Err(e @ TaskOpenError::Refused(_)) => {
            warn!(loop_id = %config.id, error = %e, "Scheduled task preflight failed");
            record_schedule_firing("error", "preflight_failed", &config.id);
            return Err(Box::new(e));
        }
        Err(e) => return Err(Box::new(e)),
    };

    if outcome.provision == SandboxProvision::Failed {
        record_schedule_firing("error", "provision_failed", &config.id);
    }

    Ok(outcome.task_id)
}

fn route_to_config(route: &LoopSpawnConfig, firing: &Firing) -> LoopContext {
    LoopContext {
        id: firing.loop_id.clone(),
        adapter_kind: SCHEDULED_ORIGIN.to_string(),
        source_id: firing.loop_id.clone(),
        source_name: Some(firing.loop_id.clone()),
        workspace_id: route.workspace_id.clone(),
        routing_mode: route.routing_mode,
        prompt_template: route.prompt_template.clone(),
        active: firing.active,
        allow_direct_send: false,
        adapter_config: Some(firing.adapter_config.clone()),
        ongoing_task_id: route.ongoing_task_id.clone(),
        scope: route.scope,
        task_name: route.task_name.clone(),
        tag_ids: route.tag_ids.clone(),
        memory_preload: route.memory_preload.clone(),
        agent_properties: route.agent_properties.clone(),
        skills: route.skills.clone(),
        run_as_user_id: Some(firing.run_as_user_id.clone()),
        agent_profile: route.agent_profile,
        model: route.model.clone(),
        close_after_settled_days: route.close_after_settled_days,
        settle_after_hours: route.settle_after_hours,
        node_selector: route.node_selector.clone(),
        stop_when_done: route.stop_when_done.unwrap_or(false),
        room: route.room.clone(),
        created_by: route.created_by.clone(),
        created_at: route.created_at,
        updated_at: route.updated_at,
    }
}

#[cfg(test)]
mod tests;
