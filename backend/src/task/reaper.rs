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
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Service};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, ListParams};
use moatless_events::{TaskSubType, TaskTurnSubType};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::dao::{TaskPatch, TaskRepository};
use super::models::TaskStatus;
use crate::config::{Config, ReconcileConfig};
use crate::event_handler::agent_handlers;
use crate::event_handler::models::AgentEvent;
use crate::events::EventPublisher;
use moatless_events::{EventType, SandboxEventPayload, SandboxSubType};
use crate::message::dao::MessageRepository;
use crate::sandbox::k8s::client::K8sClient;
use crate::sandbox::k8s::leader_election::LeaderElection;
use crate::sandbox::provider::SandboxProvider;
use crate::state::AppState;
use crate::task_turn::models::{TurnStatus, TurnTransition, UnclosedTurn};

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
    metrics::describe_counter!(
        "task_reconcile_remediations_total",
        "Desired-vs-actual reconcile remediations by the task reaper, labelled by `type` (reprovision/pending_scale_zero) and `outcome`"
    );
    let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
    let interval = Duration::from_secs(config.sandbox.idle_check_interval_secs);
    let default_ttl_minutes = config.sandbox.idle_timeout_minutes as i32;
    let default_inactive_ttl_days = config.sandbox.inactive_task_timeout_days as i32;
    let orphan_gc_enabled = config.sandbox.orphan_gc_enabled;
    let orphan_gc_retention_hours = config.sandbox.orphan_gc_retention_hours as i64;

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

        // Bounded re-provision attempts per task, kept across cycles for this
        // leader's lifetime. Not persisted — a leader change resets the budget,
        // which is acceptable for a best-effort backstop (a successful provision
        // clears the entry; the pending-timeout / orphan-turn passes are the
        // ultimate stops for a task that can never come up).
        let mut reprovision_attempts: HashMap<String, u32> = HashMap::new();

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
                        &config.reconcile,
                        &mut reprovision_attempts,
                        default_ttl_minutes,
                        default_inactive_ttl_days,
                        orphan_gc_enabled,
                        orphan_gc_retention_hours,
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
    reconcile: &ReconcileConfig,
    reprovision_attempts: &mut HashMap<String, u32>,
    default_ttl_minutes: i32,
    default_inactive_ttl_days: i32,
    orphan_gc_enabled: bool,
    orphan_gc_retention_hours: i64,
) -> Result<(), ReaperError> {
    // One cluster-wide Deployments LIST, shared by the idle-stop pass (running =
    // replicas > 0) and the desired-vs-actual reconcile pass (all Deployments).
    let deployments = list_sandbox_deployments(k8s).await?;
    let sandbox_ids: Vec<String> = deployments
        .iter()
        .filter(|d| d.desired_replicas > 0)
        .map(|d| d.task_id.clone())
        .collect();

    // Tasks whose latest turn is stuck `active` past the floor. Shared by the
    // reconcile pass (re-provision candidates) and the orphaned-turn closer.
    let cutoff = Utc::now() - chrono::Duration::seconds(RECONCILE_STUCK_TURN_THRESHOLD_SECS);
    let stuck_turns = match app_state
        .task_turns
        .list_unclosed_active_turns(cutoff)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "reconcile: failed to list unclosed active turns");
            Vec::new()
        }
    };

    // Desired-vs-actual reconcile pass (config-gated, all classes default OFF).
    // Returns the tasks it re-provisioned/is retrying so the orphaned-turn closer
    // below leaves their turns open instead of racing the recovery.
    let reprovisioned = run_reconcile_pass(
        app_state,
        k8s,
        tasks,
        sandbox,
        events,
        reconcile,
        &deployments,
        &stuck_turns,
        reprovision_attempts,
    )
    .await;

    // Third pass: close turns left stuck `active` by a lost terminal event.
    // Independent of the idle/close passes below and best-effort, so a failure
    // here never blocks them.
    reconcile_orphaned_turns(app_state, &sandbox_ids, &stuck_turns, &reprovisioned).await;

    // Orphan-GC pass: delete leftover sandbox K8s resources whose task no longer
    // exists or was closed past the retention window. Runs unconditionally (not
    // gated on there being running sandboxes/open tasks — the whole point is
    // resources whose task is gone) and best-effort so it never blocks the
    // idle/close passes below.
    if orphan_gc_enabled {
        gc_orphaned_resources(k8s, tasks, orphan_gc_retention_hours).await;
    }

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

    // Build the latest-activity map: messages → agent tool-call activity →
    // sandbox_started_at → created_at_db, keeping the newest across all sources.
    let mut latest_times = messages.get_latest_message_times(&all_ids).await?;

    let merge_newest = |latest_times: &mut HashMap<String, DateTime<Utc>>,
                        entries: HashMap<String, DateTime<Utc>>| {
        for (task_id, ts) in entries {
            latest_times
                .entry(task_id)
                .and_modify(|existing| {
                    if ts > *existing {
                        *existing = ts;
                    }
                })
                .or_insert(ts);
        }
    };

    // Fold in agent activity (tool calls + streamed chunks). A single long turn
    // emits no new conversation message and no lifecycle transition, so without
    // this the idle floor freezes at the last user message / sandbox start and
    // the reaper stops a working agent mid tool call.
    match app_state.task_events.get_latest_agent_activity(&all_ids).await {
        Ok(activity_times) => merge_newest(&mut latest_times, activity_times),
        Err(e) => {
            warn!(error = %e, "Failed to fetch agent activity times, idle floor omits tool-call activity");
        }
    }

    match tasks.get_sandbox_started_times(&all_ids).await {
        Ok(started_times) => merge_newest(&mut latest_times, started_times),
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
                // Defense in depth: the activity floor can go stale (dropped
                // events, watcher failover, delivery lag), so ask the agent
                // itself before pulling the plug. A positive "running" answer
                // skips the stop, and live background processes (e.g. a
                // `run_in_background` build that outlived the turn) defer it
                // up to the capped busy-extension window. An unreadable
                // status (no client, gRPC error) falls through, otherwise a
                // sandbox with a broken gRPC path could never be reaped.
                // Costs one gRPC call per reap candidate per cycle, against
                // a sandbox that is up.
                if let Some(status) = agent_status_snapshot(app_state, task_id).await {
                    if !matches!(status.status.as_str(), "idle" | "stopped") {
                        info!(
                            task_id = %task_id,
                            ttl_minutes,
                            last_activity = ?latest_at,
                            "Idle floor expired but agent reports running, skipping stop"
                        );
                        continue;
                    }
                    if status.busy_processes
                        && busy_extension_active(
                            reap_floor,
                            now,
                            app_state.config.sandbox.busy_extension_max_hours,
                        )
                    {
                        info!(
                            task_id = %task_id,
                            ttl_minutes,
                            last_activity = ?latest_at,
                            "Idle floor expired but sandbox reports live background processes, deferring stop"
                        );
                        continue;
                    }
                }
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
                                        initiator: Some("reaper".to_string()),
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

/// Reconcile turns stuck open because their terminal event was lost.
///
/// Reads the authoritative `task_turns` table (turns still `active` or
/// `interrupted` past the age floor) rather than deriving openness from the
/// event log — turn state is now committed transactionally at ingress, so this
/// is a pure backstop for the rare lost-terminal case.
///
/// A turn is closed only once confirmed genuinely orphaned: either its sandbox
/// Deployment is gone (not in `running_sandbox_ids`) or its agent reports
/// `idle`/`stopped`. A turn whose agent is still `running`, or whose status
/// can't be determined, is left alone so a slow-but-live turn is never killed.
async fn reconcile_orphaned_turns(
    app_state: &AppState,
    running_sandbox_ids: &[String],
    stuck: &[UnclosedTurn],
    exclude: &HashSet<String>,
) {
    if stuck.is_empty() {
        return;
    }

    let running: HashSet<&str> = running_sandbox_ids.iter().map(String::as_str).collect();

    for turn in stuck {
        // A task the reconcile pass is re-provisioning keeps its turn open so
        // agent recovery can resume it once the sandbox is back up.
        if exclude.contains(&turn.task_id) {
            debug!(task_id = %turn.task_id, "reconcile: turn owned by re-provision, skipping close");
            continue;
        }
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

        close_orphaned_turn(app_state, turn).await;
    }
}

/// Returns `Some(true)` if the sandbox agent is `idle`/`stopped`, `Some(false)`
/// if it's still `running`, and `None` when the status can't be read (no client,
/// gRPC error) — the caller treats `None` conservatively as "possibly live".
async fn agent_is_terminal(app_state: &AppState, task_id: &str) -> Option<bool> {
    agent_status_snapshot(app_state, task_id)
        .await
        .map(|resp| matches!(resp.status.as_str(), "idle" | "stopped"))
}

/// Live agent status straight from the sandbox, `None` when unreachable
/// (no client, gRPC error).
async fn agent_status_snapshot(
    app_state: &AppState,
    task_id: &str,
) -> Option<sandbox_proto::GetAgentStatusResponse> {
    let client = match app_state.sandbox.sandbox_client(task_id).await {
        Ok(c) => c,
        Err(e) => {
            debug!(task_id, error = %e, "reconcile: no sandbox client for status check");
            return None;
        }
    };
    match client.get_agent_status().await {
        Ok(resp) => Some(resp),
        Err(e) => {
            debug!(task_id, error = %e, "reconcile: get_agent_status failed");
            None
        }
    }
}

/// Whether live background processes may still defer an idle stop. The busy
/// extension is capped at `max_hours` past the activity floor so a leaked
/// daemon can't pin a sandbox forever; `max_hours == 0` disables the
/// extension, and without a floor there is nothing to anchor the cap to.
fn busy_extension_active(
    reap_floor: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    max_hours: i64,
) -> bool {
    if max_hours <= 0 {
        return false;
    }
    match reap_floor {
        Some(floor) => now - floor < chrono::Duration::hours(max_hours),
        None => false,
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
    // False-negative guard: if the agent already delivered its final reply for
    // this turn, the terminal event was merely lost — not the result. Closing
    // such a turn as `error` surfaces a misleading "stopped without reporting a
    // result" message even though the answer is right there in the chat. Close
    // it as `completed` instead.
    match app_state
        .messages
        .last_message_is_completed_reply(&turn.task_id, turn.turn_number)
        .await
    {
        Ok(true) => {
            close_orphaned_turn_as_completed(app_state, turn).await;
            return;
        }
        Ok(false) => {}
        Err(e) => {
            warn!(
                task_id = %turn.task_id,
                error = %e,
                "reconcile: failed to inspect last message; closing turn as error"
            );
        }
    }

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

/// Close a stuck turn whose agent had already delivered its final reply — its
/// terminal event was lost, not its result. Projects the turn to `completed`,
/// returns the task to Open, and emits `taskTurn/completed` so the UI unblocks,
/// all without appending a spurious error message.
///
/// The turn is projected directly (rather than relying solely on the published
/// event round-tripping) because a lost terminal event is the very failure this
/// path guards against — if the reconcile event were also lost the turn would
/// otherwise stay `active` and be reconciled again on the next cycle.
async fn close_orphaned_turn_as_completed(app_state: &AppState, turn: &UnclosedTurn) {
    if let Some(turn_number) = turn.turn_number {
        if let Err(e) = app_state
            .task_turns
            .project_turn(
                &turn.task_id,
                turn_number,
                turn.agent_type.clone(),
                TurnTransition::Terminal(TurnStatus::Completed),
                Utc::now(),
            )
            .await
        {
            warn!(
                task_id = %turn.task_id,
                error = %e,
                "reconcile: failed to project orphaned turn as completed"
            );
        }
    }

    // Return the task to Open unless it was closed mid-run (mirrors the agent
    // terminal handlers — a Closed task is never resurrected by automation).
    if let Err(e) = app_state
        .tasks
        .update_status_if_not_closed(&turn.task_id, TaskStatus::Open)
        .await
    {
        warn!(
            task_id = %turn.task_id,
            error = %e,
            "reconcile: failed to reopen task after completed reconcile"
        );
    }

    if let Some(ref events) = app_state.events {
        events
            .publish_task_turn_event(
                &turn.task_id,
                TaskTurnSubType::Completed,
                turn.agent_type.as_deref(),
                turn.turn_number,
                serde_json::json!({ "reason": "reconciled_orphaned_turn_completed" }),
            )
            .await;
    }

    metrics::counter!("task_turns_reconciled_total").increment(1);
    info!(
        task_id = %turn.task_id,
        turn_number = ?turn.turn_number,
        "Reconciled orphaned active turn as completed (agent had already replied)"
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
    // Auto-closed tasks are revivable too when storage retention is enabled.
    let retain_storage = app_state.config.sandbox.retain_storage_on_close;
    if let Err(e) = sandbox.cleanup(task_id, retain_storage).await {
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

/// Outcome of evaluating a labeled sandbox resource for orphan GC.
#[derive(Debug, PartialEq, Eq)]
enum OrphanDecision {
    /// The resource's task is gone, or was closed past the retention window —
    /// safe to delete.
    Delete,
    /// The task is still live (Open), errored (may be retried/inspected), or was
    /// closed only recently — keep the resource.
    Keep,
}

/// Pure decision function: decide whether a task's leftover sandbox resources
/// are orphans safe to GC.
///
/// `task` is the task's `(status, updated_at)` if the row still exists in the
/// DB, or `None` when it's been deleted. Rules, deliberately conservative:
/// - Deleted (`None`) → the resources can't belong to any task → delete.
/// - `Open` → live task → never GC (matches the plan's hard rule).
/// - `Error` → may be retried or inspected → keep.
/// - `Closed` → GC only once `updated_at` is older than the retention window; a
///   missing `updated_at` is treated as "recently changed" → keep.
fn decide_orphan(
    task: Option<(TaskStatus, Option<DateTime<Utc>>)>,
    retention: chrono::Duration,
    now: DateTime<Utc>,
) -> OrphanDecision {
    match task {
        None => OrphanDecision::Delete,
        Some((TaskStatus::Closed, updated_at)) => match updated_at {
            Some(ts) if now - ts > retention => OrphanDecision::Delete,
            _ => OrphanDecision::Keep,
        },
        Some(_) => OrphanDecision::Keep,
    }
}

/// Collect the `moatless.io/task-id` label from an iterator of resource metadata
/// into `ids`.
fn collect_ids_from_meta<'a>(
    ids: &mut HashSet<String>,
    metas: impl Iterator<Item = &'a ObjectMeta>,
) {
    for meta in metas {
        if let Some(id) = meta
            .labels
            .as_ref()
            .and_then(|l| l.get("moatless.io/task-id"))
        {
            ids.insert(id.clone());
        }
    }
}

/// Discover every task-id that still has at least one labeled sandbox resource
/// (Deployment, Service, PVC, or IngressRoute). Best-effort: a failure listing
/// any one resource type is logged and skipped.
async fn collect_labeled_task_ids(k8s: &K8sClient) -> HashSet<String> {
    let ns = k8s.namespace();
    let lp = ListParams::default().labels("moatless.io/task-id");
    let mut ids = HashSet::new();

    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), ns);
    match deployments.list(&lp).await {
        Ok(list) => collect_ids_from_meta(&mut ids, list.items.iter().map(|d| &d.metadata)),
        Err(e) => warn!(error = %e, "orphan-gc: failed to list Deployments"),
    }

    let services: Api<Service> = Api::namespaced(k8s.client.clone(), ns);
    match services.list(&lp).await {
        Ok(list) => collect_ids_from_meta(&mut ids, list.items.iter().map(|s| &s.metadata)),
        Err(e) => warn!(error = %e, "orphan-gc: failed to list Services"),
    }

    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(k8s.client.clone(), ns);
    match pvcs.list(&lp).await {
        Ok(list) => collect_ids_from_meta(&mut ids, list.items.iter().map(|p| &p.metadata)),
        Err(e) => warn!(error = %e, "orphan-gc: failed to list PVCs"),
    }

    for id in crate::sandbox::k8s::ingress::list_ingress_route_task_ids(k8s).await {
        ids.insert(id);
    }

    ids
}

/// Best-effort delete of every labeled K8s resource for a task. Each delete is
/// idempotent (404 → Ok) and its failure is logged, never propagated.
async fn delete_orphaned_task_resources(k8s: &K8sClient, task_id: &str) {
    use crate::sandbox::k8s::{deployment, ingress, pvc, service};

    if let Err(e) = deployment::delete_deployment(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete Deployment");
    }
    if let Err(e) = service::delete_sandbox_service(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete Service");
    }
    if let Err(e) = ingress::delete_task_ingress_routes(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete IngressRoutes");
    }
    if let Err(e) = pvc::delete_task_pvc(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete PVC");
    }
}

/// Orphan-GC pass: LIST labeled sandbox resources, then delete those whose task
/// no longer exists or was closed past `retention_hours`. Never touches
/// resources for Open (live) or Error tasks. Every deletion is logged at info
/// with `task_id`. Fully best-effort — errors are logged, never propagated.
async fn gc_orphaned_resources(
    k8s: &K8sClient,
    tasks: &Arc<dyn TaskRepository>,
    retention_hours: i64,
) {
    let retention = chrono::Duration::hours(retention_hours);
    let now = Utc::now();

    let task_ids = collect_labeled_task_ids(k8s).await;
    if task_ids.is_empty() {
        return;
    }

    let mut gc_count = 0u32;
    for task_id in task_ids {
        let task = match tasks.find_by_id_internal(&task_id).await {
            Ok(Some(t)) => Some((t.status, t.updated_at)),
            Ok(None) => None,
            Err(e) => {
                // A transient DB error must not be mistaken for a deleted task —
                // skip this task-id this cycle rather than risk deleting live
                // resources.
                warn!(task_id = %task_id, error = %e, "orphan-gc: task lookup failed, skipping");
                continue;
            }
        };

        if decide_orphan(task, retention, now) == OrphanDecision::Delete {
            info!(
                task_id = %task_id,
                retention_hours,
                "orphan-gc: deleting leftover sandbox resources (task deleted or closed past retention)"
            );
            delete_orphaned_task_resources(k8s, &task_id).await;
            gc_count += 1;
        }
    }

    if gc_count > 0 {
        info!(gc_count, "orphan-gc pass complete");
    }
}

/// A managed sandbox Deployment, reduced to what the reaper cycle needs.
struct DeploymentInfo {
    task_id: String,
    /// `spec.replicas` (defaults to 1 when unset). `0` means intentionally
    /// stopped; `> 0` means the sandbox is meant to be running.
    desired_replicas: i32,
}

/// List all managed sandbox Deployments (any replica count) with their task-id
/// and desired replicas via a single cluster-wide LIST. The idle-stop pass reads
/// `desired_replicas > 0`; the reconcile pass reads the full set to detect
/// orphaned resources and missing Deployments.
async fn list_sandbox_deployments(k8s: &K8sClient) -> Result<Vec<DeploymentInfo>, kube::Error> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels("app.kubernetes.io/managed-by=moatless");

    let list = deployments.list(&lp).await?;

    let infos = list
        .items
        .into_iter()
        .filter_map(|dep| {
            let task_id = dep
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get("moatless.io/task-id").cloned())?;
            let desired_replicas = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
            Some(DeploymentInfo {
                task_id,
                desired_replicas,
            })
        })
        .collect();

    Ok(infos)
}

/// A managed sandbox pod stuck in the `Pending` phase, with its age and the
/// scheduling reason if the scheduler reported one.
struct PendingPodInfo {
    task_id: String,
    pod_name: String,
    /// Seconds since the pod's `creationTimestamp`.
    age_secs: i64,
    /// Reason/message from the `False` `PodScheduled` condition, if present
    /// (e.g. `Unschedulable: 0/3 nodes are available…`). Read from the LIST
    /// result — no extra per-pod K8s GET.
    reason: Option<String>,
}

/// List managed sandbox pods currently in the `Pending` phase via a single
/// cluster-wide LIST. Only fetched when the pending-timeout remediation is
/// enabled, so a disabled reconcile pass adds no Pods LIST.
async fn list_pending_sandbox_pods(k8s: &K8sClient) -> Result<Vec<PendingPodInfo>, kube::Error> {
    let pods: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels("app.kubernetes.io/managed-by=moatless");

    let list = pods.list(&lp).await?;
    let now_ms = k8s_openapi::jiff::Timestamp::now().as_millisecond();

    let pending = list
        .items
        .into_iter()
        .filter_map(|pod| {
            let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref());
            if phase != Some("Pending") {
                return None;
            }
            let task_id = pod
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get("moatless.io/task-id").cloned())?;
            let pod_name = pod.metadata.name.clone().unwrap_or_default();
            let age_secs = pod
                .metadata
                .creation_timestamp
                .as_ref()
                .map_or(0, |t| ((now_ms - t.0.as_millisecond()).max(0)) / 1000);
            // Scheduling reason from the PodScheduled=False condition.
            let reason = pod
                .status
                .as_ref()
                .and_then(|s| s.conditions.as_ref())
                .and_then(|conds| {
                    conds
                        .iter()
                        .find(|c| c.type_ == "PodScheduled" && c.status == "False")
                })
                .and_then(|c| {
                    let msg = c.message.clone().unwrap_or_default();
                    let reason = c.reason.clone().unwrap_or_default();
                    match (reason.is_empty(), msg.is_empty()) {
                        (true, true) => None,
                        (false, true) => Some(reason),
                        (true, false) => Some(msg),
                        (false, false) => Some(format!("{reason}: {msg}")),
                    }
                });
            Some(PendingPodInfo {
                task_id,
                pod_name,
                age_secs,
                reason,
            })
        })
        .collect();

    Ok(pending)
}

