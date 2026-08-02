//! Durable schedule-firing runner.

use std::sync::Arc;

use chrono::Utc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::adapters::AdapterKind;
use crate::adapters::common::next_fire_at;
use crate::adapters::models::{Audience, InboundMessage, LoopContext, LoopSpawnConfig, Schedule};
use crate::leader::LeaderGuard;
use crate::loops::firing::metrics::record_schedule_firing;
use crate::shared_models::AgentMode;
use crate::state::AppState;
use crate::task::models::TaskStatus;
use moatless_backend_core::scope::ScopeContext;

/// Spawn the schedule-firing background task.
///
/// The join handle is returned for the root supervisor to own — durable
/// scheduled work stops firing if this loop ends.
pub fn spawn_cron_scheduler(
    state: AppState,
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

                    if let Err(e) = tick(&state, &worker_id).await {
                        error!(error = %e, "Schedule firing tick failed");
                        record_schedule_firing("error", "tick_failed", "");
                    }
                }
            }
        }
    })
}

async fn tick(
    state: &AppState,
    worker_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let locked_schedules = state.loops.claim_due_schedules(worker_id, 20).await?;

    for locked in locked_schedules {
        match execute_schedule_firing(state, &locked).await {
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
        if let Err(e) = state.loops.record_schedule_fire(&locked.id, next).await {
            record_schedule_firing("error", "record_fire_failed", &locked.id);
            warn!(schedule_id = %locked.id, error = %e, "Failed to record schedule fire");
        }
    }

    Ok(())
}

async fn execute_schedule_firing(
    state: &AppState,
    schedule: &Schedule,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let route = state
        .loops
        .find_loop_spawn_config(&schedule.loop_id)
        .await?;

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
    let task_id = if route.routing_mode == crate::adapters::models::RoutingMode::Ongoing {
        if let Some(task_id) = route.ongoing_task_id.as_deref() {
            #[allow(clippy::disallowed_methods)]
            // reason: schedule runner is a background system with no caller identity
            let sys = ScopeContext::system();
            match state.tasks.find_by_id(task_id, &sys).await {
                Ok(Some(task)) if task.status == TaskStatus::Open => task.task_id,
                _ => {
                    is_new_task = true;
                    create_scheduled_task(state, &config, &msg).await?
                }
            }
        } else {
            is_new_task = true;
            create_scheduled_task(state, &config, &msg).await?
        }
    } else {
        is_new_task = true;
        create_scheduled_task(state, &config, &msg).await?
    };

    if is_new_task && route.routing_mode == crate::adapters::models::RoutingMode::Ongoing {
        state.loops.set_ongoing_task(&route.id, &task_id).await?;
    }

    crate::message::send::send_message_flow(
        state,
        &task_id,
        &schedule.message_template,
        None,
        Option::<&AgentMode>::None,
        None,
        &[],
        &[],
        true,
        msg.external_event_id.as_deref(),
        Some("scheduled"),
        None,
        Some(&schedule.id),
        None,
        false,
        None,
        None,
    )
    .await?;

    if !is_new_task {
        crate::sandbox::notify::ensure_sandbox_and_notify(state, &task_id).await?;
    }

    Ok(())
}

async fn create_scheduled_task(
    state: &AppState,
    config: &LoopContext,
    msg: &InboundMessage,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Provisioning now happens inside the creation service. Preserve the
    // `provision_failed` metric using the provision outcome it reports back.
    let outcome = match crate::loops::firing::inbound::create_task_for_external_message(
        state, config, msg, None,
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
        Err(e @ crate::task::creation::TaskCreationError::Preflight(_)) => {
            warn!(loop_id = %config.id, error = %e, "Scheduled task preflight failed");
            record_schedule_firing("error", "preflight_failed", &config.id);
            return Err(Box::new(e));
        }
        Err(e) => return Err(Box::new(e)),
    };

    if outcome.provision == crate::task::creation::ProvisionOutcome::Failed {
        record_schedule_firing("error", "provision_failed", &config.id);
    }

    Ok(outcome.task.task_id)
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
