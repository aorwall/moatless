//! Durable schedule-firing runner.

use std::sync::Arc;

use chrono::Utc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::loops::dao::LoopDao;
use crate::loops::firing::metrics::record_schedule_firing;
use crate::loops::schedule::next_fire_at;
use moatless_adapter_model::capabilities::AdapterInboundPresentation;
use moatless_adapter_model::registry::Adapters;
use moatless_core::leader::LeaderGuard;
use moatless_ports::sandbox::SandboxWaker;
use moatless_ports::task::TaskReader;
use moatless_ports::task::{
    InboundTurn, MessageAcceptor, SandboxProvision, TaskOpenError, TaskOpener, TurnOrigin,
};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::{AdapterKind, Audience, InboundMessage, SCHEDULED_ORIGIN};
use moatless_types::loops::LoopContext;
use crate::loops::models::LoopSpawnConfig;
use crate::loops::models::Schedule;
use moatless_types::task::TaskStatus;

/// Always prepended to the `append_system_prompt` of a task a schedule fires,
/// so a Loop prompt does not have to restate that the run is unattended.
///
/// Without it a scheduled agent reads the same prompt an interactive one does,
/// and stops to ask a question that nobody answers until the next person opens
/// the task.
const SCHEDULE_ORIGIN_CONTEXT: &str = "\
# Autonomous Run

A schedule started this run, and no person is waiting on it. A question you ask
here reaches somebody only when they open the task later.

Do not ask for approval, for confirmation, or for a choice between options. Run
the instructions that follow to their end, including every step that creates,
changes, or assigns something.

When the instructions cover the case in front of you, follow them. When they do
not, take the most conservative action they allow. Say in your final message
what you did and what you left undone.

When a person does message this task, answer them and continue.";

/// Durable schedule-firing runner: claims due schedules and fires each as an
/// inbound message, holding the ports it needs rather than the whole `AppState`.
pub struct ScheduleRunner {
    loops: Arc<LoopDao>,
    tasks: Arc<dyn TaskReader>,
    message_send: Arc<dyn MessageAcceptor>,
    adapters: Option<Arc<Adapters>>,
    task_creation: Arc<dyn TaskOpener>,
    sandbox_waker: Arc<dyn SandboxWaker>,
}

impl ScheduleRunner {
    /// Assemble the runner from the ports it fires schedules through.
    pub fn new(
        loops: Arc<LoopDao>,
        tasks: Arc<dyn TaskReader>,
        message_send: Arc<dyn MessageAcceptor>,
        adapters: Option<Arc<Adapters>>,
        task_creation: Arc<dyn TaskOpener>,
        sandbox_waker: Arc<dyn SandboxWaker>,
    ) -> Self {
        Self {
            loops,
            tasks,
            message_send,
            adapters,
            task_creation,
            sandbox_waker,
        }
    }

