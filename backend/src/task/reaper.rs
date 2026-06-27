//! Unified task reaper.
//!
//! Background job that periodically:
//! 1. Stops idle sandbox Deployments (short TTL — `ttl_minutes`).
//! 2. Auto-closes inactive Open tasks (long TTL — `inactive_ttl_days`).
//!
//! Uses K8s Lease-based leader election so only one replica runs the reaper.
//! Replaces the legacy `sandbox::idle_reaper` — both behaviours share one
//! cycle, one lease (`moatless-task-reaper`), and one activity overlay.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use k8s_openapi::api::apps::v1::Deployment;
use kube::api::{Api, ListParams};
use moatless_events::TaskSubType;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::dao::{TaskPatch, TaskRepository};
use super::models::TaskStatus;
use crate::config::Config;
use crate::event_handler::agent_handlers;
use crate::event_handler::models::AgentEvent;
use crate::events::EventPublisher;
use moatless_events::{EventType, SandboxEventPayload, SandboxSubType};
use crate::message::dao::MessageRepository;
use crate::sandbox::k8s::client::K8sClient;
use crate::sandbox::k8s::leader_election::LeaderElection;
use crate::sandbox::provider::SandboxProvider;
use crate::state::AppState;
use crate::task_event::dao::UnclosedTurn;

/// Minimum age of a stuck `active` turn before the reaper reconciles it.
///
/// Sits comfortably above normal turn latency so a slow-but-live turn is never
/// force-closed; only a turn whose terminal event was genuinely lost (e.g.
/// dropped during a Redis outage) survives past this floor.
const RECONCILE_STUCK_TURN_THRESHOLD_SECS: i64 = 5 * 60;

