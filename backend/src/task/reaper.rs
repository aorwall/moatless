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

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Service};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, ListParams};
use moatless_events::{TaskSubType, TaskTurnSubType};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::dao::{TaskDao, TaskPatch};
use super::models::{SandboxDesiredState, SandboxStopReason, TaskStatus};
use crate::bindings::dao::TaskBindingDao;
use crate::config::Config;
use crate::event_handler::agent_handlers;
use crate::event_handler::models::AgentEvent;
use crate::events::EventPublisher;
use crate::message::dao::MessageDao;
use crate::sandbox::k8s::client::K8sClient;
use crate::sandbox::k8s::leader_election::LeaderElection;
use crate::sandbox::lifecycle::{ArchiveRetention, Lifecycle};
use crate::state::AppState;
use crate::task_event::dao::TaskEventDao;
use crate::task_turn::dao::TaskTurnDao;
use crate::task_turn::models::{TurnStatus, TurnTransition, UnclosedTurn};
use moatless_backend_core::scope::ScopeContext;
use moatless_events::EventType;

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

/// The unified task reaper's ports — one background job reads all of them, so
/// the struct is the explicit manifest of what a reap cycle touches.
///
/// `app_state` is retained for a single un-decomposed downstream: the
/// orphaned-turn error path forwards the whole state to
/// [`agent_handlers::handle_agent_error`], which is not yet split into ports.
/// Every other read goes through a named field; that one call is the sole
/// coupling that keeps the reaper on the composition-root baseline until
/// agent_handlers is decomposed.
pub struct TaskReaper {
    agent_status: Arc<dyn AgentStatusPort>,
    k8s: K8sClient,
    messages: Arc<dyn MessageDao>,
    tasks: Arc<dyn TaskDao>,
    task_turns: Arc<dyn TaskTurnDao>,
    task_events: Arc<dyn TaskEventDao>,
    sandbox: Arc<Lifecycle>,
    task_bindings: Arc<dyn TaskBindingDao>,
    events: Option<Arc<dyn EventPublisher>>,
    config: Arc<Config>,
    app_state: AppState,
}

impl TaskReaper {
    /// Assemble the reaper at the composition root. Its ports are the state's;
    /// `app_state` is kept only for the [`agent_handlers::handle_agent_error`]
    /// leak (see the struct doc).
    pub fn new(
        app_state: AppState,
        agent_status: Arc<dyn AgentStatusPort>,
        k8s: K8sClient,
    ) -> Self {
        Self {
            agent_status,
            k8s,
            messages: app_state.messages.clone(),
            tasks: app_state.tasks.clone(),
            task_turns: app_state.task_turns.clone(),
            task_events: app_state.task_events.clone(),
            sandbox: app_state.sandbox.clone(),
            task_bindings: app_state.task_bindings.clone(),
            events: app_state.events.clone(),
            config: app_state.config.clone(),
            app_state,
        }
    }