// =============================================================================
// Reconcile pass (Phase 4): desired-vs-actual backstop
// =============================================================================

/// Outcome of the pure per-task deployment reconcile decision.
#[derive(Debug, PartialEq, Eq)]
enum ReconcileAction {
    /// Deployment missing for a live task mid-turn — re-provision it.
    Reprovision,
    /// Nothing to do.
    None,
}

/// Pure inputs for the per-task deployment reconcile decision. Kept free of any
/// K8s/DB types so the decision logic is unit-testable without a cluster.
///
/// Resources-present-but-task-gone cleanup is intentionally *not* handled here —
/// that orphan-GC lives in `gc_orphaned_resources` (see PR #270); this reconcile
/// pass focuses on re-provisioning a vanished Deployment for a live turn.
struct DeploymentReconcileInput {
    reprovision_enabled: bool,
    /// A managed Deployment exists for this task (any replica count).
    deployment_present: bool,
    /// The task row exists in the DB.
    task_present: bool,
    /// The task's status is `Open` (a live, resumable task).
    task_open: bool,
    /// The task has an in-flight (unclosed active) turn.
    has_active_turn: bool,
    /// A recent explicit stop was observed (user/agent stopped) — never fight it.
    recent_explicit_stop: bool,
    /// Re-provision attempts already spent this leader lifetime.
    reprovision_attempts: u32,
    max_reprovision_attempts: u32,
}