/// Errors that can occur during a reap cycle.
#[derive(Debug, thiserror::Error)]
enum ReaperError {
    #[error("Kubernetes API error: {0}")]
    Kube(#[from] kube::Error),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Spawn the unified task reaper as a background tokio task.
///
/// The reaper runs in a loop:
/// 1. Try to acquire leadership via K8s Lease (`"moatless-task-reaper"`).
/// 2. If leader, run a reap cycle (idle-sandbox stop + inactive-task close).
/// 3. Sleep for `config.sandbox.idle_check_interval_secs` before the next cycle.
#[allow(clippy::too_many_arguments)]
pub fn spawn_task_reaper(
    app_state: AppState,
    k8s: K8sClient,
    messages: Arc<dyn MessageRepository>,
    tasks: Arc<dyn TaskRepository>,
    sandbox: Arc<dyn SandboxProvider>,
    events: Option<Arc<dyn EventPublisher>>,
    config: Arc<Config>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    metrics::describe_counter!(
        "task_turns_reconciled_total",
        "Active task turns force-closed by the reaper after a lost terminal event"
    );
    let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
    let interval = Duration::from_secs(config.sandbox.idle_check_interval_secs);
    let default_ttl_minutes = config.sandbox.idle_timeout_minutes as i32;
    let default_inactive_ttl_days = config.sandbox.inactive_task_timeout_days as i32;

    tokio::spawn(async move {
        // Lease duration must exceed the check interval so the lease
        // doesn't expire between cycles. Use 2x the interval as buffer.
        let lease_duration_secs = (config.sandbox.idle_check_interval_secs * 2) as i32;
        let leader_election = LeaderElection::new_with_name(
            &k8s,
            identity.clone(),
            "moatless-task-reaper".to_string(),
        )
        .with_lease_duration(lease_duration_secs);

        loop {
            if cancel.is_cancelled() {
                info!("Task reaper shutting down (cancellation)");
                return;
            }

            match leader_election.try_acquire_or_renew().await {
                Ok(true) => {
                    debug!(identity = %identity, "Task reaper is leader, running reap cycle");
                    if let Err(e) = run_reap_cycle(
                        &app_state,
                        &k8s,
                        &messages,
                        &tasks,
                        &sandbox,
                        &events,
                        default_ttl_minutes,
                        default_inactive_ttl_days,
                    )
                    .await
                    {
                        warn!(error = %e, "Task reaper cycle failed");
                    }
                }
                Ok(false) => {
                    debug!(identity = %identity, "Task reaper not leader, skipping cycle");
                }
                Err(e) => {
                    warn!(error = %e, "Task reaper leader election error");
                }
            }

            tokio::select! {
                () = cancel.cancelled() => {
                    info!("Task reaper shutting down (cancellation)");
                    return;
                }
                () = tokio::time::sleep(interval) => {}
            }
        }
    })
}

/// Run one reap cycle: stop idle sandboxes, then close inactive Open tasks.
#[allow(clippy::too_many_arguments)]
async fn run_reap_cycle(
    app_state: &AppState,
    k8s: &K8sClient,
    messages: &Arc<dyn MessageRepository>,
    tasks: &Arc<dyn TaskRepository>,
    sandbox: &Arc<dyn SandboxProvider>,
    events: &Option<Arc<dyn EventPublisher>>,
    default_ttl_minutes: i32,
    default_inactive_ttl_days: i32,
) -> Result<(), ReaperError> {
    let sandbox_ids = list_running_sandbox_task_ids(k8s).await?;

    // Third pass: close turns left stuck `active` by a lost terminal event.
    // Independent of the idle/close passes below and best-effort, so a failure
    // here never blocks them.
    reconcile_orphaned_turns(app_state, &sandbox_ids).await;

    let open_ids = match tasks.list_open_task_ids().await {
        Ok(ids) => ids,
        Err(e) => {
            warn!(error = %e, "Failed to list open tasks, skipping inactive-task close");
            Vec::new()
        }
    };

    // Union the two sets so we fetch activity timestamps for everything we may act on.
    let mut all_ids_set: HashSet<String> = HashSet::with_capacity(sandbox_ids.len() + open_ids.len());
    all_ids_set.extend(sandbox_ids.iter().cloned());
    all_ids_set.extend(open_ids.iter().cloned());

    if all_ids_set.is_empty() {
        debug!("No running sandboxes and no open tasks");
        return Ok(());
    }

    let all_ids: Vec<String> = all_ids_set.iter().cloned().collect();
    debug!(
        sandbox_count = sandbox_ids.len(),
        open_count = open_ids.len(),
        unique_count = all_ids.len(),
        "Task reaper considering tasks"
    );

    // Build the latest-activity map: messages → sandbox_started_at → created_at_db.
    let mut latest_times = messages.get_latest_message_times(&all_ids).await?;

    match tasks.get_sandbox_started_times(&all_ids).await {
        Ok(started_times) => {
            for (task_id, started_at) in started_times {
                latest_times
                    .entry(task_id)
                    .and_modify(|existing| {
                        if started_at > *existing {
                            *existing = started_at;
                        }
                    })
                    .or_insert(started_at);
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to fetch sandbox start times, using message times only");
        }
    }

    let no_activity_ids: Vec<String> = all_ids
        .iter()
        .filter(|id| !latest_times.contains_key(*id))
        .cloned()
        .collect();

    if !no_activity_ids.is_empty() {
        match tasks.get_created_times(&no_activity_ids).await {
            Ok(created_times) => {
                for (task_id, created_at) in created_times {
                    latest_times.entry(task_id).or_insert(created_at);
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to fetch task creation times, skipping no-activity tasks");
            }
        }
    }

    // Per-task TTL maps. Missing entries fall back to the global defaults.
    let ttl_map = tasks.get_ttl_minutes_map(&all_ids).await.unwrap_or_else(|e| {
        warn!(error = %e, "Failed to fetch ttl_minutes map, falling back to default for all tasks");
        HashMap::new()
    });
    let inactive_map = tasks
        .get_inactive_ttl_days_map(&all_ids)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "Failed to fetch inactive_ttl_days map, falling back to default for all tasks");
            HashMap::new()
        });

    // Latest agent run-state per sandbox task, used to date the idle-stop floor
    // from when the agent last stopped (and to never reap a running agent).
    // Only sandbox tasks reach the reap branch, so query just those. Falls back
    // to the activity floor on error.
    let agent_lifecycle = app_state
        .task_events
        .get_latest_agent_lifecycle(&sandbox_ids)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "Failed to fetch agent lifecycle events, reaper falls back to activity floor");
            HashMap::new()
        });

    let sandbox_set: HashSet<&str> = sandbox_ids.iter().map(String::as_str).collect();
    let open_set: HashSet<&str> = open_ids.iter().map(String::as_str).collect();

    let now = Utc::now();
    let mut stopped_count = 0u32;
    let mut closed_count = 0u32;

    for task_id in &all_ids {
        let latest_at = latest_times.get(task_id).copied();
        let inactive_ttl_days = inactive_map
            .get(task_id)
            .copied()
            .unwrap_or(default_inactive_ttl_days);

        // Close supersedes stop — `auto_close_task` calls `cleanup`, which
        // tears down the Deployment regardless of whether it was scaled to 0.
        if open_set.contains(task_id.as_str())
            && decide_close(inactive_ttl_days, latest_at, now) == CloseDecision::Close
        {
            info!(
                task_id = %task_id,
                inactive_ttl_days,
                last_activity = ?latest_at,
                "Auto-closing inactive task"
            );
            auto_close_task(app_state, task_id, tasks, sandbox, events).await;
            closed_count += 1;
            continue;
        }

        if !sandbox_set.contains(task_id.as_str()) {
            continue;
        }

        // Fold the agent run-state into the idle-stop floor so a running agent
        // is never reaped and a stopped agent's TTL counts from when it stopped.
        let reap_floor = compute_reap_floor(agent_lifecycle.get(task_id), latest_at, now);

        let ttl_minutes = ttl_map.get(task_id).copied().unwrap_or(default_ttl_minutes);
        match decide_reap(ttl_minutes, reap_floor, now) {
            ReapDecision::Skip(reason) => {
                debug!(task_id = %task_id, reason, "Skipping sandbox reap");
            }
            ReapDecision::Keep => {}
            ReapDecision::Stop => {
                info!(
                    task_id = %task_id,
                    ttl_minutes,
                    last_activity = ?latest_at,
                    "Stopping idle sandbox"
                );
                // Stop the agent gracefully before scaling the Deployment to 0
                // so the pod exits promptly instead of riding its full
                // termination grace until SIGKILL (exit 137). Best-effort; an
                // already-idle agent makes this a no-op.
                crate::sandbox::notify::stop_agent_for_task(app_state, task_id).await;
                match sandbox.stop(task_id).await {
                    Ok(()) => {
                        info!(task_id = %task_id, "Idle sandbox stopped successfully");
                        stopped_count += 1;
                        // Emit a distinctly-tagged stop so the UI can render
                        // "Stopped (idle)" instead of the crash-ish lifecycle tag
                        // the watcher would otherwise downgrade (pod_failed →
                        // Stopped). `idle_reaped` is the disambiguator.
                        if let Some(events) = events {
                            events
                                .publish_sandbox_event_with_payload(
                                    task_id,
                                    SandboxSubType::Stopped,
                                    &SandboxEventPayload {
                                        message: format!(
                                            "Sandbox stopped after {ttl_minutes} min idle"
                                        ),
                                        lifecycle_event: Some("idle_reaped".to_string()),
                                        ..Default::default()
                                    },
                                )
                                .await;
                        }
                    }
                    Err(e) => {
                        warn!(task_id = %task_id, error = %e, "Failed to stop idle sandbox");
                    }
                }
            }
        }
    }

    if stopped_count > 0 || closed_count > 0 {
        info!(stopped_count, closed_count, "Task reaper cycle complete");
    } else {
        debug!("Task reaper cycle complete, nothing to do");
    }

    Ok(())
}