    /// Spawn the schedule-firing background task.
    ///
    /// The join handle is returned for the root supervisor to own — durable
    /// scheduled work stops firing if this loop ends.
    pub fn spawn(
        self,
        leader: Option<Arc<dyn LeaderGuard>>,
        cancel: CancellationToken,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!("Schedule firing runner started");
            let worker_id = std::env::var("HOSTNAME").unwrap_or_else(|_| "local".to_string());

            loop {
                tokio::select! {
                    () = cancel.cancelled() => {
                        info!("Schedule firing runner shutting down");
                        return;
                    }
                    () = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                        if let Some(ref guard) = leader {
                            match guard.try_acquire_or_renew().await {
                                Ok(true) => {}
                                Ok(false) => {
                                    debug!("Schedule firing runner not leader, skipping tick");
                                    record_schedule_firing("ignored", "not_leader", "");
                                    continue;
                                }
                                Err(e) => {
                                    warn!(error = %e, "Schedule firing leader election error, skipping tick");
                                    record_schedule_firing("error", "leader_election_failed", "");
                                    continue;
                                }
                            }
                        }

                        if let Err(e) = self.tick(&worker_id).await {
                            error!(error = %e, "Schedule firing tick failed");
                            record_schedule_firing("error", "tick_failed", "");
                        }
                    }
                }
            }
        })
    }

    async fn tick(&self, worker_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let locked_schedules = self.loops.claim_due_schedules(worker_id, 20).await?;

        for locked in locked_schedules {
            match self.execute_schedule_firing(&locked).await {
                Ok(()) => {
                    record_schedule_firing("success", "", &locked.id);
                }
                Err(e) => {
                    record_schedule_firing("error", "execution_failed", &locked.id);
                    warn!(schedule_id = %locked.id, error = %e, "Schedule firing execution failed");
                }
            }

            // Release the lock and reschedule this schedule. Contain failures per-schedule so a
            // single bad cron expression or DB hiccup doesn't strand the rest of the
            // already-locked batch until their 2-minute lock expiry.
            let next = match next_fire_at(&locked.cron_expression) {
                Ok(next) => next,
                Err(e) => {
                    record_schedule_firing("error", "next_fire_failed", &locked.id);
                    warn!(schedule_id = %locked.id, error = %format!("Invalid request: {e}"), "Failed to compute next fire time");
                    continue;
                }
            };
            if let Err(e) = self.loops.record_schedule_fire(&locked.id, next).await {
                record_schedule_firing("error", "record_fire_failed", &locked.id);
                warn!(schedule_id = %locked.id, error = %e, "Failed to record schedule fire");
            }
        }

        Ok(())
    }

    async fn execute_schedule_firing(
        &self,
        schedule: &Schedule,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let route = self.loops.find_loop_spawn_config(&schedule.loop_id).await?;

        let config = route_to_config(&route, schedule);
        let run_as_user_id = schedule
            .run_as_user_id
            .clone()
            .or_else(|| route.run_as_user_id.clone())
            .ok_or_else(|| {
                std::io::Error::other("claimed schedule Loop has no approved run-as user")
            })?;
        let msg = InboundMessage {
            adapter_kind: AdapterKind::Webhook,
            source_id: schedule.id.clone(),
            thread_id: None,
            message_thread_id: format!("scheduled:{}", schedule.id),
            user_id: run_as_user_id,
            // A schedule fires on its own; no external person authored it.
            author: None,
            sender_external_id: None,
            text: schedule.message_template.clone(),
            file_ids: vec![],
            thread_properties: serde_json::json!({"schedule_id": schedule.id}),
            adapter_context: serde_json::json!({"schedule_id": schedule.id}),
            external_event_id: Some(format!(
                "scheduled:{}:{}",
                schedule.id,
                Utc::now().timestamp()
            )),
            source_task_id: None,
            audience: Audience::Everyone,
        };

        let mut is_new_task = false;
        let task_id = if route.routing_mode == moatless_types::loops::RoutingMode::Ongoing {
            if let Some(task_id) = route.ongoing_task_id.as_deref() {
                #[allow(clippy::disallowed_methods)]
                // reason: schedule runner is a background system with no caller identity
                let sys = ScopeContext::system();
                match self.tasks.describe(task_id, &sys).await {
                    Ok(Some(task)) if task.status == TaskStatus::Open => task.task_id,
                    _ => {
                        is_new_task = true;
                        self.open_scheduled_task(&config, &msg).await?
                    }
                }
            } else {
                is_new_task = true;
                self.open_scheduled_task(&config, &msg).await?
            }
        } else {
            is_new_task = true;
            self.open_scheduled_task(&config, &msg).await?
        };

        if is_new_task && route.routing_mode == moatless_types::loops::RoutingMode::Ongoing {
            self.loops.set_ongoing_task(&route.id, &task_id).await?;
        }

        self.message_send
            .accept(InboundTurn {
                task_id: task_id.clone(),
                text: schedule.message_template.clone(),
                file_ids: Vec::new(),
                resume: true,
                origin: TurnOrigin {
                    event_id: msg.external_event_id.clone(),
                    adapter: Some(SCHEDULED_ORIGIN.to_string()),
                    source_name: Some(schedule.id.clone()),
                    ..TurnOrigin::default()
                },
                sent_by: None,
                skills: config.skills.clone(),
            })
            .await?;

        if !is_new_task {
            self.sandbox_waker.wake(&task_id).await?;
        }

        Ok(())
    }

    async fn open_scheduled_task(
        &self,
        config: &LoopContext,
        msg: &InboundMessage,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        create_scheduled_task(
            self.task_creation.as_ref(),
            self.adapters.as_deref(),
            config,
            msg,
        )
        .await
    }
}