    /// Spawn the unified task reaper as a background tokio task.
    ///
    /// The reaper runs in a loop:
    /// 1. Try to acquire leadership via K8s Lease (`"moatless-task-reaper"`).
    /// 2. If leader, run a reap cycle (idle-sandbox stop + inactive-task close).
    /// 3. Sleep for `config.sandbox.idle_check_interval_secs` before the next cycle.
    pub fn spawn(self, cancel: CancellationToken) -> JoinHandle<()> {
        metrics::describe_counter!(
            "task_turns_reconciled_total",
            "Active task turns force-closed by the reaper after a lost terminal event"
        );
        let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
        let interval = Duration::from_secs(self.config.sandbox.idle_check_interval_secs);
        let default_ttl_minutes = self.config.sandbox.idle_timeout_minutes as i32;
        let default_inactive_ttl_days = self.config.sandbox.inactive_task_timeout_days as i32;
        let orphan_gc_enabled = self.config.sandbox.orphan_gc_enabled;
        // Lease duration must exceed the check interval so the lease doesn't
        // expire between cycles. Use 2x the interval as buffer.
        let lease_duration_secs = (self.config.sandbox.idle_check_interval_secs * 2) as i32;

        tokio::spawn(async move {
            let leader_election = LeaderElection::new_with_name(
                &self.k8s,
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
                        if let Err(e) = self
                            .run_reap_cycle(
                                default_ttl_minutes,
                                default_inactive_ttl_days,
                                orphan_gc_enabled,
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
    async fn run_reap_cycle(
        &self,
        default_ttl_minutes: i32,
        default_inactive_ttl_days: i32,
        orphan_gc_enabled: bool,
    ) -> Result<(), ReaperError> {
        // One cluster-wide Deployments LIST supplies the running Sandbox set.
        let deployments = list_sandbox_deployments(&self.k8s).await?;
        let sandbox_ids: Vec<String> = deployments
            .iter()
            .filter(|d| d.desired_replicas > 0)
            .map(|d| d.task_id.clone())
            .collect();

        // Tasks whose latest turn is stuck `active` past the floor.
        let cutoff = Utc::now() - chrono::Duration::seconds(RECONCILE_STUCK_TURN_THRESHOLD_SECS);
        let stuck_turns = match self.task_turns.list_unclosed_active_turns(cutoff).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "reconcile: failed to list unclosed active turns");
                Vec::new()
            }
        };

        // Close turns left stuck `active` by a lost terminal event.
        // Independent of the idle/close passes below and best-effort, so a
        // failure here never blocks them.
        self.reconcile_orphaned_turns(&sandbox_ids, &stuck_turns)
            .await;

        // Orphan-GC deletes only taskless resources. Desired Removed convergence
        // for existing Tasks belongs to the Sandbox reconciler.
        if orphan_gc_enabled {
            gc_orphaned_resources(&self.k8s, &self.tasks).await;
        }

        let Some(snapshot) = self
            .load_reaper_snapshot(&sandbox_ids, default_ttl_minutes, default_inactive_ttl_days)
            .await?
        else {
            return Ok(());
        };

        let stats = self.apply_reaper_decisions(snapshot).await;
        log_reaper_stats(stats);

        Ok(())
    }
}

struct ReaperSnapshot {
    all_ids: Vec<String>,
    sandbox_ids: Vec<String>,
    open_ids: Vec<String>,
    latest_times: HashMap<String, DateTime<Utc>>,
    ttl_map: HashMap<String, i32>,
    inactive_map: HashMap<String, i32>,
    agent_lifecycle: HashMap<String, (String, DateTime<Utc>)>,
    default_ttl_minutes: i32,
    default_inactive_ttl_days: i32,
    now: DateTime<Utc>,
}

#[derive(Clone, Copy)]
struct ReaperStats {
    stopped_count: u32,
    closed_count: u32,
}

// The reaper's methods are split across several `impl` blocks so each stays next
// to the free helpers between them that it calls (the decision fns, the DAO-thin
// loaders, the agent-status port). The blocks collapse into one if those helpers
// ever become methods.
impl TaskReaper {
    async fn load_reaper_snapshot(
        &self,
        sandbox_ids: &[String],
        default_ttl_minutes: i32,
        default_inactive_ttl_days: i32,
    ) -> Result<Option<ReaperSnapshot>, ReaperError> {
        let open_ids = match self.tasks.list_open_task_ids().await {
            Ok(ids) => ids,
            Err(e) => {
                warn!(error = %e, "Failed to list open tasks, skipping inactive-task close");
                Vec::new()
            }
        };

        // Union the two sets so we fetch activity timestamps for everything we may act on.
        let mut all_ids_set: HashSet<String> =
            HashSet::with_capacity(sandbox_ids.len() + open_ids.len());
        all_ids_set.extend(sandbox_ids.iter().cloned());
        all_ids_set.extend(open_ids.iter().cloned());

        if all_ids_set.is_empty() {
            debug!("No running sandboxes and no open tasks");
            return Ok(None);
        }

        let all_ids: Vec<String> = all_ids_set.iter().cloned().collect();
        debug!(
            sandbox_count = sandbox_ids.len(),
            open_count = open_ids.len(),
            unique_count = all_ids.len(),
            "Task reaper considering tasks"
        );

        let latest_times = self.load_latest_activity_times(&all_ids).await?;
        let ttl_map = load_ttl_map(&self.tasks, &all_ids).await;
        let inactive_map = load_inactive_ttl_map(&self.tasks, &all_ids).await;
        let agent_lifecycle = self.load_agent_lifecycle(sandbox_ids).await;

        Ok(Some(ReaperSnapshot {
            all_ids,
            sandbox_ids: sandbox_ids.to_vec(),
            open_ids,
            latest_times,
            ttl_map,
            inactive_map,
            agent_lifecycle,
            default_ttl_minutes,
            default_inactive_ttl_days,
            now: Utc::now(),
        }))
    }

    async fn load_latest_activity_times(
        &self,
        all_ids: &[String],
    ) -> Result<HashMap<String, DateTime<Utc>>, ReaperError> {
        let mut latest_times = self.messages.get_latest_message_times(all_ids).await?;

        // A single long turn emits no new conversation message and no lifecycle
        // transition, so agent activity keeps the idle floor fresh during tool calls.
        match self.task_events.get_latest_agent_activity(all_ids).await {
            Ok(activity_times) => merge_newest(&mut latest_times, activity_times),
            Err(e) => {
                warn!(error = %e, "Failed to fetch agent activity times, idle floor omits tool-call activity");
            }
        }

        match self.tasks.get_sandbox_started_times(all_ids).await {
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
            match self.tasks.get_created_times(&no_activity_ids).await {
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

        Ok(latest_times)
    }
}

fn merge_newest(
    latest_times: &mut HashMap<String, DateTime<Utc>>,
    entries: HashMap<String, DateTime<Utc>>,
) {
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
}

async fn load_ttl_map(tasks: &Arc<dyn TaskDao>, all_ids: &[String]) -> HashMap<String, i32> {
    tasks.get_ttl_minutes_map(all_ids).await.unwrap_or_else(|e| {
        warn!(error = %e, "Failed to fetch ttl_minutes map, falling back to default for all tasks");
        HashMap::new()
    })
}

async fn load_inactive_ttl_map(
    tasks: &Arc<dyn TaskDao>,
    all_ids: &[String],
) -> HashMap<String, i32> {
    tasks
        .get_inactive_ttl_days_map(all_ids)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "Failed to fetch inactive_ttl_days map, falling back to default for all tasks");
            HashMap::new()
        })
}

impl TaskReaper {
    async fn load_agent_lifecycle(
        &self,
        sandbox_ids: &[String],
    ) -> HashMap<String, (String, DateTime<Utc>)> {
        // Latest agent run-state per sandbox task, used to date the idle-stop floor
        // from when the agent last stopped (and to never reap a running agent).
        // Only sandbox tasks reach the reap branch, so query just those. Falls back
        // to the activity floor on error.
        self.task_events
        .get_latest_agent_lifecycle(sandbox_ids)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "Failed to fetch agent lifecycle events, reaper falls back to activity floor");
            HashMap::new()
        })
    }

    async fn apply_reaper_decisions(&self, snapshot: ReaperSnapshot) -> ReaperStats {
        let sandbox_set: HashSet<&str> = snapshot.sandbox_ids.iter().map(String::as_str).collect();
        let open_set: HashSet<&str> = snapshot.open_ids.iter().map(String::as_str).collect();

        let mut stats = ReaperStats {
            stopped_count: 0,
            closed_count: 0,
        };

        for task_id in &snapshot.all_ids {
            let latest_at = snapshot.latest_times.get(task_id).copied();
            let inactive_ttl_days = snapshot
                .inactive_map
                .get(task_id)
                .copied()
                .unwrap_or(snapshot.default_inactive_ttl_days);

            // Close supersedes stop — `auto_close_task` calls `cleanup`, which
            // tears down the Deployment regardless of whether it was scaled to 0.
            if open_set.contains(task_id.as_str())
                && decide_close(inactive_ttl_days, latest_at, snapshot.now) == CloseDecision::Close
            {
                info!(
                    task_id = %task_id,
                    inactive_ttl_days,
                    last_activity = ?latest_at,
                    "Auto-closing inactive task"
                );
                self.auto_close_task(task_id).await;
                stats.closed_count += 1;
                continue;
            }

            if !sandbox_set.contains(task_id.as_str()) {
                continue;
            }

            // Fold the agent run-state into the idle-stop floor so a running agent
            // is never reaped and a stopped agent's TTL counts from when it stopped.
            let reap_floor = compute_reap_floor(
                snapshot.agent_lifecycle.get(task_id),
                latest_at,
                snapshot.now,
            );

            let ttl_minutes = snapshot
                .ttl_map
                .get(task_id)
                .copied()
                .unwrap_or(snapshot.default_ttl_minutes);
            match decide_reap(ttl_minutes, reap_floor, snapshot.now) {
                ReapDecision::Skip(reason) => {
                    debug!(task_id = %task_id, reason, "Skipping sandbox reap");
                }
                ReapDecision::Keep => {}
                ReapDecision::Stop => {
                    if self
                        .stop_idle_sandbox(task_id, ttl_minutes, latest_at, reap_floor)
                        .await
                    {
                        stats.stopped_count += 1;
                    }
                }
            }
        }

        stats
    }

    async fn stop_idle_sandbox(
        &self,
        task_id: &str,
        ttl_minutes: i32,
        latest_at: Option<DateTime<Utc>>,
        reap_floor: Option<DateTime<Utc>>,
    ) -> bool {
        // Defense in depth: the activity floor can go stale, so ask the agent
        // itself before pulling the plug. An unreadable status falls through.
        if let Some(status) = agent_status_snapshot(self.agent_status.as_ref(), task_id).await {
            if agent_holds_the_sandbox(&status.status) {
                info!(
                    task_id = %task_id,
                    ttl_minutes,
                    last_activity = ?latest_at,
                    "Idle floor expired but agent reports running, skipping stop"
                );
                return false;
            }
            if status.busy_processes
                && busy_extension_active(
                    reap_floor,
                    Utc::now(),
                    self.config.sandbox.busy_extension_max_hours,
                )
            {
                info!(
                    task_id = %task_id,
                    ttl_minutes,
                    last_activity = ?latest_at,
                    "Idle floor expired but sandbox reports live background processes, deferring stop"
                );
                return false;
            }
        }

        info!(
            task_id = %task_id,
            ttl_minutes,
            last_activity = ?latest_at,
            "Stopping idle sandbox"
        );
        match self
            .sandbox
            .stop(task_id, SandboxStopReason::IdleReaper)
            .await
        {
            Ok(_) => {
                info!(task_id = %task_id, "Idle sandbox stopped successfully");
                true
            }
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Failed to stop idle sandbox");
                false
            }
        }
    }
}