/// Pure decision function: given a task's desired (DB) vs actual (cluster) state,
/// decide the reconcile action. No I/O — the caller performs the side effect.
fn decide_deployment_reconcile(i: &DeploymentReconcileInput) -> ReconcileAction {
    // Re-provision only for a live, Open task that is mid-turn, whose Deployment
    // vanished, and that was not just stopped by a human — never fight an
    // in-flight stop. A present Deployment (even scaled to 0 while idle) is left
    // alone; orphaned-resource cleanup is handled by `gc_orphaned_resources`.
    if i.reprovision_enabled
        && !i.deployment_present
        && i.task_present
        && i.task_open
        && i.has_active_turn
        && !i.recent_explicit_stop
        && i.reprovision_attempts < i.max_reprovision_attempts
    {
        return ReconcileAction::Reprovision;
    }

    ReconcileAction::None
}

/// Pure decision for the pending-pod timeout remediation: a pod scheduled to a
/// task that has sat `Pending` at least `deadline_secs`.
fn should_scale_pending_pod(enabled: bool, age_secs: i64, deadline_secs: i64) -> bool {
    enabled && deadline_secs > 0 && age_secs >= deadline_secs
}

/// Run the config-gated desired-vs-actual reconcile pass.
///
/// Two independent, individually-gated remediation classes, all default OFF:
/// 1. **Re-provision** a Deployment that vanished for an Open task mid-turn.
/// 2. **Pending-timeout**: scale to 0 + surface an error for pods stuck Pending.
///
/// Resources-present-but-task-gone cleanup is handled separately by
/// `gc_orphaned_resources` (PR #270), so this pass deliberately does not touch
/// existing Deployments.
///
/// Cost when all classes are OFF: zero extra K8s/DB calls (the Deployments LIST
/// and stuck-turn query are already done by the caller). Returns the set of
/// tasks it re-provisioned/is retrying so the orphaned-turn closer keeps their
/// turns open.
#[allow(clippy::too_many_arguments)]
async fn run_reconcile_pass(
    app_state: &AppState,
    k8s: &K8sClient,
    tasks: &Arc<dyn TaskRepository>,
    sandbox: &Arc<dyn SandboxProvider>,
    events: &Option<Arc<dyn EventPublisher>>,
    reconcile: &ReconcileConfig,
    deployments: &[DeploymentInfo],
    stuck_turns: &[UnclosedTurn],
    reprovision_attempts: &mut HashMap<String, u32>,
) -> HashSet<String> {
    let mut reprovisioned: HashSet<String> = HashSet::new();

    let deployment_ids: HashSet<&str> =
        deployments.iter().map(|d| d.task_id.as_str()).collect();

    // ---- Class 1: re-provision missing Deployments for in-flight turns ----
    if reconcile.reprovision_enabled {
        let candidates: Vec<&UnclosedTurn> = stuck_turns
            .iter()
            .filter(|t| !deployment_ids.contains(t.task_id.as_str()))
            .collect();

        if !candidates.is_empty() {
            let ids: Vec<String> = candidates.iter().map(|t| t.task_id.clone()).collect();
            match tasks.get_task_statuses(&ids).await {
                Ok(statuses) => {
                    // Latest agent run-state: a `stopped` marker means an explicit
                    // stop we must not fight.
                    let lifecycle = app_state
                        .task_events
                        .get_latest_agent_lifecycle(&ids)
                        .await
                        .unwrap_or_else(|e| {
                            warn!(error = %e, "reconcile: agent lifecycle fetch failed, assuming no recent stop");
                            HashMap::new()
                        });

                    for turn in candidates {
                        let id = &turn.task_id;
                        let recent_explicit_stop = lifecycle
                            .get(id)
                            .is_some_and(|(sub_type, _)| sub_type == "stopped");
                        let attempts = reprovision_attempts.get(id).copied().unwrap_or(0);
                        let action = decide_deployment_reconcile(&DeploymentReconcileInput {
                            reprovision_enabled: true,
                            deployment_present: false,
                            task_present: statuses.contains_key(id),
                            task_open: statuses.get(id).copied() == Some(TaskStatus::Open),
                            has_active_turn: true,
                            recent_explicit_stop,
                            reprovision_attempts: attempts,
                            max_reprovision_attempts: reconcile.max_reprovision_attempts,
                        });
                        if action != ReconcileAction::Reprovision {
                            continue;
                        }
                        // Keep the turn open while we recover; drop it only if this
                        // attempt exhausts the budget (then the orphaned-turn closer
                        // finalises it and surfaces the error).
                        let exhausted =
                            reprovision_task(app_state, tasks, sandbox, events, reconcile, id, reprovision_attempts)
                                .await;
                        if !exhausted {
                            reprovisioned.insert(id.clone());
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "reconcile: get_task_statuses failed, skipping re-provision pass");
                }
            }
        }
    }

    // ---- Class 2 (orphan cleanup: resources present but task gone) is
    // deliberately omitted here — it is owned by `gc_orphaned_resources`
    // (PR #270), so the numbering skips straight to Class 3. ----

    // ---- Class 3: pending-pod hard-deadline timeout ----
    if reconcile.pending_timeout_enabled {
        let deadline_secs = (reconcile.pending_timeout_minutes as i64) * 60;
        match list_pending_sandbox_pods(k8s).await {
            Ok(pending) => {
                for pod in pending {
                    if !should_scale_pending_pod(true, pod.age_secs, deadline_secs) {
                        continue;
                    }
                    scale_stuck_pending_pod(k8s, events, &pod).await;
                }
            }
            Err(e) => {
                warn!(error = %e, "reconcile: pending-pod LIST failed, skipping pending-timeout pass");
            }
        }
    }

    reprovisioned
}

/// Attempt one re-provision of a task whose Deployment vanished. Returns `true`
/// when this attempt exhausted the retry budget (an environment error was
/// surfaced), `false` while still recovering or after a successful provision.
async fn reprovision_task(
    app_state: &AppState,
    tasks: &Arc<dyn TaskRepository>,
    sandbox: &Arc<dyn SandboxProvider>,
    events: &Option<Arc<dyn EventPublisher>>,
    reconcile: &ReconcileConfig,
    task_id: &str,
    reprovision_attempts: &mut HashMap<String, u32>,
) -> bool {
    let request = match crate::sandbox::auto_provision::build_provision_request(
        tasks.as_ref(),
        app_state.repositories.as_ref(),
        app_state.provider_tokens.as_ref(),
        app_state.plugin_repos.as_ref(),
        task_id,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            // A request that can never be built (repo row deleted, credentials
            // revoked) is a terminal failure — count it against the SAME budget
            // as a failed `provision()`. Otherwise the task would loop forever:
            // the turn is kept open every cycle and no `environment_error` is ever
            // surfaced, which is exactly the wedge this budget exists to break.
            warn!(task_id = %task_id, error = %e, "reconcile: failed to build provision request");
            return record_reprovision_failure(
                events,
                reconcile,
                task_id,
                reprovision_attempts,
                &e.to_string(),
            )
            .await;
        }
    };

    info!(task_id = %task_id, "reconcile: re-provisioning Deployment that vanished for an in-flight turn");
    match sandbox.provision(task_id, &request).await {
        Ok(_) => {
            reprovision_attempts.remove(task_id);
            metrics::counter!(
                "task_reconcile_remediations_total",
                "type" => "reprovision",
                "outcome" => "success",
            )
            .increment(1);
            info!(task_id = %task_id, "reconcile: re-provision succeeded");
            false
        }
        Err(e) => {
            record_reprovision_failure(
                events,
                reconcile,
                task_id,
                reprovision_attempts,
                &e.to_string(),
            )
            .await
        }
    }
}

