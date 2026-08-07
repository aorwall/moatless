//! Unified task reaper.
//!
//! Background job that periodically:
//! 1. Stops idle sandbox Deployments (short TTL — `ttl_minutes`).
//! 2. Auto-closes inactive Open tasks (long TTL — `inactive_ttl_days`).
//!
//! Runs on the elected leader only (through a `LeaderGuard` port) so a single
//! replica drives the cycle. Replaces the legacy `sandbox::idle_reaper` — both
//! behaviours share one cycle, one lease (`moatless-task-reaper`), and one
//! activity overlay.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_events::{TaskSubType, TaskTurnSubType};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::dao::{TaskDao, TaskPatch};
use super::models::{SandboxDesiredState, SandboxStopReason, TaskStatus};
use crate::agent_event::AgentEvent;
use crate::bindings::dao::TaskBindingDao;
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use moatless_core::leader::LeaderGuard;
use crate::message::dao::MessageDao;
use crate::sandbox::lifecycle::{ArchiveRetention, Lifecycle};
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
    #[error("sandbox query error: {0}")]
    SandboxQuery(Box<dyn std::error::Error + Send + Sync>),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// The unified task reaper's ports — one background job reads all of them, so
/// the struct is the explicit manifest of what a reap cycle touches. The
/// orphaned-turn error path delivers a synthetic `agent.error` through the
/// [`AgentErrorSink`] port rather than reaching into the server-band error
/// machinery, so the reaper names no server type.
pub struct TaskReaper {
    agent_status: Arc<dyn AgentStatusPort>,
    leader: Arc<dyn LeaderGuard>,
    messages: Arc<dyn MessageDao>,
    tasks: Arc<dyn TaskDao>,
    task_turns: Arc<dyn TaskTurnDao>,
    task_events: Arc<dyn TaskEventDao>,
    sandbox: Arc<Lifecycle>,
    task_bindings: Arc<dyn TaskBindingDao>,
    events: Option<Arc<dyn EventPublisher>>,
    config: Arc<Config>,
    agent_error_sink: Arc<dyn AgentErrorSink>,
    sandbox_resources: Arc<dyn SandboxResources>,
}