/// Reconcile turns stuck `active` because their terminal event was lost.
///
/// A turn is closed only once confirmed genuinely orphaned: either its sandbox
/// Deployment is gone (not in `running_sandbox_ids`) or its agent reports
/// `idle`/`stopped`. A turn whose agent is still `running`, or whose status
/// can't be determined, is left alone so a slow-but-live turn is never killed.
async fn reconcile_orphaned_turns(app_state: &AppState, running_sandbox_ids: &[String]) {
    let cutoff = Utc::now() - chrono::Duration::seconds(RECONCILE_STUCK_TURN_THRESHOLD_SECS);
    let stuck = match app_state
        .task_events
        .list_tasks_with_unclosed_active_turn(cutoff)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "reconcile: failed to list unclosed active turns");
            return;
        }
    };

    if stuck.is_empty() {
        return;
    }

    let running: HashSet<&str> = running_sandbox_ids.iter().map(String::as_str).collect();

    for turn in stuck {
        let orphaned = if running.contains(turn.task_id.as_str()) {
            // Sandbox is up — only orphaned if the agent has gone terminal.
            // Unknown status ⇒ conservative skip (treat as possibly live).
            matches!(agent_is_terminal(app_state, &turn.task_id).await, Some(true))
        } else {
            // No running Deployment ⇒ nothing can still be driving the turn.
            true
        };

        if !orphaned {
            debug!(task_id = %turn.task_id, "reconcile: turn still live, skipping");
            continue;
        }

        close_orphaned_turn(app_state, &turn).await;
    }
}