/// Whether a live agent status is one the sandbox must be kept alive for.
///
/// `waiting` is not: the run is parked on a person and doing no work, so the
/// ordinary idle TTL decides. The question it is parked on is stored, and
/// answering it after the reclaim starts a fresh turn carrying it.
fn agent_holds_the_sandbox(status: &str) -> bool {
    !matches!(status, "idle" | "stopped" | "waiting")
}

fn log_reaper_stats(stats: ReaperStats) {
    if stats.stopped_count > 0 || stats.closed_count > 0 {
        info!(
            stopped_count = stats.stopped_count,
            closed_count = stats.closed_count,
            "Task reaper cycle complete"
        );
    } else {
        debug!("Task reaper cycle complete, nothing to do");
    }
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
impl TaskReaper {
    async fn reconcile_orphaned_turns(
        &self,
        running_sandbox_ids: &[String],
        stuck: &[UnclosedTurn],
    ) {
        if stuck.is_empty() {
            return;
        }

        let running: HashSet<&str> = running_sandbox_ids.iter().map(String::as_str).collect();

        for turn in stuck {
            if !self.task_desires_running(&turn.task_id).await {
                debug!(
                    task_id = %turn.task_id,
                    "reconcile: task does not desire a running sandbox, leaving turn for retry"
                );
                continue;
            }
            let orphaned = if running.contains(turn.task_id.as_str()) {
                // Sandbox is up — only orphaned if the agent has gone terminal.
                // Unknown status ⇒ conservative skip (treat as possibly live).
                matches!(
                    agent_is_terminal(self.agent_status.as_ref(), &turn.task_id).await,
                    Some(true)
                )
            } else {
                // No running Deployment ⇒ nothing can still be driving the turn.
                true
            };

            if !orphaned {
                debug!(task_id = %turn.task_id, "reconcile: turn still live, skipping");
                continue;
            }

            self.close_orphaned_turn(turn).await;
        }
    }

    async fn task_desires_running(&self, task_id: &str) -> bool {
        match self.sandbox.desired_state(task_id).await {
            Ok(desired) => desired == SandboxDesiredState::Running,
            Err(e) => {
                warn!(task_id, error = %e, "reconcile: desired-state lookup failed");
                false
            }
        }
    }
}

/// Returns `Some(true)` if the sandbox agent is `idle`/`stopped`, `Some(false)`
/// if it's still `running`, and `None` when the status can't be read (no client,
/// gRPC error) — the caller treats `None` conservatively as "possibly live".
async fn agent_is_terminal(agent_status: &dyn AgentStatusPort, task_id: &str) -> Option<bool> {
    agent_status_snapshot(agent_status, task_id)
        .await
        .map(|resp| agent_turn_is_over(&resp.status))
}

/// Whether a live agent status means the turn it was running has ended.
///
/// `waiting` does not: the run is parked mid-turn and continues where it left
/// off the moment the answer lands. That the sandbox under it may be reclaimed
/// in the meantime is a separate question — see [`agent_holds_the_sandbox`].
fn agent_turn_is_over(status: &str) -> bool {
    matches!(status, "idle" | "stopped")
}

/// Live agent status straight from the sandbox, `None` when unreachable
/// (no client, gRPC error).
async fn agent_status_snapshot(
    agent_status: &dyn AgentStatusPort,
    task_id: &str,
) -> Option<sandbox_proto::GetAgentStatusResponse> {
    agent_status.agent_status(task_id).await
}

/// The reaper's read of a Sandbox's live agent status — the one interior read it
/// needs, declared consumer-side so the provider handle stays off `AppState`.
#[async_trait]
pub trait AgentStatusPort: Send + Sync {
    async fn agent_status(&self, task_id: &str) -> Option<sandbox_proto::GetAgentStatusResponse>;
}

/// Satisfies [`AgentStatusPort`] through the task-addressed connector.
pub struct ConnectorAgentStatus {
    connector: Arc<dyn moatless_backend_core::sandbox::SandboxConnector>,
}

impl ConnectorAgentStatus {
    pub fn new(connector: Arc<dyn moatless_backend_core::sandbox::SandboxConnector>) -> Self {
        Self { connector }
    }
}

#[async_trait]
impl AgentStatusPort for ConnectorAgentStatus {
    async fn agent_status(&self, task_id: &str) -> Option<sandbox_proto::GetAgentStatusResponse> {
        let client = match self.connector.sandbox_client(task_id).await {
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
impl TaskReaper {
    async fn close_orphaned_turn(&self, turn: &UnclosedTurn) {
        // False-negative guard: if the agent already delivered its final reply for
        // this turn, the terminal event was merely lost — not the result. Closing
        // such a turn as `error` surfaces a misleading "stopped without reporting a
        // result" message even though the answer is right there in the chat. Close
        // it as `completed` instead.
        match self
            .messages
            .last_message_is_completed_reply(&turn.task_id, turn.turn_number)
            .await
        {
            Ok(true) => {
                self.close_orphaned_turn_as_completed(turn).await;
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

        agent_handlers::handle_agent_error(&self.app_state, &event).await;
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
    async fn close_orphaned_turn_as_completed(&self, turn: &UnclosedTurn) {
        if let Some(turn_number) = turn.turn_number {
            if let Err(e) = self
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
        // reason: reaper reconciles orphaned turns across all task owners; system ctx required.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        if let Err(e) = self
            .tasks
            .update_status_if_not_closed(&turn.task_id, TaskStatus::Open, &sys_ctx)
            .await
        {
            warn!(
                task_id = %turn.task_id,
                error = %e,
                "reconcile: failed to reopen task after completed reconcile"
            );
        }

        if let Some(ref events) = self.events {
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

impl TaskReaper {
    /// Mirror of [`crate::task::handlers::close_task`] for unattended use.
    ///
    /// Best-effort: every step is logged on failure but never propagates — the
    /// reaper loop must never panic.
    ///
    /// Not [`crate::task::closer::TaskCloser::close_single`]: that path carries an
    /// already-Closed idempotence guard this one does not, so they are not yet
    /// interchangeable.
    async fn auto_close_task(&self, task_id: &str) {
        // reason: reaper acts across all task owners; system ctx bypasses per-user scope.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();

        let agent_type = match self.tasks.find_by_id(task_id, &sys_ctx).await {
            Ok(Some(row)) => Some(row.agent_type),
            Ok(None) => {
                warn!(task_id, "auto-close: task vanished mid-cycle");
                return;
            }
            Err(e) => {
                warn!(task_id, error = %e, "auto-close: find_by_id failed");
                return;
            }
        };

        // Commit `Closed` BEFORE the teardown (the original order cleaned up first,
        // then wrote the status). With `Closed` persisted first, any concurrent
        // `auto_provision` / `get_pending_messages` (gated by Fix B) sees `Closed`
        // and bails. A teardown failure below can't leave the task non-terminal.
        if let Err(e) = self
            .tasks
            .update_task(
                task_id,
                &TaskPatch {
                    status: Some(TaskStatus::Closed),
                    ..Default::default()
                },
                &sys_ctx,
            )
            .await
        {
            warn!(task_id, error = %e, "auto-close: update_task failed");
            return;
        }

        // Remove the sandbox (best-effort; `Closed` already persisted). Lifecycle
        // issues the graceful agent stop and retains S3 archives per config so an
        // auto-closed task stays revivable.
        let retention = ArchiveRetention::on_close(self.config.sandbox.retain_storage_on_close);
        if let Err(e) = self.sandbox.remove(task_id, retention).await {
            warn!(task_id, error = %e, "auto-close: sandbox remove failed (continuing)");
        }

        // A closed task is terminal — deactivate its bindings so inbound comments
        // don't resurrect it. Best-effort; the runtime guards are the backstop.
        if let Err(e) = self.task_bindings.deactivate_by_task(task_id).await {
            warn!(task_id, error = %e, "auto-close: failed to deactivate bindings");
        }

        if let (Some(events), Some(at)) = (&self.events, agent_type) {
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
}

/// Outcome of evaluating a labeled sandbox resource for orphan GC.
#[derive(Debug, PartialEq, Eq)]
enum OrphanDecision {
    /// The resource's Task row is gone, so lifecycle cannot express removal.
    Delete,
    /// An existing Task's desired-state convergence belongs to lifecycle.
    Keep,
}

/// Pure decision function: decide whether a task's leftover sandbox resources
/// are orphans safe to GC.
fn decide_orphan(task_exists: bool) -> OrphanDecision {
    if task_exists {
        OrphanDecision::Keep
    } else {
        OrphanDecision::Delete
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
    use crate::sandbox::k8s::{deployment, ingress, networkpolicy, pvc, service};

    if let Err(e) = deployment::delete_deployment(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete Deployment");
    }
    if let Err(e) = service::delete_sandbox_service(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete Service");
    }
    if let Err(e) = ingress::delete_task_ingress_routes(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete IngressRoutes");
    }
    if let Err(e) = networkpolicy::delete_task_preview_network_policy(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete preview NetworkPolicy");
    }
    if let Err(e) = pvc::delete_task_pvc(k8s, task_id).await {
        warn!(task_id = %task_id, error = %e, "orphan-gc: failed to delete PVC");
    }
}

/// Orphan-GC pass: LIST labeled sandbox resources, then delete only resources
/// whose Task row no longer exists. Fully best-effort.
async fn gc_orphaned_resources(k8s: &K8sClient, tasks: &Arc<dyn TaskDao>) {
    let task_ids = collect_labeled_task_ids(k8s).await;
    if task_ids.is_empty() {
        return;
    }

    // reason: orphan GC checks task existence across all owners; system ctx required.
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    let mut gc_count = 0u32;
    for task_id in task_ids {
        let task_exists = match tasks.find_by_id(&task_id, &sys_ctx).await {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(e) => {
                // A transient DB error must not be mistaken for a deleted task —
                // skip this task-id this cycle rather than risk deleting live
                // resources.
                warn!(task_id = %task_id, error = %e, "orphan-gc: task lookup failed, skipping");
                continue;
            }
        };

        if decide_orphan(task_exists) == OrphanDecision::Delete {
            info!(
                task_id = %task_id,
                "orphan-gc: deleting taskless sandbox resources"
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
/// and desired replicas via a single cluster-wide LIST.
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

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingAgentStatus {
        calls: std::sync::Mutex<Vec<String>>,
        response: sandbox_proto::GetAgentStatusResponse,
    }

    #[async_trait]
    impl AgentStatusPort for RecordingAgentStatus {
        async fn agent_status(
            &self,
            task_id: &str,
        ) -> Option<sandbox_proto::GetAgentStatusResponse> {
            self.calls.lock().unwrap().push(task_id.to_string());
            Some(self.response.clone())
        }
    }

    #[tokio::test]
    async fn idle_eligibility_reads_agent_status_through_its_port() {
        let port = RecordingAgentStatus {
            calls: std::sync::Mutex::new(Vec::new()),
            response: sandbox_proto::GetAgentStatusResponse {
                status: "idle".to_string(),
                ..Default::default()
            },
        };

        let status = agent_status_snapshot(&port, "task-1")
            .await
            .expect("recording port returns status");

        assert_eq!(status.status, "idle");
        assert_eq!(*port.calls.lock().unwrap(), vec!["task-1"]);
    }

    /// A parked run is doing no work, so it does not hold its sandbox open past
    /// the ordinary TTL. Everything the agent is actually running does.
    #[test]
    fn a_waiting_agent_does_not_hold_its_sandbox() {
        assert!(!agent_holds_the_sandbox("waiting"));
        assert!(!agent_holds_the_sandbox("idle"));
        assert!(!agent_holds_the_sandbox("stopped"));
        assert!(agent_holds_the_sandbox("running"));
        // A status this build does not know is treated as live.
        assert!(agent_holds_the_sandbox("from-a-newer-sandbox"));
    }

    /// A turn parked on a person is not a finished turn: it continues where it
    /// left off the moment the answer lands, so the reconciler leaves its turn
    /// open even though the reaper may reclaim the sandbox under it.
    #[test]
    fn a_waiting_agents_turn_is_not_over() {
        assert!(!agent_turn_is_over("waiting"));
        assert!(agent_turn_is_over("idle"));
        assert!(agent_turn_is_over("stopped"));
        assert!(!agent_turn_is_over("running"));
    }

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
        let decision = decide_close(7, Some(now - chrono::Duration::days(8)), now);
        assert_eq!(decision, CloseDecision::Close);
    }

    #[test]
    fn close_within_ttl_keeps() {
        let now = Utc::now();
        // 7-day TTL, last activity 1 day ago → keep.
        let decision = decide_close(7, Some(now - chrono::Duration::days(1)), now);
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

    #[test]
    fn orphan_deleted_task_is_gc_immediately() {
        assert_eq!(decide_orphan(false), OrphanDecision::Delete);
    }

    #[test]
    fn orphan_open_task_is_never_gc() {
        assert_eq!(decide_orphan(true), OrphanDecision::Keep);
    }

    #[test]
    fn existing_closed_task_is_left_to_lifecycle() {
        assert_eq!(decide_orphan(true), OrphanDecision::Keep);
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
        let metas = [labeled("a"), labeled("a"), labeled("b"), unlabeled];
        collect_ids_from_meta(&mut ids, metas.iter());
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("a"));
        assert!(ids.contains("b"));
    }
}
