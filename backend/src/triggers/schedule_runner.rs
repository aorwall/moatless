//! Durable scheduled-job runner.

use std::sync::Arc;

use chrono::Utc;
use cron::Schedule;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::adapters::models::{InboundMessage, ScheduledJob, TaskRoute, TriggerContext};
use crate::adapters::AdapterKind;
use crate::leader::LeaderGuard;
use crate::shared_models::AgentMode;
use crate::state::AppState;
use crate::task::models::TaskStatus;
use crate::triggers::metrics::record_job;

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
    });
}

async fn tick(
    state: &AppState,
    worker_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let due_jobs = sqlx::query_as::<_, ScheduledJob>(
        r#"SELECT t.id,
                  t.id AS route_id,
                  sts.cron_expression AS schedule_expr,
                  sts.timezone,
                  sts.message_template,
                  r.run_as_user_id,
                  t.active,
                  sts.last_fired_at,
                  sts.next_fire_at,
                  sts.locked_by,
                  sts.locked_until,
                  t.created_by,
                  t.scope,
                  t.team_id,
                  t.owner_user_id,
                  t.created_at,
                  t.updated_at
           FROM triggers t
           JOIN trigger_rules r ON r.trigger_id = t.id
           JOIN schedule_trigger_sources sts ON sts.trigger_id = t.id
           WHERE t.kind = 'schedule'
             AND t.active = true
             AND sts.next_fire_at <= now()
             AND (sts.locked_until IS NULL OR sts.locked_until < now())
           ORDER BY sts.next_fire_at ASC
           LIMIT 20"#,
    )
    .fetch_all(&state.db)
    .await?;

    for job in due_jobs {
        let locked = sqlx::query_as::<_, ScheduledJob>(
            r#"WITH locked_source AS (
                   UPDATE schedule_trigger_sources
                   SET locked_by = $2,
                       locked_until = now() + interval '2 minutes'
                   WHERE trigger_id = $1
                     AND next_fire_at <= now()
                     AND (locked_until IS NULL OR locked_until < now())
                   RETURNING *
               ),
               touched_trigger AS (
                   UPDATE triggers
                   SET updated_at = now()
                   WHERE id = $1 AND active = true AND kind = 'schedule'
                   RETURNING *
               )
               SELECT t.id,
                      t.id AS route_id,
                      s.cron_expression AS schedule_expr,
                      s.timezone,
                      s.message_template,
                      r.run_as_user_id,
                      t.active,
                      s.last_fired_at,
                      s.next_fire_at,
                      s.locked_by,
                      s.locked_until,
                      t.created_by,
                      t.scope,
                      t.team_id,
                      t.owner_user_id,
                      t.created_at,
                      t.updated_at
               FROM touched_trigger t
               JOIN trigger_rules r ON r.trigger_id = t.id
               JOIN locked_source s ON s.trigger_id = t.id"#,
        )
        .bind(&job.id)
        .bind(worker_id)
        .fetch_optional(&state.db)
        .await?;

        let Some(locked) = locked else {
            record_job("ignored", "no_lock", &job.id);
            continue;
        };

        match execute_job(state, &locked).await {
            Ok(()) => {
                record_job("success", "", &locked.id);
            }
            Err(e) => {
                record_job("error", "execution_failed", &locked.id);
                warn!(job_id = %locked.id, error = %e, "Scheduled job execution failed");
            }
        }

        let next = compute_next_fire(&locked.schedule_expr)?;
        sqlx::query(
            r#"UPDATE schedule_trigger_sources
               SET last_fired_at = now(),
                   next_fire_at = $2,
                   locked_by = NULL,
                   locked_until = NULL
               WHERE trigger_id = $1"#,
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
    let route = sqlx::query_as::<_, TaskRoute>(
        r#"SELECT t.id,
                  r.repository_id,
                  r.routing_mode,
                  r.ongoing_task_id,
                  r.prompt AS prompt_template,
                  r.task_name,
                  r.tag_ids,
                  r.run_as_user_id,
                  r.agent_type,
                  t.scope,
                  t.team_id,
                  t.owner_user_id,
                  t.created_by,
                  t.created_at,
                  t.updated_at
           FROM triggers t
           JOIN trigger_rules r ON r.trigger_id = t.id
           WHERE t.id = $1"#,
    )
    .bind(&job.route_id)
    .fetch_one(&state.db)
    .await?;

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
    };

    let mut is_new_task = false;
    let task_id = if route.routing_mode == crate::adapters::models::RoutingMode::Ongoing {
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

    if is_new_task && route.routing_mode == crate::adapters::models::RoutingMode::Ongoing {
        sqlx::query(
            r#"WITH updated_rule AS (
                   UPDATE trigger_rules
                   SET ongoing_task_id = $2
                   WHERE trigger_id = $1
                   RETURNING trigger_id
               )
               UPDATE triggers
               SET updated_at = now()
               WHERE id IN (SELECT trigger_id FROM updated_rule)"#,
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
        msg.external_event_id.as_deref(),
        Some("scheduled"),
        Some(&job.id),
        None,
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
    config: &TriggerContext,
    msg: &InboundMessage,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let task_id =
        crate::triggers::inbound::create_task_for_external_message(state, config, msg, None)
            .await?;

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
                record_job("error", "provision_failed", &config.id);
                warn!(task_id = %task_id, error = %e, "Failed to provision sandbox for scheduled task");
            }
        }
        Err(e) => {
            record_job("error", "provision_failed", &config.id);
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