/// Returns `Some(true)` if the sandbox agent is `idle`/`stopped`, `Some(false)`
/// if it's still `running`, and `None` when the status can't be read (no client,
/// gRPC error) — the caller treats `None` conservatively as "possibly live".
async fn agent_is_terminal(app_state: &AppState, task_id: &str) -> Option<bool> {
    let client = match app_state.sandbox.sandbox_client(task_id).await {
        Ok(c) => c,
        Err(e) => {
            debug!(task_id, error = %e, "reconcile: no sandbox client for status check");
            return None;
        }
    };
    match client.get_agent_status().await {
        Ok(resp) => Some(matches!(resp.status.as_str(), "idle" | "stopped")),
        Err(e) => {
            debug!(task_id, error = %e, "reconcile: get_agent_status failed");
            None
        }
    }
}

/// Force a stuck turn to a terminal state by driving the same close path a real
/// `agent.error` takes: appends an error result message, flips the task back to
/// Open, and publishes `taskTurn/error` so the UI unblocks.
///
/// The synthetic event id is deterministic per `(task, turn)` so repeated
/// reconcile cycles (e.g. while Redis is still down and the terminal event
/// hasn't yet round-tripped into `task_events`) upsert the same message row
/// instead of stacking duplicates.
async fn close_orphaned_turn(app_state: &AppState, turn: &UnclosedTurn) {
    let turn_label = turn
        .turn_number
        .map_or_else(|| "unknown".to_string(), |n| n.to_string());
    let event_id = format!("reconcile-{}-{}", turn.task_id, turn_label);
    let timestamp = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let event = AgentEvent {
        event_id,
        event_type: EventType::Agent,
        sub_type: "error".to_string(),
        task_id: turn.task_id.clone(),
        timestamp,
        is_error: true,
        agent_type: turn.agent_type.clone(),
        turn_number: turn.turn_number,
        payload: serde_json::json!({
            "error": "The agent stopped without reporting a result. The backend closed this turn so you can continue.",
            "reason": "reconciled_orphaned_turn",
        }),
    };

    agent_handlers::handle_agent_error(app_state, &event).await;
    metrics::counter!("task_turns_reconciled_total").increment(1);
    info!(
        task_id = %turn.task_id,
        turn_number = ?turn.turn_number,
        "Reconciled orphaned active turn"
    );
}