/// Free rather than a method on [`ScheduleRunner`]: it reads two of the six
/// ports, and a test drives it with a `TaskOpener` double and no `LoopDao`.
async fn create_scheduled_task(
    task_creation: &dyn TaskOpener,
    adapters: Option<&Adapters>,
    config: &LoopContext,
    msg: &InboundMessage,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Provisioning happens inside the creation service. Preserve the
    // `provision_failed` metric using the provision outcome it reports back.
    // Resolve the bound adapter's reply instructions from the registry and
    // pass them to the firing entry point as a value.
    let reply_instructions = adapters
        .and_then(|providers| providers.inbound_presentation_by_name(&config.adapter_kind))
        .and_then(AdapterInboundPresentation::reply_instructions);
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
        // Permanent failure (e.g. a bad GitHub token). The service already
        // persisted the task as errored + published a `SandboxStatus::Error`
        // event. Record a distinct metric so a broken token is separable from a
        // transient blip in the generic `execution_failed` counter; the per-tick
        // cron retry is inherent and left to Loop auto-disable (a separate
        // ticket).
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

fn route_to_config(route: &LoopSpawnConfig, schedule: &Schedule) -> LoopContext {
    LoopContext {
        id: schedule.id.clone(),
        adapter_kind: SCHEDULED_ORIGIN.to_string(),
        source_id: schedule.id.clone(),
        source_name: Some(schedule.id.clone()),
        repository_id: route.repository_id.clone(),
        workspace_id: route.workspace_id.clone(),
        routing_mode: route.routing_mode,
        prompt_template: route.prompt_template.clone(),
        active: schedule.active,
        allow_direct_send: false,
        adapter_config: Some(serde_json::json!({
            "message": schedule.message_template,
            "schedule": schedule.cron_expression,
            "timezone": schedule.timezone,
        })),
        ongoing_task_id: route.ongoing_task_id.clone(),
        scope: route.scope,
        task_name: route.task_name.clone(),
        tag_ids: route.tag_ids.clone(),
        skills: route.skills.clone(),
        run_as_user_id: schedule
            .run_as_user_id
            .clone()
            .or_else(|| route.run_as_user_id.clone()),
        agent_type: route.agent_type.clone(),
        model: route.model.clone(),
        inactive_ttl_days: route.inactive_ttl_days,
        created_by: route.created_by.clone(),
        created_at: route.created_at,
        updated_at: route.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use moatless_ports::task::{NewTask, TaskOpened};
    use moatless_types::access_control::Scope;
    use moatless_types::agent::AgentType;
    use moatless_types::loops::RoutingMode;

    use super::*;

    /// Captures the `NewTask` the runner opens, which is where the assembled
    /// `append_system_prompt` becomes observable.
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

    fn schedule_context(prompt_template: Option<&str>) -> LoopContext {
        let now = Utc::now();
        LoopContext {
            id: "sch-1".to_string(),
            adapter_kind: SCHEDULED_ORIGIN.to_string(),
            source_id: "sch-1".to_string(),
            source_name: Some("sch-1".to_string()),
            repository_id: "repo-1".to_string(),
            workspace_id: None,
            routing_mode: RoutingMode::BySubject,
            prompt_template: prompt_template.map(str::to_string),
            active: true,
            allow_direct_send: false,
            adapter_config: None,
            ongoing_task_id: None,
            scope: Scope::User,
            task_name: Some("Docs Improver".to_string()),
            tag_ids: vec![],
            skills: vec![],
            run_as_user_id: Some("user-1".to_string()),
            agent_type: AgentType::from(AgentType::CLAUDE_CODE),
            model: None,
            inactive_ttl_days: None,
            created_by: "user-1".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    fn scheduled_message(text: &str) -> InboundMessage {
        InboundMessage {
            adapter_kind: AdapterKind::Webhook,
            source_id: "sch-1".to_string(),
            thread_id: None,
            message_thread_id: "scheduled:sch-1".to_string(),
            user_id: "user-1".to_string(),
            author: None,
            sender_external_id: None,
            text: text.to_string(),
            file_ids: vec![],
            thread_properties: serde_json::Value::Null,
            adapter_context: serde_json::Value::Null,
            external_event_id: Some("scheduled:sch-1:1780000000".to_string()),
            source_task_id: None,
            audience: Audience::Everyone,
        }
    }

    async fn opened_prompt(config: &LoopContext) -> Option<String> {
        let opener = CapturingOpener::default();
        create_scheduled_task(&opener, None, config, &scheduled_message("/docs-improver"))
            .await
            .expect("the opener accepts every task");
        let captured = opener.0.lock().unwrap().take();
        captured
            .expect("the runner opened a task")
            .append_system_prompt
    }

    /// The agent otherwise reads the same prompt an interactive one does. A
    /// scheduled `docs-improver` run stopped at its assignment step and asked
    /// the user to pick an option, which nobody was there to answer.
    #[tokio::test]
    async fn a_scheduled_task_is_told_that_nobody_is_waiting_on_it() {
        let config = schedule_context(Some("Run the `docs-improver` skill."));

        let prompt = opened_prompt(&config)
            .await
            .expect("a scheduled firing always has a prompt");

        assert!(prompt.starts_with(SCHEDULE_ORIGIN_CONTEXT));
        assert!(prompt.ends_with("Run the `docs-improver` skill."));
    }

    /// A Loop with no prompt of its own still gets the origin context.
    #[tokio::test]
    async fn the_origin_context_does_not_depend_on_a_loop_prompt() {
        let config = schedule_context(None);

        assert_eq!(
            opened_prompt(&config).await.as_deref(),
            Some(SCHEDULE_ORIGIN_CONTEXT)
        );
    }
}
