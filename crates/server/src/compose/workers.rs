use crate::{reaper, sandbox, state::AppState};
use moatless_core::{config::Config, leader::LeaderGuard};
use moatless_tasks::event_ingress::relay::IngressRelay;
use std::sync::Arc;
use tracing::info;

pub(crate) fn ingress_relay(app_state: &AppState) -> Option<IngressRelay> {
    Some(IngressRelay::new(
        app_state.ingress_outbox.clone(),
        app_state.events.clone()?,
    ))
}

pub(crate) fn leader(
    k8s: &sandbox::k8s::client::K8sClient,
    name: &str,
    lease_duration_secs: i32,
) -> Arc<dyn LeaderGuard> {
    let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
    let election = sandbox::k8s::leader_election::LeaderElection::new_with_name(
        k8s,
        identity,
        name.to_string(),
    )
    .with_lease_duration(lease_duration_secs);
    Arc::new(sandbox::k8s::leader_election::K8sLeaderGuard(election))
}

pub(crate) async fn event_connection(config: &Config) -> Option<redis::aio::ConnectionManager> {
    if !config.redis.event_handler_enabled {
        info!("Event handler consumer disabled (EVENT_HANDLER_ENABLED=false)");
        return None;
    }
    let redis_url = config.redis.url.as_ref()?;

    let client = match redis::Client::open(redis_url.as_str()) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to create Redis client for event handler");
            return None;
        }
    };
    let conn = match redis::aio::ConnectionManager::new_with_config(
        client,
        moatless_core::events::redis_manager_config(),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to connect Redis for event handler");
            return None;
        }
    };

    Some(conn)
}

pub(crate) fn task_reaper(
    app_state: &AppState,
    connector: Arc<dyn sandbox_proto::client::SandboxConnector>,
    k8s: sandbox::k8s::client::K8sClient,
) -> Option<reaper::TaskReaper> {
    if !app_state.config.sandbox.idle_reaper_enabled {
        info!("Task reaper disabled");
        return None;
    }
    let lease_duration_secs = (app_state.config.sandbox.idle_check_interval_secs * 2) as i32;
    let leader = leader(&k8s, "moatless-task-reaper", lease_duration_secs);
    let reaper = reaper::TaskReaper::new(
        connector,
        leader,
        app_state.messages.clone(),
        app_state.tasks.clone(),
        app_state.task_turns.clone(),
        app_state.task_events.clone(),
        app_state.sandbox.clone(),
        app_state.task_bindings.clone(),
        app_state.events.clone(),
        app_state.config.clone(),
        app_state.into(),
        k8s,
    );
    Some(reaper)
}

pub(crate) fn schedule_runner(
    app_state: &AppState,
) -> moatless_loops::loops::firing::schedule_runner::ScheduleRunner {
    moatless_loops::loops::firing::schedule_runner::ScheduleRunner::new(
        app_state.loops.clone(),
        app_state.task_reader.clone(),
        app_state.message_send.clone(),
        app_state.adapters.clone(),
        app_state.task_creation.clone(),
        app_state.sandbox_waker.clone(),
    )
}
