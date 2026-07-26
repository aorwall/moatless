//! Durable scheduled-job runner.

use std::sync::Arc;

use chrono::Utc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::adapters::common::next_fire_at;
use crate::adapters::models::{InboundMessage, LoopContext, LoopSpawnConfig, Schedule};
use crate::adapters::AdapterKind;
use crate::leader::LeaderGuard;
use crate::loops::firing::metrics::record_job;
use crate::shared_models::AgentMode;
use crate::state::AppState;
use crate::task::models::TaskStatus;
use moatless_backend_core::scope::ScopeContext;

/// Spawn the scheduled-job background task.
///
/// The join handle is returned for the root supervisor to own — durable
/// scheduled work stops firing if this loop ends.
pub fn spawn_cron_scheduler(
    state: AppState,
    leader: Option<Arc<dyn LeaderGuard>>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!("Scheduled job runner started");
        let worker_id = std::env::var("HOSTNAME").unwrap_or_else(|_| "local".to_string());

        loop {
            tokio::select! {
                () = cancel.cancelled() => {
                    info!("Scheduled job runner shutting down");
                    return;
                }
                () = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                    if let Some(ref guard) = leader {
                        match guard.try_acquire_or_renew().await {
                            Ok(true) => {}
                            Ok(false) => {
                                debug!("Scheduled job runner not leader, skipping tick");
                                record_job("ignored", "not_leader", "");
                                continue;
                            }
                            Err(e) => {
                                warn!(error = %e, "Scheduled job leader election error, skipping tick");
                                record_job("error", "leader_election_failed", "");
                                continue;
                            }
                        }
                    }

                    if let Err(e) = tick(&state, &worker_id).await {
                        error!(error = %e, "Scheduled job tick failed");
                        record_job("error", "tick_failed", "");
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
    let locked_jobs = state.loops.claim_due_schedules(worker_id, 20).await?;

    for locked in locked_jobs {
        match execute_job(state, &locked).await {
            Ok(()) => {
                record_job("success", "", &locked.id);
            }
            Err(e) => {
                record_job("error", "execution_failed", &locked.id);
                warn!(job_id = %locked.id, error = %e, "Scheduled job execution failed");
            }
        }

        // Release the lock and reschedule this job. Contain failures per-job so a
        // single bad cron expression or DB hiccup doesn't strand the rest of the
        // already-locked batch until their 2-minute lock expiry.
        let next = match next_fire_at(&locked.cron_expression) {
            Ok(next) => next,
            Err(e) => {
                record_job("error", "next_fire_failed", &locked.id);
                warn!(job_id = %locked.id, error = %e, "Failed to compute next fire time");
                continue;
            }
        };
        if let Err(e) = state.loops.record_schedule_fire(&locked.id, next).await {
            record_job("error", "record_fire_failed", &locked.id);
            warn!(job_id = %locked.id, error = %e, "Failed to record schedule fire");
        }
    }

    Ok(())
}

async fn execute_job(
    state: &AppState,
    job: &Schedule,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let route = state.loops.find_loop_spawn_config(&job.loop_id).await?;

    let config = route_to_config(&route, job);
    let msg = InboundMessage {
        provider: AdapterKind::Webhook,
        source_id: job.id.clone(),
        thread_id: None,
        message_thread_id: format!("scheduled:{}", job.id),
        user_id: job
            .run_as_user_id
            .clone()
            .or_else(|| route.run_as_user_id.clone())
            .unwrap_or_else(|| route.effective_user_id().to_string()),
        text: job.message_template.clone(),
        file_ids: vec![],
        thread_properties: serde_json::json!({"scheduled_job_id": job.id}),
        provider_context: serde_json::json!({"scheduled_job_id": job.id}),
        external_event_id: Some(format!("scheduled:{}:{}", job.id, Utc::now().timestamp())),
        source_task_id: None,
    };

    let mut is_new_task = false;
    let task_id = if route.routing_mode == crate::adapters::models::RoutingMode::Ongoing {
        if let Some(task_id) = route.ongoing_task_id.as_deref() {
            #[allow(clippy::disallowed_methods)] // reason: schedule runner is a background system with no caller identity
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
        &job.message_template,
        None,
        Option::<&AgentMode>::None,
        &[],
        &[],
        true,
        msg.external_event_id.as_deref(),
        Some("scheduled"),
        None,
        Some(&job.id),
        None,
        false,
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
        // cron retry is inherent and left to trigger auto-disable (a separate
        // ticket).
        Err(e @ crate::task::creation::TaskCreationError::Preflight(_)) => {
            warn!(trigger_id = %config.id, error = %e, "Scheduled task preflight failed");
            record_job("error", "preflight_failed", &config.id);
            return Err(Box::new(e));
        }
        Err(e) => return Err(Box::new(e)),
    };

    if outcome.provision == crate::task::creation::ProvisionOutcome::Failed {
        record_job("error", "provision_failed", &config.id);
    }

    Ok(outcome.task.task_id)
}

fn route_to_config(route: &LoopSpawnConfig, job: &Schedule) -> LoopContext {
    LoopContext {
        id: job.id.clone(),
        provider: "scheduled".to_string(),
        source_id: job.id.clone(),
        source_name: Some(job.id.clone()),
        repository_id: route.repository_id.clone(),
        routing_mode: route.routing_mode,
        prompt_template: route.prompt_template.clone(),
        active: job.active,
        allow_direct_send: false,
        provider_config: Some(serde_json::json!({
            "message": job.message_template,
            "schedule": job.cron_expression,
            "timezone": job.timezone,
        })),
        ongoing_task_id: route.ongoing_task_id.clone(),
        scope: route.scope,
        task_name: route.task_name.clone(),
        tag_ids: route.tag_ids.clone(),
        run_as_user_id: job
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