/// Record one failed re-provision attempt against the per-task budget. Emits the
/// `failure` metric, and — when the attempt exhausts the budget — the `exhausted`
/// metric plus an `environment_error` event so the orphaned-turn closer can
/// finalise the turn next cycle. Returns `true` iff this attempt exhausted the
/// budget. Shared by both failure modes (un-buildable request and `provision()`
/// error) so neither can loop unbounded.
async fn record_reprovision_failure(
    events: &Option<Arc<dyn EventPublisher>>,
    reconcile: &ReconcileConfig,
    task_id: &str,
    reprovision_attempts: &mut HashMap<String, u32>,
    error: &str,
) -> bool {
    let attempts = *reprovision_attempts
        .entry(task_id.to_string())
        .and_modify(|c| *c += 1)
        .or_insert(1);
    metrics::counter!(
        "task_reconcile_remediations_total",
        "type" => "reprovision",
        "outcome" => "failure",
    )
    .increment(1);
    warn!(
        task_id = %task_id,
        attempts,
        max = reconcile.max_reprovision_attempts,
        error,
        "reconcile: re-provision attempt failed"
    );

    if attempts >= reconcile.max_reprovision_attempts {
        metrics::counter!(
            "task_reconcile_remediations_total",
            "type" => "reprovision",
            "outcome" => "exhausted",
        )
        .increment(1);
        if let Some(events) = events {
            events
                .publish_sandbox_event_with_payload(
                    task_id,
                    SandboxSubType::Error,
                    &SandboxEventPayload {
                        message: format!(
                            "Failed to re-provision the sandbox after {attempts} attempts"
                        ),
                        lifecycle_event: Some("environment_error".to_string()),
                        error: Some(error.to_string()),
                        initiator: Some("reconcile".to_string()),
                        ..Default::default()
                    },
                )
                .await;
        }
        return true;
    }
    false
}