impl TaskReaper {
    /// Assemble the reaper at the composition root from its ports — each wired
    /// from the router-held `AppState` there, so this module names none.
    #[expect(
        clippy::too_many_arguments,
        reason = "the reaper's ports are its explicit per-cycle manifest; the composition root wires each one"
    )]
    pub fn new(
        agent_status: Arc<dyn AgentStatusPort>,
        leader: Arc<dyn LeaderGuard>,
        messages: Arc<dyn MessageDao>,
        tasks: Arc<dyn TaskDao>,
        task_turns: Arc<dyn TaskTurnDao>,
        task_events: Arc<dyn TaskEventDao>,
        sandbox: Arc<Lifecycle>,
        task_bindings: Arc<dyn TaskBindingDao>,
        events: Option<Arc<dyn EventPublisher>>,
        config: Arc<Config>,
        agent_error_sink: Arc<dyn AgentErrorSink>,
        sandbox_resources: Arc<dyn SandboxResources>,
    ) -> Self {
        Self {
            agent_status,
            leader,
            messages,
            tasks,
            task_turns,
            task_events,
            sandbox,
            task_bindings,
            events,
            config,
            agent_error_sink,
            sandbox_resources,
        }
    }

    /// Spawn the unified task reaper as a background tokio task.
    ///
    /// The reaper runs in a loop:
    /// 1. Try to acquire leadership through the `LeaderGuard` port.
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

        tokio::spawn(async move {
            loop {
                if cancel.is_cancelled() {
                    info!("Task reaper shutting down (cancellation)");
                    return;
                }

                match self.leader.try_acquire_or_renew().await {
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
        // The running Sandbox set for this cycle, via the k8s-native port. A
        // failed LIST aborts the cycle (`?`) rather than letting an empty set be
        // read as "nothing is running" and reaping live turns under it.
        let sandbox_ids = self
            .sandbox_resources
            .list_running_sandbox_ids()
            .await
            .map_err(ReaperError::SandboxQuery)?;

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
            self.sandbox_resources.gc_orphaned_resources().await;
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

/// The reaper's delivery of the synthetic `agent.error` it raises when it closes
/// an orphaned turn, declared consumer-side so the server-band error machinery
/// (`AppState` and the agent handlers it reaches) stays off the reaper.
#[async_trait]
pub trait AgentErrorSink: Send + Sync {
    async fn deliver_agent_error(&self, event: &AgentEvent);
}

/// The k8s resources backing sandboxes, as the reaper needs to touch them —
/// declared consumer-side so the whole Kubernetes body lives in
/// `sandbox::k8s::sandbox_resources` (beside the client) and the reaper names no
/// k8s type. Two operations the same cycle needs on the same client:
///
/// - [`list_running_sandbox_ids`](Self::list_running_sandbox_ids): the
///   authoritative running-Sandbox set (managed Deployments with
///   `desired_replicas > 0`). Fallible and propagated — a failed LIST aborts the
///   cycle rather than letting an empty set look like "nothing is running" and
///   reaping live turns.
/// - [`gc_orphaned_resources`](Self::gc_orphaned_resources): delete labeled
///   sandbox resources whose Task row is gone. Best-effort: failures are logged
///   inside the implementation, never surfaced.
#[async_trait]
pub trait SandboxResources: Send + Sync {
    async fn list_running_sandbox_ids(
        &self,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;

    async fn gc_orphaned_resources(&self);
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

/// Close a stuck turn: if the agent already delivered its final reply the
/// terminal event was merely lost, so close as `completed`; otherwise deliver a
/// synthetic `agent.error` through the sink.
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

        close_orphaned_turn_as_error(self.agent_error_sink.as_ref(), turn).await;
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

/// Force a stuck turn to `error` by delivering a synthetic `agent.error` through
/// the sink — the same close path a real `agent.error` takes: appends an error
/// result message, flips the task back to Open, and publishes `taskTurn/error`
/// so the UI unblocks.
///
/// The synthetic event id is deterministic per `(task, turn)` so repeated
/// reconcile cycles (e.g. while Redis is still down and the terminal event
/// hasn't yet round-tripped into `task_events`) upsert the same message row
/// instead of stacking duplicates.
///
/// Free so the reconcile delivery is testable through the [`AgentErrorSink`]
/// port without constructing a full reaper.
async fn close_orphaned_turn_as_error(sink: &dyn AgentErrorSink, turn: &UnclosedTurn) {
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

    sink.deliver_agent_error(&event).await;
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

    #[derive(Default)]
    struct RecordingAgentErrorSink {
        events: std::sync::Mutex<Vec<AgentEvent>>,
    }

    #[async_trait]
    impl AgentErrorSink for RecordingAgentErrorSink {
        async fn deliver_agent_error(&self, event: &AgentEvent) {
            self.events.lock().unwrap().push(AgentEvent {
                event_id: event.event_id.clone(),
                event_type: event.event_type,
                sub_type: event.sub_type.clone(),
                task_id: event.task_id.clone(),
                timestamp: event.timestamp.clone(),
                is_error: event.is_error,
                agent_type: event.agent_type.clone(),
                turn_number: event.turn_number,
                payload: event.payload.clone(),
            });
        }
    }

    /// Holds two things about [`close_orphaned_turn_as_error`]: the reconcile
    /// event's shape, and that the sink is fired exactly once. It does NOT cover
    /// the reaper's invocation of this helper from `close_orphaned_turn` —
    /// deleting that call leaves this green while orphaned turns silently stop
    /// closing. That edge is unguarded because `TaskReaper` has no integration
    /// harness (it takes a dozen collaborators, one of them `Lifecycle`); see the
    /// coverage gap recorded in the Step 9 plan outcome.
    #[tokio::test]
    async fn close_orphaned_turn_as_error_delivers_reconcile_event_to_sink() {
        let sink = RecordingAgentErrorSink::default();
        let turn = UnclosedTurn {
            task_id: "task-9".to_string(),
            turn_number: Some(4),
            agent_type: Some("claude".to_string()),
        };

        close_orphaned_turn_as_error(&sink, &turn).await;

        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 1, "the sink must be fired exactly once");
        let event = &events[0];
        assert_eq!(event.event_id, "reconcile-task-9-4");
        assert_eq!(event.event_type, EventType::Agent);
        assert_eq!(event.sub_type, "error");
        assert_eq!(event.task_id, "task-9");
        assert!(event.is_error);
        assert_eq!(event.agent_type.as_deref(), Some("claude"));
        assert_eq!(event.turn_number, Some(4));
        assert_eq!(
            event.payload.get("reason").and_then(|v| v.as_str()),
            Some("reconciled_orphaned_turn")
        );
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

}
