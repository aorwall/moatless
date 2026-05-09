//! Durable scheduled-job runner.

use std::sync::Arc;

use chrono::Utc;
use cron::Schedule;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::channel::models::{InboundMessage, ScheduledJob, TaskRoute, TriggerContext};
use crate::channel::provider::ChannelType;
use crate::leader::LeaderGuard;
use crate::shared_models::AgentMode;
use crate::state::AppState;
use crate::task::models::TaskStatus;

/// Spawn the scheduled-job background task.
pub fn spawn_cron_scheduler(
    state: AppState,
    leader: Option<Arc<dyn LeaderGuard>>,
    cancel: CancellationToken,
) {
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
                                continue;
                            }
                            Err(e) => {
                                warn!(error = %e, "Scheduled job leader election error, skipping tick");
                                continue;
                            }
                        }
                    }

                    if let Err(e) = tick(&state, &worker_id).await {
                        error!(error = %e, "Scheduled job tick failed");
                    }
                }
            }
        }
    });
}

async fn tick(
    state: &AppState,
    worker_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let due_jobs = sqlx::query_as::<_, ScheduledJob>(
        r#"SELECT *
           FROM scheduled_jobs
           WHERE active = true
             AND next_fire_at <= now()
             AND (locked_until IS NULL OR locked_until < now())
           ORDER BY next_fire_at ASC
           LIMIT 20"#,
    )
    .fetch_all(&state.db)
    .await?;

    for job in due_jobs {
        let locked = sqlx::query_as::<_, ScheduledJob>(
            r#"UPDATE scheduled_jobs
               SET locked_by = $2,
                   locked_until = now() + interval '2 minutes',
                   updated_at = now()
               WHERE id = $1
                 AND active = true
                 AND next_fire_at <= now()
                 AND (locked_until IS NULL OR locked_until < now())
               RETURNING *"#,
        )
        .bind(&job.id)
        .bind(worker_id)
        .fetch_optional(&state.db)
        .await?;

        let Some(locked) = locked else {
            continue;
        };

        if let Err(e) = execute_job(state, &locked).await {
            warn!(job_id = %locked.id, error = %e, "Scheduled job execution failed");
        }

        let next = compute_next_fire(&locked.schedule_expr)?;
        sqlx::query(
            r#"UPDATE scheduled_jobs
               SET last_fired_at = now(),
                   next_fire_at = $2,
                   locked_by = NULL,
                   locked_until = NULL,
                   updated_at = now()
               WHERE id = $1"#,
        )
        .bind(&locked.id)
        .bind(next)
        .execute(&state.db)
        .await?;
    }

    Ok(())
}

fn compute_next_fire(
    schedule_expr: &str,
) -> Result<chrono::DateTime<Utc>, Box<dyn std::error::Error + Send + Sync>> {
    let schedule: Schedule = schedule_expr.parse()?;
    schedule
        .after(&Utc::now())
        .next()
        .ok_or_else(|| "schedule has no future fire time".into())
}

async fn execute_job(
    state: &AppState,
    job: &ScheduledJob,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let route = sqlx::query_as::<_, TaskRoute>("SELECT * FROM task_routes WHERE id = $1")
        .bind(&job.route_id)
        .fetch_one(&state.db)
        .await?;

    let config = route_to_config(&route, job);
    let msg = InboundMessage {
        provider: ChannelType::Webhook,
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
        channel_external_id: Some(format!("scheduled:{}:{}", job.id, Utc::now().timestamp())),
    };

    let mut is_new_task = false;
    let task_id = if route.routing_mode == crate::channel::models::RoutingMode::Ongoing {
        if let Some(task_id) = route.ongoing_task_id.as_deref() {
            match state.tasks.find_by_id_internal(task_id).await {
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

    if is_new_task && route.routing_mode == crate::channel::models::RoutingMode::Ongoing {
        sqlx::query(
            "UPDATE task_routes SET ongoing_task_id = $2, updated_at = now() WHERE id = $1",
        )
        .bind(&route.id)
        .bind(&task_id)
        .execute(&state.db)
        .await?;
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
        msg.channel_external_id.as_deref(),
        Some("scheduled"),
        Some(&job.id),
        None,
        None,
        false,
    )
    .await?;

    if !is_new_task {
        crate::sandbox::notify::ensure_sandbox_and_notify(state, &task_id).await?;
    }

    Ok(())
}

async fn create_scheduled_task(
    state: &AppState,
    config: &TriggerContext,
    msg: &InboundMessage,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let task_id =
        crate::channel::inbound::create_task_for_channel(state, config, msg, None).await?;

    match crate::sandbox::auto_provision::build_provision_request(
        state.tasks.as_ref(),
        state.repositories.as_ref(),
        state.provider_tokens.as_ref(),
        state.plugin_repos.as_ref(),
        &task_id,
    )
    .await
    {
        Ok(request) => {
            if let Err(e) = state.sandbox.provision(&task_id, &request).await {
                warn!(task_id = %task_id, error = %e, "Failed to provision sandbox for scheduled task");
            }
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to build provision request for scheduled task");
        }
    }

    Ok(task_id)
}

fn route_to_config(route: &TaskRoute, job: &ScheduledJob) -> TriggerContext {
    TriggerContext {
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
            "schedule": job.schedule_expr,
            "timezone": job.timezone,
        })),
        ongoing_task_id: route.ongoing_task_id.clone(),
        scope: route.scope,
        team_id: route.team_id.clone(),
        task_name: route.task_name.clone(),
        tag_ids: route.tag_ids.clone(),
        run_as_user_id: job
            .run_as_user_id
            .clone()
            .or_else(|| route.run_as_user_id.clone()),
        agent_type: route.agent_type.clone(),
        created_by: route.created_by.clone(),
        created_at: route.created_at,
        updated_at: route.updated_at,
    }
}