/// Scale a stuck-Pending pod's Deployment to 0 and surface an environment error
/// carrying the scheduling reason. Best-effort.
async fn scale_stuck_pending_pod(
    k8s: &K8sClient,
    events: &Option<Arc<dyn EventPublisher>>,
    pod: &PendingPodInfo,
) {
    warn!(
        task_id = %pod.task_id,
        pod_name = %pod.pod_name,
        age_secs = pod.age_secs,
        reason = ?pod.reason,
        "reconcile: pod stuck Pending past hard deadline — scaling Deployment to 0"
    );

    match crate::sandbox::k8s::deployment::scale_deployment(k8s, &pod.task_id, 0).await {
        Ok(()) => {
            metrics::counter!(
                "task_reconcile_remediations_total",
                "type" => "pending_scale_zero",
                "outcome" => "success",
            )
            .increment(1);
            if let Some(events) = events {
                let reason = pod.reason.clone();
                // A `PodScheduled=False` reason means the pod never got a node —
                // genuine scheduling failure. No reason means it IS scheduled but
                // stuck later in Pending: image pull or volume attach (e.g. a
                // Multi-Attach stall) not completing. Don't conflate the two, or
                // operators chase the wrong cause.
                let message = reason.clone().map_or_else(
                    || {
                        format!(
                            "Sandbox scaled down after sitting Pending for {}s after scheduling (image pull or volume attach not completing)",
                            pod.age_secs
                        )
                    },
                    |r| format!("Sandbox scaled down: scheduling timed out ({r})"),
                );
                events
                    .publish_sandbox_event_with_payload(
                        &pod.task_id,
                        SandboxSubType::Error,
                        &SandboxEventPayload {
                            message,
                            lifecycle_event: Some("environment_error".to_string()),
                            error: Some(reason.unwrap_or_else(|| {
                                format!("Pod stuck Pending for {}s", pod.age_secs)
                            })),
                            initiator: Some("reconcile".to_string()),
                            pending_seconds: Some(pod.age_secs),
                            ..Default::default()
                        },
                    )
                    .await;
            }
        }
        Err(e) => {
            metrics::counter!(
                "task_reconcile_remediations_total",
                "type" => "pending_scale_zero",
                "outcome" => "failure",
            )
            .increment(1);
            warn!(task_id = %pod.task_id, error = %e, "reconcile: failed to scale down stuck-Pending Deployment");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(seconds_ago: i64) -> DateTime<Utc> {
        Utc::now() - chrono::Duration::seconds(seconds_ago)
    }

    // =========================================================================
    // busy_extension_active
    // =========================================================================

    #[test]
    fn busy_extension_holds_within_cap_and_expires_after() {
        let now = Utc::now();
        // Floor 1h ago, 4h cap — still within the extension window.
        assert!(busy_extension_active(Some(ts(3600)), now, 4));
        // Floor 5h ago, 4h cap — a leaked daemon no longer pins the sandbox.
        assert!(!busy_extension_active(Some(ts(5 * 3600)), now, 4));
    }

    #[test]
    fn busy_extension_disabled_or_unanchored() {
        let now = Utc::now();
        // max_hours == 0 disables the extension entirely.
        assert!(!busy_extension_active(Some(ts(60)), now, 0));
        // No activity floor — nothing to anchor the cap to.
        assert!(!busy_extension_active(None, now, 4));
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

    // =========================================================================
    // decide_orphan (orphan-GC pass)
    // =========================================================================

    fn retention_24h() -> chrono::Duration {
        chrono::Duration::hours(24)
    }

    #[test]
    fn orphan_deleted_task_is_gc_immediately() {
        // Task row gone from the DB → resources are pure orphans, GC regardless
        // of any window.
        let now = Utc::now();
        assert_eq!(
            decide_orphan(None, retention_24h(), now),
            OrphanDecision::Delete
        );
    }

    #[test]
    fn orphan_open_task_is_never_gc() {
        let now = Utc::now();
        // Even with a very old updated_at, an Open task is live → keep.
        let old = now - chrono::Duration::days(30);
        assert_eq!(
            decide_orphan(Some((TaskStatus::Open, Some(old))), retention_24h(), now),
            OrphanDecision::Keep
        );
    }

    #[test]
    fn orphan_error_task_is_never_gc() {
        let now = Utc::now();
        let old = now - chrono::Duration::days(30);
        assert_eq!(
            decide_orphan(Some((TaskStatus::Error, Some(old))), retention_24h(), now),
            OrphanDecision::Keep
        );
    }

    #[test]
    fn orphan_closed_task_within_retention_is_kept() {
        let now = Utc::now();
        // Closed 1h ago, 24h window → keep.
        let recent = now - chrono::Duration::hours(1);
        assert_eq!(
            decide_orphan(
                Some((TaskStatus::Closed, Some(recent))),
                retention_24h(),
                now
            ),
            OrphanDecision::Keep
        );
    }

    #[test]
    fn orphan_closed_task_past_retention_is_gc() {
        let now = Utc::now();
        // Closed 25h ago, 24h window → delete.
        let old = now - chrono::Duration::hours(25);
        assert_eq!(
            decide_orphan(Some((TaskStatus::Closed, Some(old))), retention_24h(), now),
            OrphanDecision::Delete
        );
    }

    #[test]
    fn orphan_closed_task_missing_updated_at_is_kept() {
        // No updated_at → treat conservatively as recently changed → keep.
        let now = Utc::now();
        assert_eq!(
            decide_orphan(Some((TaskStatus::Closed, None)), retention_24h(), now),
            OrphanDecision::Keep
        );
    }

    #[test]
    fn collect_ids_from_meta_dedupes_and_skips_unlabeled() {
        let mut ids = HashSet::new();
        let labeled = |task_id: &str| ObjectMeta {
            labels: Some(
                [("moatless.io/task-id".to_string(), task_id.to_string())]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        };
        let unlabeled = ObjectMeta::default();
        let metas = vec![labeled("a"), labeled("a"), labeled("b"), unlabeled];
        collect_ids_from_meta(&mut ids, metas.iter());
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("a"));
        assert!(ids.contains("b"));
    }

    // =========================================================================
    // decide_deployment_reconcile (Phase 4)
    // =========================================================================

    /// A live task mid-turn with a healthy Deployment — nothing to reconcile.
    fn healthy_input() -> DeploymentReconcileInput {
        DeploymentReconcileInput {
            reprovision_enabled: true,
            deployment_present: true,
            task_present: true,
            task_open: true,
            has_active_turn: true,
            recent_explicit_stop: false,
            reprovision_attempts: 0,
            max_reprovision_attempts: 3,
        }
    }

    #[test]
    fn reconcile_healthy_task_is_noop() {
        assert_eq!(
            decide_deployment_reconcile(&healthy_input()),
            ReconcileAction::None
        );
    }

    #[test]
    fn reconcile_missing_deployment_midturn_reprovisions() {
        let input = DeploymentReconcileInput {
            deployment_present: false,
            ..healthy_input()
        };
        assert_eq!(
            decide_deployment_reconcile(&input),
            ReconcileAction::Reprovision
        );
    }

    #[test]
    fn reconcile_missing_deployment_without_turn_is_noop() {
        // No in-flight turn ⇒ an idle sandbox that was legitimately removed; do
        // not resurrect it.
        let input = DeploymentReconcileInput {
            deployment_present: false,
            has_active_turn: false,
            ..healthy_input()
        };
        assert_eq!(decide_deployment_reconcile(&input), ReconcileAction::None);
    }

    #[test]
    fn reconcile_never_fights_recent_explicit_stop() {
        let input = DeploymentReconcileInput {
            deployment_present: false,
            recent_explicit_stop: true,
            ..healthy_input()
        };
        assert_eq!(decide_deployment_reconcile(&input), ReconcileAction::None);
    }

    #[test]
    fn reconcile_reprovision_respects_attempt_budget() {
        let input = DeploymentReconcileInput {
            deployment_present: false,
            reprovision_attempts: 3,
            max_reprovision_attempts: 3,
            ..healthy_input()
        };
        assert_eq!(decide_deployment_reconcile(&input), ReconcileAction::None);
    }

    #[test]
    fn reconcile_reprovision_disabled_is_noop() {
        let input = DeploymentReconcileInput {
            reprovision_enabled: false,
            deployment_present: false,
            ..healthy_input()
        };
        assert_eq!(decide_deployment_reconcile(&input), ReconcileAction::None);
    }

    #[test]
    fn reconcile_present_deployment_is_never_reprovisioned() {
        // Deployment present (even scaled to 0 while idle) for a task with no
        // turn — the idle-stop steady state. Orphan cleanup is out of scope here
        // (handled by gc_orphaned_resources), so this stays a no-op.
        let input = DeploymentReconcileInput {
            has_active_turn: false,
            ..healthy_input()
        };
        assert_eq!(decide_deployment_reconcile(&input), ReconcileAction::None);
    }

    #[test]
    fn reconcile_closed_task_missing_deployment_is_noop() {
        // A Closed task whose Deployment vanished is not re-provisioned — only
        // live Open tasks mid-turn are.
        let input = DeploymentReconcileInput {
            deployment_present: false,
            task_open: false,
            ..healthy_input()
        };
        assert_eq!(decide_deployment_reconcile(&input), ReconcileAction::None);
    }

    // =========================================================================
    // should_scale_pending_pod (Phase 4)
    // =========================================================================

    #[test]
    fn pending_pod_past_deadline_scales() {
        assert!(should_scale_pending_pod(true, 16 * 60, 15 * 60));
    }

    #[test]
    fn pending_pod_within_deadline_waits() {
        assert!(!should_scale_pending_pod(true, 5 * 60, 15 * 60));
    }

    #[test]
    fn pending_pod_disabled_never_scales() {
        assert!(!should_scale_pending_pod(false, 60 * 60, 15 * 60));
    }

    #[test]
    fn pending_pod_zero_deadline_never_scales() {
        assert!(!should_scale_pending_pod(true, 60 * 60, 0));
    }

    #[test]
    fn pending_pod_exactly_at_deadline_scales() {
        assert!(should_scale_pending_pod(true, 15 * 60, 15 * 60));
    }
}
