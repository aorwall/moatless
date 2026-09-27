use crate::compose::Ports;
use crate::{reaper, sandbox, state::AppState};
use moatless_core::{config::Config, leader::LeaderGuard};
use moatless_loops::compose::ScheduleRunnerInputs;
use std::sync::Arc;
use tracing::info;

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

/// Whether this process runs the fleet's event handler.
///
/// A shadow backend sets `EVENT_HANDLER_ENABLED=false` and shares the primary's
/// database, so the primary dispatches on its behalf.
pub(crate) fn event_handler_enabled(config: &Config) -> bool {
    if !config.event_handler_enabled {
        info!("Event dispatcher disabled (EVENT_HANDLER_ENABLED=false)");
        return false;
    }
    true
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
    ports: &Ports,
    app_state: &AppState,
) -> moatless_loops::loops::firing::schedule_runner::ScheduleRunner {
    ports.loop_graph.schedule_runner(ScheduleRunnerInputs {
        tasks: app_state.task_reader.clone(),
        message_send: app_state.message_send.clone(),
        adapters: app_state.adapters.clone(),
        task_creation: app_state.task_creation.clone(),
        sandbox_waker: app_state.sandbox_waker.clone(),
    })
}
