//! Durable schedule-firing runner.

use std::sync::Arc;

use chrono::Utc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::adapters::projection::next_fire_at;
use moatless_types::loops::LoopSpawnConfig;
use moatless_types::loops::Schedule;
use moatless_types::adapter::{AdapterKind, Audience, InboundMessage};
use moatless_types::loops::LoopContext;
use moatless_adapter_model::capabilities::AdapterInboundPresentation;
use moatless_adapter_model::registry::Adapters;
use crate::loops::dao::LoopDao;
use crate::loops::firing::metrics::record_schedule_firing;
use moatless_types::task::TaskStatus;
use moatless_types::access_control::ScopeContext;
use moatless_core::leader::LeaderGuard;
use moatless_backend_core::task::{
    InboundTurn, MessageAcceptor, SandboxProvision, TaskOpenError, TaskOpener, TurnOrigin,
};
use moatless_backend_core::task::TaskReader;
use moatless_backend_core::sandbox::SandboxWaker;

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
                    warn!(schedule_id = %locked.id, error = %e, "Failed to compute next fire time");
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
                        self.create_scheduled_task(&config, &msg).await?
                    }
                }
            } else {
                is_new_task = true;
                self.create_scheduled_task(&config, &msg).await?
            }
        } else {
            is_new_task = true;
            self.create_scheduled_task(&config, &msg).await?
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
                    adapter: Some("scheduled".to_string()),
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

    async fn create_scheduled_task(
        &self,
        config: &LoopContext,
        msg: &InboundMessage,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Provisioning now happens inside the creation service. Preserve the
        // `provision_failed` metric using the provision outcome it reports back.
        // Resolve the bound adapter's reply instructions from the registry and
        // pass them to the firing entry point as a value.
        let reply_instructions = self
            .adapters
            .as_ref()
            .and_then(|providers| providers.inbound_presentation_by_name(&config.adapter_kind))
            .and_then(AdapterInboundPresentation::reply_instructions);
        let outcome = match crate::loops::firing::inbound::create_task_for_external_message(
            self.task_creation.as_ref(),
            config,
            msg,
            None,
            reply_instructions,
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
}

fn route_to_config(route: &LoopSpawnConfig, schedule: &Schedule) -> LoopContext {
    LoopContext {
        id: schedule.id.clone(),
        adapter_kind: "scheduled".to_string(),
        source_id: schedule.id.clone(),
        source_name: Some(schedule.id.clone()),
        repository_id: route.repository_id.clone(),
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
        inactive_ttl_days: route.inactive_ttl_days,
        created_by: route.created_by.clone(),
        created_at: route.created_at,
        updated_at: route.updated_at,
    }
}