/// Outcome of evaluating a sandbox for the idle-stop branch.
#[derive(Debug, PartialEq, Eq)]
enum ReapDecision {
    /// Sandbox is idle past its TTL — stop it.
    Stop,
    /// Sandbox is still within its TTL — leave it running.
    Keep,
    /// Cannot decide (no activity timestamp, or TTL disabled). Reason is for logs.
    Skip(&'static str),
}

/// Whether an `agent` lifecycle `sub_type` marks the agent's run as ended.
///
/// `started` is the only non-terminal run-state marker; `completed`,
/// `stopped`, and `error` all mean the agent stopped. The DAO query already
/// restricts to these four sub-types, so anything that isn't terminal here is
/// `started` (a running agent).
fn is_terminal_agent_sub_type(sub_type: &str) -> bool {
    matches!(sub_type, "completed" | "stopped" | "error")
}

/// Compute the idle-stop floor for a sandbox task, folding the latest agent
/// run-state (`(sub_type, stopped_at)`) into the raw activity floor.
///
/// - running agent (`started`, non-terminal) ⇒ `now` ⇒ `decide_reap` returns
///   `Keep`; a running agent is never reaped.
/// - stopped agent (terminal) ⇒ `max(activity_floor, stop_time)` so the TTL
///   counts from when the agent stopped, not from the last user message.
/// - no agent event ⇒ the `activity_floor` unchanged (message → sandbox-start →
///   created fallback).
fn compute_reap_floor(
    agent_event: Option<&(String, DateTime<Utc>)>,
    activity_floor: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    match agent_event {
        Some((sub_type, _)) if !is_terminal_agent_sub_type(sub_type) => Some(now),
        Some((_, stopped_at)) => Some(match activity_floor {
            Some(existing) if existing > *stopped_at => existing,
            _ => *stopped_at,
        }),
        None => activity_floor,
    }
}

/// Pure decision function: decide whether to reap a single sandbox.
///
/// `ttl_minutes == 0` means the task is pinned indefinitely — never reap.
/// A missing `latest_at` means we have no activity floor (no messages, no
/// sandbox-started timestamp, no creation time) and skip rather than risk
/// killing a brand-new sandbox.
fn decide_reap(
    ttl_minutes: i32,
    latest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> ReapDecision {
    if ttl_minutes <= 0 {
        return ReapDecision::Skip("ttl_minutes is 0 (indefinite)");
    }
    let Some(latest_at) = latest_at else {
        return ReapDecision::Skip("no activity timestamp");
    };
    let cutoff = now - chrono::Duration::minutes(ttl_minutes as i64);
    if latest_at < cutoff {
        ReapDecision::Stop
    } else {
        ReapDecision::Keep
    }
}

/// Outcome of evaluating an Open task for the auto-close branch.
#[derive(Debug, PartialEq, Eq)]
enum CloseDecision {
    /// Task has been inactive past its TTL — close it.
    Close,
    /// Task is still within its TTL — leave it open.
    Keep,
    /// Cannot decide (no activity timestamp, or TTL disabled).
    Skip(&'static str),
}

/// Pure decision function: decide whether to auto-close a single Open task.
fn decide_close(
    inactive_ttl_days: i32,
    latest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> CloseDecision {
    if inactive_ttl_days <= 0 {
        return CloseDecision::Skip("inactive_ttl_days is 0 (indefinite)");
    }
    let Some(latest_at) = latest_at else {
        return CloseDecision::Skip("no activity timestamp");
    };
    let cutoff = now - chrono::Duration::days(inactive_ttl_days as i64);
    if latest_at < cutoff {
        CloseDecision::Close
    } else {
        CloseDecision::Keep
    }
}

/// Mirror of [`crate::task::handlers::close_task`] for unattended use.
///
/// Best-effort: every step is logged on failure but never propagates — the
/// reaper loop must never panic.
async fn auto_close_task(
    app_state: &AppState,
    task_id: &str,
    tasks: &Arc<dyn TaskRepository>,
    sandbox: &Arc<dyn SandboxProvider>,
    events: &Option<Arc<dyn EventPublisher>>,
) {
    let agent_type = match tasks.find_by_id_internal(task_id).await {
        Ok(Some(row)) => Some(row.agent_type),
        Ok(None) => {
            warn!(task_id, "auto-close: task vanished mid-cycle");
            return;
        }
        Err(e) => {
            warn!(task_id, error = %e, "auto-close: find_by_id_internal failed");
            return;
        }
    };

    // Commit `Closed` BEFORE the teardown (the original order cleaned up first,
    // then wrote the status). With `Closed` persisted first, any concurrent
    // `auto_provision` / `get_pending_messages` (gated by Fix B) sees `Closed`
    // and bails. A teardown failure below can't leave the task non-terminal.
    if let Err(e) = tasks
        .update_task(
            task_id,
            &TaskPatch {
                status: Some(TaskStatus::Closed),
                ..Default::default()
            },
        )
        .await
    {
        warn!(task_id, error = %e, "auto-close: update_task failed");
        return;
    }

    // Stop the agent gracefully before tearing down K8s resources so the pod
    // exits promptly instead of riding its full termination grace until SIGKILL
    // (exit 137). Best-effort — an already-idle agent makes this a no-op.
    crate::sandbox::notify::stop_agent_for_task(app_state, task_id).await;
    if let Err(e) = sandbox.cleanup(task_id).await {
        warn!(task_id, error = %e, "auto-close: sandbox.cleanup failed (continuing)");
    }

    // A closed task is terminal — deactivate its bindings so inbound comments
    // don't resurrect it. Best-effort; the runtime guards are the backstop.
    if let Err(e) = crate::bindings::dao::PgTaskBindingRepository(app_state.db.clone())
        .deactivate_by_task(task_id)
        .await
    {
        warn!(task_id, error = %e, "auto-close: failed to deactivate bindings");
    }

    if let (Some(events), Some(at)) = (events, agent_type) {
        events
            .publish_task_event(
                task_id,
                TaskSubType::Closed,
                Some(at.as_str()),
                serde_json::json!({}),
            )
            .await;
    }
    info!(task_id, "Task auto-closed (inactive past TTL)");
}

/// List task IDs of all running sandbox Deployments (replicas > 0).
async fn list_running_sandbox_task_ids(k8s: &K8sClient) -> Result<Vec<String>, kube::Error> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels("app.kubernetes.io/managed-by=moatless");

    let list = deployments.list(&lp).await?;

    let task_ids: Vec<String> = list
        .items
        .into_iter()
        .filter(|dep| dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1) > 0)
        .filter_map(|dep| {
            dep.metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get("moatless.io/task-id").cloned())
        })
        .collect();

    Ok(task_ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(seconds_ago: i64) -> DateTime<Utc> {
        Utc::now() - chrono::Duration::seconds(seconds_ago)
    }

    // =========================================================================
    // decide_reap
    // =========================================================================

    #[test]
    fn ttl_zero_is_indefinite() {
        let now = Utc::now();
        let very_old = now - chrono::Duration::days(7);
        let decision = decide_reap(0, Some(very_old), now);
        assert_eq!(
            decision,
            ReapDecision::Skip("ttl_minutes is 0 (indefinite)")
        );
    }

    #[test]
    fn negative_ttl_is_treated_as_indefinite() {
        let now = Utc::now();
        let very_old = now - chrono::Duration::days(7);
        let decision = decide_reap(-1, Some(very_old), now);
        assert_eq!(
            decision,
            ReapDecision::Skip("ttl_minutes is 0 (indefinite)")
        );
    }

    #[test]
    fn missing_activity_skips() {
        let now = Utc::now();
        let decision = decide_reap(60, None, now);
        assert_eq!(decision, ReapDecision::Skip("no activity timestamp"));
    }

    #[test]
    fn within_ttl_keeps() {
        let now = Utc::now();
        let decision = decide_reap(60, Some(ts(30 * 60)), now);
        assert_eq!(decision, ReapDecision::Keep);
    }

    #[test]
    fn beyond_ttl_stops() {
        let now = Utc::now();
        let decision = decide_reap(60, Some(ts(90 * 60)), now);
        assert_eq!(decision, ReapDecision::Stop);
    }

    #[test]
    fn pinned_12h_keeps_after_2h_idle() {
        let now = Utc::now();
        let decision = decide_reap(720, Some(ts(2 * 60 * 60)), now);
        assert_eq!(decision, ReapDecision::Keep);
    }

    #[test]
    fn pinned_12h_stops_after_13h_idle() {
        let now = Utc::now();
        let decision = decide_reap(720, Some(ts(13 * 60 * 60)), now);
        assert_eq!(decision, ReapDecision::Stop);
    }

    // =========================================================================
    // decide_close
    // =========================================================================

    #[test]
    fn close_zero_inactive_ttl_is_indefinite() {
        let now = Utc::now();
        let very_old = now - chrono::Duration::days(30);
        let decision = decide_close(0, Some(very_old), now);
        assert_eq!(
            decision,
            CloseDecision::Skip("inactive_ttl_days is 0 (indefinite)")
        );
    }

    #[test]
    fn close_beyond_ttl_closes() {
        let now = Utc::now();
        // 7-day TTL, last activity 8 days ago → close.
        let decision =
            decide_close(7, Some(now - chrono::Duration::days(8)), now);
        assert_eq!(decision, CloseDecision::Close);
    }

    #[test]
    fn close_within_ttl_keeps() {
        let now = Utc::now();
        // 7-day TTL, last activity 1 day ago → keep.
        let decision =
            decide_close(7, Some(now - chrono::Duration::days(1)), now);
        assert_eq!(decision, CloseDecision::Keep);
    }

    // =========================================================================
    // is_terminal_agent_sub_type / compute_reap_floor (Fix E)
    // =========================================================================

    #[test]
    fn started_is_not_terminal() {
        assert!(!is_terminal_agent_sub_type("started"));
    }

    #[test]
    fn completed_stopped_error_are_terminal() {
        assert!(is_terminal_agent_sub_type("completed"));
        assert!(is_terminal_agent_sub_type("stopped"));
        assert!(is_terminal_agent_sub_type("error"));
    }

    #[test]
    fn running_agent_pins_floor_to_now() {
        // Latest agent event is `started` ⇒ agent running ⇒ floor is `now`,
        // regardless of how stale the message/activity floor is.
        let now = Utc::now();
        let stale = now - chrono::Duration::hours(5);
        let event = ("started".to_string(), now - chrono::Duration::hours(3));
        let floor = compute_reap_floor(Some(&event), Some(stale), now);
        assert_eq!(floor, Some(now));
    }

    #[test]
    fn running_agent_is_never_reaped_even_far_past_ttl() {
        // End-to-end through decide_reap: a running agent with a very stale
        // activity floor and a short TTL still Keeps.
        let now = Utc::now();
        let stale = now - chrono::Duration::hours(10);
        let event = ("started".to_string(), now - chrono::Duration::hours(9));
        let floor = compute_reap_floor(Some(&event), Some(stale), now);
        assert_eq!(decide_reap(60, floor, now), ReapDecision::Keep);
    }

    #[test]
    fn stopped_agent_floor_dates_from_stop_time() {
        // Terminal latest event, stop more recent than the message floor ⇒
        // floor = stop time, so the TTL counts from agent-stop.
        let now = Utc::now();
        let old_message = now - chrono::Duration::hours(5);
        let stopped_at = now - chrono::Duration::minutes(30);
        let event = ("completed".to_string(), stopped_at);
        let floor = compute_reap_floor(Some(&event), Some(old_message), now);
        assert_eq!(floor, Some(stopped_at));
        // 60-min TTL, stopped 30 min ago → still within TTL → Keep.
        assert_eq!(decide_reap(60, floor, now), ReapDecision::Keep);
    }

    #[test]
    fn stopped_agent_reaped_after_ttl_from_stop() {
        // Terminal latest event, stopped longer ago than the TTL ⇒ Stop.
        let now = Utc::now();
        let stopped_at = now - chrono::Duration::minutes(90);
        let event = ("stopped".to_string(), stopped_at);
        let floor = compute_reap_floor(Some(&event), None, now);
        assert_eq!(floor, Some(stopped_at));
        assert_eq!(decide_reap(60, floor, now), ReapDecision::Stop);
    }

    #[test]
    fn stopped_agent_uses_newer_activity_floor() {
        // A user message AFTER the agent stopped keeps the floor at the message
        // time (max of the two), so post-stop activity still counts.
        let now = Utc::now();
        let stopped_at = now - chrono::Duration::hours(2);
        let newer_message = now - chrono::Duration::minutes(10);
        let event = ("error".to_string(), stopped_at);
        let floor = compute_reap_floor(Some(&event), Some(newer_message), now);
        assert_eq!(floor, Some(newer_message));
    }

    #[test]
    fn no_agent_event_falls_back_to_activity_floor() {
        let now = Utc::now();
        let activity = now - chrono::Duration::minutes(45);
        let floor = compute_reap_floor(None, Some(activity), now);
        assert_eq!(floor, Some(activity));

        // None activity floor stays None (decide_reap then Skips).
        let floor_none = compute_reap_floor(None, None, now);
        assert_eq!(floor_none, None);
        assert_eq!(
            decide_reap(60, floor_none, now),
            ReapDecision::Skip("no activity timestamp")
        );
    }
}
