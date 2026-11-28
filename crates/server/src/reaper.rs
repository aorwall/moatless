//! Unified task reaper.
//!
//! Background job that periodically stops idle sandbox Deployments (short TTL
//! `ttl_minutes`), settles Open tasks idle past `settle_after_hours`, closes
//! them `close_after_settled_days` after they were settled, and reclaims the
//! Deployment/volume of a long-stopped sandbox (`cold_after_hours`), leaving
//! the S3 snapshot as the only copy; the task stays Open and a later start
//! restores a fresh volume. Runs on the
//! elected leader only (one lease, `moatless-task-reaper`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use moatless_types::task::{TaskSubType, TaskTurnSubType};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::reaper_decisions::{
    CloseDecision, ReapDecision, decide_close, decide_cold, decide_reap, decide_settle,
    fold_latest_activity, fold_reap_floor, fold_ttl_minutes, preview_holds_the_sandbox,
    silent_since,
};

use crate::event_handler::agent_handlers::AgentErrorPorts;
use moatless_core::config::Config;
use moatless_core::events::EventPublisher;
use moatless_core::leader::LeaderGuard;
use moatless_loops::bindings::operations::Bindings;
use moatless_ports::sandbox::SandboxLifecycle;
use moatless_tasks::agent_event::AgentEvent;
use moatless_tasks::message::operations::Messages;
use moatless_tasks::task::models::SandboxDesiredState;
use moatless_tasks::task::models::SandboxStopReason;
use moatless_tasks::task::models::TaskPatch;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::operations::TaskCommands;
use moatless_tasks::task_event::operations::TaskEvents;
use moatless_tasks::task_turn::models::SettledTurn;
use moatless_tasks::task_turn::models::TurnStatus;
use moatless_tasks::task_turn::models::TurnTransition;
use moatless_tasks::task_turn::models::UnclosedTurn;
use moatless_tasks::task_turn::operations::TaskTurns;
use moatless_types::access_control::ScopeContext;
use moatless_types::event::EventType;
use moatless_types::sandbox::ArchiveRetention;
use sandbox_proto::client::SandboxConnector;

/// Minimum age of a stuck `active` turn before the reaper reconciles it.
///
/// Set above normal turn latency, so only a turn whose terminal event was
/// genuinely lost (e.g. dropped during a database outage) gets force-closed.
const RECONCILE_STUCK_TURN_THRESHOLD_SECS: i64 = 5 * 60;

/// How long an orphaned turn stays `interrupted`, resumable by a replacement
/// pod's `recover_interrupted_run`, before the reaper closes it as `error`.
///
/// Covers a node loss: node-controller grace, pod GC, a new node, and the
/// workspace restore before the new sandbox asks to recover.
const ORPHANED_TURN_RECOVERY_WINDOW_SECS: i64 = 15 * 60;

/// How long a running agent may emit no `agent` event before the reaper logs
/// its turn as silent, which the Moatless Agent Turn Silent alert reads.
///
/// A healthy turn emits `context_window` on every model response, and a
/// foreground command times out within 10 minutes.
const SILENT_TURN_THRESHOLD_SECS: i64 = 30 * 60;

/// The deployment-wide knobs one cycle runs against. Grouped because they are
/// read once at spawn and threaded whole through the cycle; each is overridable
/// per Task by a column, and the per-Task value wins.
#[derive(Clone, Copy)]
struct ReapCycleDefaults {
    ttl_minutes: i32,
    /// `0` disables auto-settle for tasks that name no TTL of their own.
    settle_after_hours: i32,
    close_after_settled_days: i32,
    /// `0` disables the cold branch for the deployment.
    cold_after_hours: i32,
    orphan_gc_enabled: bool,
}

/// Errors that can occur during a reap cycle.
#[derive(Debug, thiserror::Error)]
enum ReaperError {
    #[error("sandbox query error: {0}")]
    SandboxQuery(Box<dyn std::error::Error + Send + Sync>),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// The unified task reaper's ports — the explicit manifest of what a reap cycle
/// touches.
///
/// `agent_error` delivers the synthetic `agent.error` that closes an orphaned
/// turn through the same handler a real one takes.
pub struct TaskReaper {
    connector: Arc<dyn SandboxConnector>,
    leader: Arc<dyn LeaderGuard>,
    messages: Arc<Messages>,
    tasks: Arc<TaskCommands>,
    task_turns: Arc<TaskTurns>,
    task_events: Arc<TaskEvents>,
    sandbox: Arc<dyn SandboxLifecycle>,
    task_bindings: Arc<Bindings>,
    events: Option<Arc<dyn EventPublisher>>,
    config: Arc<Config>,
    agent_error: AgentErrorPorts,
    k8s: moatless_sandbox_k8s::client::K8sClient,
    /// When [`crate::reaper_stranded::rewake_stranded_inboxes`] last woke each Task.
    stranded_rewoken: std::sync::Mutex<HashMap<String, DateTime<Utc>>>,
}

impl TaskReaper {
    /// Assemble the reaper at the composition root from its ports — each wired
    /// from the router-held state there.
    #[expect(
        clippy::too_many_arguments,
        reason = "the reaper's ports are its explicit per-cycle manifest; the composition root wires each one"
    )]
    pub fn new(
        connector: Arc<dyn SandboxConnector>,
        leader: Arc<dyn LeaderGuard>,
        messages: Arc<Messages>,
        tasks: Arc<TaskCommands>,
        task_turns: Arc<TaskTurns>,
        task_events: Arc<TaskEvents>,
        sandbox: Arc<dyn SandboxLifecycle>,
        task_bindings: Arc<Bindings>,
        events: Option<Arc<dyn EventPublisher>>,
        config: Arc<Config>,
        agent_error: AgentErrorPorts,
        k8s: moatless_sandbox_k8s::client::K8sClient,
    ) -> Self {
        Self {
            connector,
            leader,
            messages,
            tasks,
            task_turns,
            task_events,
            sandbox,
            task_bindings,
            events,
            config,
            agent_error,
            k8s,
            stranded_rewoken: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Orphan-GC pass: LIST labeled sandbox resources, then delete only the
    /// ones whose Task row no longer exists. Fully best-effort.
    async fn gc_orphaned_resources(&self) {
        let task_ids =
            moatless_sandbox_k8s::sandbox_resources::collect_labeled_task_ids(&self.k8s).await;
        if task_ids.is_empty() {
            return;
        }

        // reason: orphan GC checks task existence across all owners; system ctx required.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();
        let mut gc_count = 0u32;
        for task_id in task_ids {
            let task_exists = match self.tasks.find_by_id(&task_id, &sys_ctx).await {
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
                moatless_sandbox_k8s::sandbox_resources::delete_orphaned_task_resources(
                    &self.k8s, &task_id,
                )
                .await;
                gc_count += 1;
            }
        }

        if gc_count > 0 {
            info!(gc_count, "orphan-gc pass complete");
        }
    }

    /// Spawn the unified task reaper as a background tokio task.
    ///
    /// Loops: try to acquire leadership via `LeaderGuard`, run a reap cycle if
    /// leader, then sleep `config.sandbox.idle_check_interval_secs` before
    /// repeating.
    pub fn spawn(self, cancel: CancellationToken) -> JoinHandle<()> {
        metrics::describe_counter!(
            "task_turns_reconciled_total",
            "Open task turns the reaper closed after a lost terminal event, by pass (orphaned | superseded)"
        );
        metrics::describe_counter!(
            "task_turns_interrupted_total",
            "Orphaned task turns the reaper marked interrupted, leaving them for sandbox recovery"
        );
        metrics::describe_counter!(
            "task_turns_reopened_total",
            "Turns allocated by the reaper for a live agent run whose open event was lost"
        );
        metrics::describe_counter!(
            "inbox_stranded_rewakes_total",
            "Open tasks re-woken because a pending message had no turn to take it"
        );
        metrics::describe_counter!(
            "task_turns_disagreed_total",
            "Heartbeats reporting a run the turn row says already ended — a coordinator defect, not a lost event"
        );
        let identity = self.config.server.worker_identity.clone();
        let interval = Duration::from_secs(self.config.sandbox.idle_check_interval_secs);
        let default_ttl_minutes = self.config.sandbox.idle_timeout_minutes as i32;
        let default_settle_after_hours = self.config.sandbox.settle_after_hours as i32;
        let default_close_after_settled_days = self.config.sandbox.close_after_settled_days as i32;
        let default_cold_after_hours = self.config.sandbox.cold_after_hours as i32;
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
                            .run_reap_cycle(ReapCycleDefaults {
                                ttl_minutes: default_ttl_minutes,
                                settle_after_hours: default_settle_after_hours,
                                close_after_settled_days: default_close_after_settled_days,
                                cold_after_hours: default_cold_after_hours,
                                orphan_gc_enabled,
                            })
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

    /// Run one reap cycle: settle idle Open tasks, close settled ones, stop idle
    /// sandboxes, then reclaim the storage of the ones stopped long enough.
    async fn run_reap_cycle(&self, defaults: ReapCycleDefaults) -> Result<(), ReaperError> {
        // The running Sandbox set for this cycle. A failed LIST aborts the cycle
        // (`?`) rather than letting an empty set be read as "nothing is running"
        // and reaping live turns under it.
        let sandbox_ids =
            moatless_sandbox_k8s::sandbox_resources::list_running_sandbox_ids(&self.k8s)
                .await
                .map_err(|e| ReaperError::SandboxQuery(Box::new(e)))?;

        // The Members attached to those Sandboxes. Same failure contract as the
        // LIST: an empty set would be read as "no Members", and every Member's
        // live turn would be closed under it.
        let members = self.tasks.list_sandbox_members(&sandbox_ids).await?;
        let sandboxes = group_members(&sandbox_ids, members);
        let live_ids = live_task_ids(&sandboxes);

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
        self.reconcile_orphaned_turns(&live_ids, &stuck_turns).await;

        // The converse: turns closed while their run kept going. Same
        // best-effort contract.
        match self
            .task_turns
            .list_settled_turns_with_later_activity()
            .await
        {
            Ok(settled) => self.reconcile_live_runs(&live_ids, &settled).await,
            Err(e) => {
                warn!(error = %e, "reconcile: failed to list settled turns with later activity");
            }
        }

        // Settle turns a later turn already ran past. These never reach the
        // pass above — it lists only each task's newest open turn — so without
        // this they stay open for good, reporting the task busy.
        match self.task_turns.list_superseded_open_turns(cutoff).await {
            Ok(superseded) => self.reconcile_superseded_turns(&superseded).await,
            Err(e) => warn!(error = %e, "reconcile: failed to list superseded open turns"),
        }

        crate::reaper_stranded::rewake_stranded_inboxes(
            &self.messages,
            self.agent_error.sandbox_waker.as_ref(),
            &self.stranded_rewoken,
            Utc::now(),
        )
        .await;

        // Orphan-GC deletes only taskless resources. Desired Removed convergence
        // for existing Tasks belongs to the Sandbox reconciler.
        if defaults.orphan_gc_enabled {
            self.gc_orphaned_resources().await;
        }

        let Some(snapshot) = self
            .load_reaper_snapshot(sandboxes, &live_ids, defaults)
            .await?
        else {
            return Ok(());
        };

        let stats = self.apply_reaper_decisions(snapshot).await;
        log_reaper_stats(stats);

        Ok(())
    }
}

/// A running Sandbox and the Tasks attached to it.
struct RunningSandbox {
    sandbox_id: String,
    /// Every active Member, including the Task whose id names the Sandbox.
    /// Never empty: a Sandbox with no fork and no open owner row still lists
    /// its own id, which is what the idle-stop branch decides on.
    members: Vec<String>,
}

/// Group `(task_id, sandbox_id)` pairs under the Sandboxes that are running.
fn group_members(sandbox_ids: &[String], members: Vec<(String, String)>) -> Vec<RunningSandbox> {
    let mut by_sandbox: HashMap<&str, Vec<String>> = sandbox_ids
        .iter()
        .map(|id| (id.as_str(), vec![id.clone()]))
        .collect();
    for (task_id, sandbox_id) in members {
        if let Some(entry) = by_sandbox.get_mut(sandbox_id.as_str()) {
            if !entry.contains(&task_id) {
                entry.push(task_id);
            }
        }
    }
    sandbox_ids
        .iter()
        .map(|sandbox_id| RunningSandbox {
            sandbox_id: sandbox_id.clone(),
            members: by_sandbox
                .remove(sandbox_id.as_str())
                .unwrap_or_else(|| vec![sandbox_id.clone()]),
        })
        .collect()
}

/// Every Task id running inside one of the Sandboxes, deduplicated.
fn live_task_ids(sandboxes: &[RunningSandbox]) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut ids = Vec::new();
    for sandbox in sandboxes {
        for member in &sandbox.members {
            if seen.insert(member.as_str()) {
                ids.push(member.clone());
            }
        }
    }
    ids
}

struct ReaperSnapshot {
    all_ids: Vec<String>,
    sandboxes: Vec<RunningSandbox>,
    open_ids: Vec<String>,
    latest_times: HashMap<String, DateTime<Utc>>,
    ttl_map: HashMap<String, i32>,
    settle_map: HashMap<String, i32>,
    close_map: HashMap<String, i32>,
    /// See [`moatless_tasks::task::dao::TaskDao::get_settled_since_map`].
    settled_since: HashMap<String, DateTime<Utc>>,
    cold_map: HashMap<String, i32>,
    /// When each stopped task's sandbox stopped, for the ones that still own
    /// storage. Absent means nothing to reclaim — never had a sandbox, still
    /// running, or already cold.
    stopped_since: HashMap<String, DateTime<Utc>>,
    agent_lifecycle: HashMap<String, (String, DateTime<Utc>)>,
    /// See [`moatless_tasks::task_event::dao::TaskEventDao::get_latest_agent_activity`].
    agent_activity: HashMap<String, DateTime<Utc>>,
    default_ttl_minutes: i32,
    default_settle_after_hours: i32,
    default_close_after_settled_days: i32,
    default_cold_after_hours: i32,
    now: DateTime<Utc>,
}

#[derive(Clone, Copy)]
struct ReaperStats {
    stopped: u32,
    settled: u32,
    closed: u32,
    reclaimed: u32,
}

// Split across several `impl` blocks so each stays near the free helpers it
// calls (decision fns, DAO-thin loaders, the agent-status port); they'd
// collapse into one if those helpers became methods.
impl TaskReaper {
    async fn load_reaper_snapshot(
        &self,
        sandboxes: Vec<RunningSandbox>,
        live_ids: &[String],
        defaults: ReapCycleDefaults,
    ) -> Result<Option<ReaperSnapshot>, ReaperError> {
        let open_ids = match self.tasks.list_open_task_ids().await {
            Ok(ids) => ids,
            Err(e) => {
                warn!(error = %e, "Failed to list open tasks, skipping auto-settle and auto-close");
                Vec::new()
            }
        };

        // Union the two sets so we fetch activity timestamps for everything we may act on.
        let mut all_ids_set: HashSet<String> =
            HashSet::with_capacity(live_ids.len() + open_ids.len());
        all_ids_set.extend(live_ids.iter().cloned());
        all_ids_set.extend(open_ids.iter().cloned());

        if all_ids_set.is_empty() {
            debug!("No running sandboxes and no open tasks");
            return Ok(None);
        }

        let all_ids: Vec<String> = all_ids_set.iter().cloned().collect();
        debug!(
            sandbox_count = sandboxes.len(),
            member_count = live_ids.len(),
            open_count = open_ids.len(),
            unique_count = all_ids.len(),
            "Task reaper considering tasks"
        );

        let agent_activity = self.load_agent_activity(&all_ids).await;
        let latest_times = self
            .load_latest_activity_times(&all_ids, &agent_activity)
            .await?;
        let ttl_map = load_ttl_map(&self.tasks, &all_ids).await;
        let settle_map = load_settle_map(&self.tasks, &all_ids).await;
        let close_map = load_close_map(&self.tasks, &all_ids).await;
        let settled_since = self.load_settled_since(&open_ids).await;
        let cold_map = load_cold_map(&self.tasks, &all_ids).await;
        let stopped_since = self.load_stopped_since(&open_ids).await;
        let agent_lifecycle = self.load_agent_lifecycle(live_ids).await;

        Ok(Some(ReaperSnapshot {
            all_ids,
            sandboxes,
            open_ids,
            latest_times,
            ttl_map,
            settle_map,
            close_map,
            settled_since,
            cold_map,
            stopped_since,
            agent_lifecycle,
            agent_activity,
            default_ttl_minutes: defaults.ttl_minutes,
            default_settle_after_hours: defaults.settle_after_hours,
            default_close_after_settled_days: defaults.close_after_settled_days,
            default_cold_after_hours: defaults.cold_after_hours,
            now: Utc::now(),
        }))
    }

    /// When each settled Open task was settled. Failure yields an empty map,
    /// which closes nothing and lets the settle branch re-settle at most.
    async fn load_settled_since(&self, open_ids: &[String]) -> HashMap<String, DateTime<Utc>> {
        self.tasks
            .get_settled_since_map(open_ids)
            .await
            .unwrap_or_else(|e| {
                warn!(error = %e, "Failed to read settled tasks, skipping auto-close this cycle");
                HashMap::new()
            })
    }

    /// When each Open task's sandbox stopped, for those that still own storage.
    /// Failure yields an empty map: the cold branch is a cost optimisation, and
    /// skipping a cycle of it costs storage, while guessing costs work.
    async fn load_stopped_since(&self, open_ids: &[String]) -> HashMap<String, DateTime<Utc>> {
        self.sandbox
            .stopped_with_storage(open_ids)
            .await
            .unwrap_or_else(|e| {
                warn!(error = %e, "Failed to read stopped sandboxes, skipping the cold branch this cycle");
                HashMap::new()
            })
    }

    /// Failure yields an empty map: the idle floor then omits tool-call
    /// activity, and no turn is reported silent this cycle.
    async fn load_agent_activity(&self, all_ids: &[String]) -> HashMap<String, DateTime<Utc>> {
        self.task_events
            .get_latest_agent_activity(all_ids)
            .await
            .unwrap_or_else(|e| {
                warn!(error = %e, "Failed to fetch agent activity times, idle floor omits tool-call activity");
                HashMap::new()
            })
    }

    async fn load_latest_activity_times(
        &self,
        all_ids: &[String],
        agent_activity: &HashMap<String, DateTime<Utc>>,
    ) -> Result<HashMap<String, DateTime<Utc>>, ReaperError> {
        let mut latest_times = self.messages.get_latest_message_times(all_ids).await?;

        // A single long turn emits no new conversation message and no lifecycle
        // transition, so agent activity keeps the idle floor fresh during tool calls.
        merge_newest(&mut latest_times, agent_activity.clone());

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

async fn load_ttl_map(tasks: &Arc<TaskCommands>, all_ids: &[String]) -> HashMap<String, i32> {
    tasks.get_ttl_minutes_map(all_ids).await.unwrap_or_else(|e| {
        warn!(error = %e, "Failed to fetch ttl_minutes map, falling back to default for all tasks");
        HashMap::new()
    })
}

async fn load_settle_map(tasks: &Arc<TaskCommands>, all_ids: &[String]) -> HashMap<String, i32> {
    tasks
        .get_settle_after_hours_map(all_ids)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "Failed to fetch settle_after_hours map, falling back to default for all tasks");
            HashMap::new()
        })
}

async fn load_close_map(tasks: &Arc<TaskCommands>, all_ids: &[String]) -> HashMap<String, i32> {
    tasks
        .get_close_after_settled_days_map(all_ids)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "Failed to fetch close_after_settled_days map, falling back to default for all tasks");
            HashMap::new()
        })
}

async fn load_cold_map(tasks: &Arc<TaskCommands>, all_ids: &[String]) -> HashMap<String, i32> {
    tasks
        .get_cold_after_hours_map(all_ids)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "Failed to fetch cold_after_hours map, falling back to default for all tasks");
            HashMap::new()
        })
}

impl TaskReaper {
    async fn load_agent_lifecycle(
        &self,
        live_ids: &[String],
    ) -> HashMap<String, (String, DateTime<Utc>)> {
        // Latest agent run-state per Member: dates the idle-stop floor from
        // when it last stopped, so a running agent is never reaped. Only
        // Members reach the reap branch; falls back to the activity floor on
        // error.
        self.task_events
        .get_latest_agent_lifecycle(live_ids)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "Failed to fetch agent lifecycle events, reaper falls back to activity floor");
            HashMap::new()
        })
    }

    /// Two passes, one per subject. The Task pass closes and reclaims per Task
    /// row; the Sandbox pass stops per pod, folding every Member.
    async fn apply_reaper_decisions(&self, snapshot: ReaperSnapshot) -> ReaperStats {
        let mut stats = ReaperStats {
            stopped: 0,
            settled: 0,
            closed: 0,
            reclaimed: 0,
        };

        let closed = self.apply_task_decisions(&snapshot, &mut stats).await;
        self.apply_sandbox_decisions(&snapshot, &closed, &mut stats)
            .await;
        self.report_silent_turns(&snapshot).await;

        stats
    }

    /// Log each Member whose agent reports `running` but has emitted no `agent`
    /// event for [`SILENT_TURN_THRESHOLD_SECS`]. Only reports: nothing else
    /// acts on a running agent, so the turn holds its sandbox until someone
    /// does.
    async fn report_silent_turns(&self, snapshot: &ReaperSnapshot) {
        let threshold = chrono::Duration::seconds(SILENT_TURN_THRESHOLD_SECS);
        for task_id in snapshot.sandboxes.iter().flat_map(|s| &s.members) {
            let Some(last_activity) = silent_since(
                snapshot.agent_lifecycle.get(task_id),
                snapshot.agent_activity.get(task_id).copied(),
                threshold,
                snapshot.now,
            ) else {
                continue;
            };
            // The live status rules out a turn parked on a question, which
            // also emits nothing while it waits.
            let Some(status) = agent_status_snapshot(self.connector.as_ref(), task_id).await else {
                continue;
            };
            if status.status != "running" {
                continue;
            }
            warn!(
                task_id = %task_id,
                last_activity = %last_activity,
                silent_minutes = (snapshot.now - last_activity).num_minutes(),
                busy_processes = status.busy_processes,
                "Agent turn silent: agent reports running but has emitted no agent event"
            );
        }
    }

    /// The per-Task pass: auto-close a settled Open task, auto-settle an idle
    /// one, or reclaim the storage of one whose sandbox has been stopped long
    /// enough. Returns the tasks it closed.
    async fn apply_task_decisions(
        &self,
        snapshot: &ReaperSnapshot,
        stats: &mut ReaperStats,
    ) -> HashSet<String> {
        let live_set: HashSet<&str> = snapshot
            .sandboxes
            .iter()
            .flat_map(|s| s.members.iter().map(String::as_str))
            .collect();
        let open_set: HashSet<&str> = snapshot.open_ids.iter().map(String::as_str).collect();
        let mut closed = HashSet::new();

        for task_id in &snapshot.all_ids {
            if !open_set.contains(task_id.as_str()) {
                continue;
            }
            let latest_at = snapshot.latest_times.get(task_id).copied();
            let settled_at = snapshot.settled_since.get(task_id).copied();
            let close_after_settled_days = snapshot
                .close_map
                .get(task_id)
                .copied()
                .unwrap_or(snapshot.default_close_after_settled_days);

            // Close supersedes stop — `auto_close_task` calls `cleanup`, which
            // tears down the Deployment regardless of whether it was scaled to 0.
            if decide_close(close_after_settled_days, settled_at, snapshot.now)
                == CloseDecision::Close
            {
                info!(
                    task_id = %task_id,
                    close_after_settled_days,
                    settled_at = ?settled_at,
                    "Auto-closing settled task"
                );
                self.auto_close_task(task_id).await;
                closed.insert(task_id.clone());
                stats.closed += 1;
                continue;
            }

            let settle_after_hours = snapshot
                .settle_map
                .get(task_id)
                .copied()
                .unwrap_or(snapshot.default_settle_after_hours);
            // A follower `auto_settle_task` skips keeps `settled_at` at `None`, so
            // this runs every cycle for such a task; only a moved row is news.
            if settled_at.is_none() && decide_settle(settle_after_hours, latest_at, snapshot.now) {
                let idle_since =
                    snapshot.now - chrono::Duration::hours(i64::from(settle_after_hours));
                match self
                    .tasks
                    .auto_settle_task(task_id, snapshot.now, idle_since)
                    .await
                {
                    Ok(true) => {
                        info!(
                            task_id = %task_id,
                            settle_after_hours,
                            last_activity = ?latest_at,
                            "Auto-settled idle task"
                        );
                        stats.settled += 1;
                    }
                    Ok(false) => {}
                    Err(e) => warn!(task_id = %task_id, error = %e, "auto-settle failed"),
                }
            }

            // A task running inside a pod still holds its volume, so the cold
            // branch has nothing to reclaim from it.
            if !live_set.contains(task_id.as_str())
                && self.reclaim_cold_storage(task_id, snapshot).await
            {
                stats.reclaimed += 1;
            }
        }

        closed
    }

    /// The per-Sandbox pass: stop a pod whose every Member is idle past the
    /// folded TTL.
    async fn apply_sandbox_decisions(
        &self,
        snapshot: &ReaperSnapshot,
        closed: &HashSet<String>,
        stats: &mut ReaperStats,
    ) {
        for sandbox in &snapshot.sandboxes {
            // A close this cycle may already be tearing the pod down. The next
            // cycle decides on the membership the close left behind.
            if sandbox.members.iter().any(|m| closed.contains(m)) {
                continue;
            }

            let latest_at = fold_latest_activity(&sandbox.members, &snapshot.latest_times);
            let reap_floor = fold_reap_floor(
                &sandbox.members,
                &snapshot.agent_lifecycle,
                &snapshot.latest_times,
                snapshot.now,
            );
            let ttl_minutes = fold_ttl_minutes(
                &sandbox.members,
                &snapshot.ttl_map,
                snapshot.default_ttl_minutes,
            );

            match decide_reap(ttl_minutes, reap_floor, snapshot.now) {
                ReapDecision::Skip(reason) => {
                    debug!(sandbox_id = %sandbox.sandbox_id, reason, "Skipping sandbox reap");
                }
                ReapDecision::Keep => {}
                ReapDecision::Stop => {
                    if self
                        .stop_idle_sandbox(sandbox, ttl_minutes, latest_at, reap_floor)
                        .await
                    {
                        stats.stopped += 1;
                    }
                }
            }
        }
    }

    /// The cold branch: reclaim the Deployment and volume of a task that has
    /// been stopped past its cold clock. Returns whether storage was actually
    /// reclaimed.
    async fn reclaim_cold_storage(&self, task_id: &str, snapshot: &ReaperSnapshot) -> bool {
        let cold_after_hours = snapshot
            .cold_map
            .get(task_id)
            .copied()
            .unwrap_or(snapshot.default_cold_after_hours);
        // Absent from the map means there is no storage to reclaim: the task
        // never had a sandbox, or an earlier cycle already cooled it.
        let Some(stopped_at) = snapshot.stopped_since.get(task_id).copied() else {
            return false;
        };
        if !decide_cold(cold_after_hours, stopped_at, snapshot.now) {
            return false;
        }

        info!(
            task_id = %task_id,
            cold_after_hours,
            stopped_at = %stopped_at,
            "Reclaiming the storage of a long-stopped sandbox"
        );
        match self.sandbox.archive_storage(task_id).await {
            Ok(()) => true,
            Err(e) => {
                // Every failure path keeps the volume, so this is a warning and
                // not a lost workspace; the next cycle retries.
                warn!(task_id = %task_id, error = %e, "Failed to reclaim sandbox storage");
                false
            }
        }
    }

    async fn stop_idle_sandbox(
        &self,
        sandbox: &RunningSandbox,
        ttl_minutes: i32,
        latest_at: Option<DateTime<Utc>>,
        reap_floor: Option<DateTime<Utc>>,
    ) -> bool {
        // Defense in depth: the activity floor can go stale, so each
        // Member's agent is asked before the pod stops. One Member still
        // working keeps it, since the pod hosts every Member's process; an
        // unreadable status falls through.
        for member in &sandbox.members {
            let Some(status) = agent_status_snapshot(self.connector.as_ref(), member).await else {
                continue;
            };
            if agent_holds_the_sandbox(&status.status) {
                info!(
                    sandbox_id = %sandbox.sandbox_id,
                    task_id = %member,
                    ttl_minutes,
                    last_activity = ?latest_at,
                    "Idle floor expired but a member's agent reports running, skipping stop"
                );
                return false;
            }
            // The sandbox bounds this itself: a registered command is killed at
            // its own deadline and stops counting as busy, so there is no cap to
            // apply here. A command may legitimately declare a deadline longer
            // than any fixed one this side could pick.
            if status.busy_processes {
                info!(
                    sandbox_id = %sandbox.sandbox_id,
                    task_id = %member,
                    ttl_minutes,
                    last_activity = ?latest_at,
                    "Idle floor expired but a member reports a running registered command, deferring stop"
                );
                return false;
            }
            let last_preview_request = status
                .last_preview_request_unix_ms
                .and_then(DateTime::<Utc>::from_timestamp_millis);
            if preview_holds_the_sandbox(last_preview_request, reap_floor, ttl_minutes, Utc::now())
            {
                info!(
                    sandbox_id = %sandbox.sandbox_id,
                    task_id = %member,
                    ttl_minutes,
                    last_activity = ?latest_at,
                    last_preview_request = ?last_preview_request,
                    "Idle floor expired but a Script's preview is in use, deferring stop"
                );
                return false;
            }
        }

        info!(
            sandbox_id = %sandbox.sandbox_id,
            member_count = sandbox.members.len(),
            ttl_minutes,
            last_activity = ?latest_at,
            "Stopping idle sandbox"
        );
        match self
            .sandbox
            .stop(&sandbox.sandbox_id, SandboxStopReason::IdleReaper)
            .await
        {
            Ok(()) => {
                info!(sandbox_id = %sandbox.sandbox_id, "Idle sandbox stopped successfully");
                true
            }
            Err(e) => {
                warn!(sandbox_id = %sandbox.sandbox_id, error = %e, "Failed to stop idle sandbox");
                false
            }
        }
    }
}

/// Whether a live agent status is one the sandbox must be kept alive for.
///
/// `waiting` is not: the run is parked on a person and doing no work, so the
/// ordinary idle TTL decides; answering it after a reclaim starts a fresh turn
/// carrying the stored question.
fn agent_holds_the_sandbox(status: &str) -> bool {
    !matches!(status, "idle" | "stopped" | "waiting")
}

fn log_reaper_stats(stats: ReaperStats) {
    if stats.stopped > 0 || stats.settled > 0 || stats.closed > 0 || stats.reclaimed > 0 {
        info!(
            stopped_count = stats.stopped,
            settled_count = stats.settled,
            closed_count = stats.closed,
            reclaimed_count = stats.reclaimed,
            "Task reaper cycle complete"
        );
    } else {
        debug!("Task reaper cycle complete, nothing to do");
    }
}

/// Reconcile turns whose terminal event was lost: a pod killed before its
/// agent emitted it, or a publish dropped in a database outage.
///
/// Acts only on a confirmed orphan (task in no pod, or agent `idle`/`stopped`);
/// a `running` or undetermined agent is left alone. An unanswered orphan is
/// interrupted first; see [`orphaned_turn_action`].
impl TaskReaper {
    async fn reconcile_orphaned_turns(&self, live_task_ids: &[String], stuck: &[UnclosedTurn]) {
        if stuck.is_empty() {
            return;
        }

        let running: HashSet<&str> = live_task_ids.iter().map(String::as_str).collect();

        for turn in stuck {
            if !self.task_desires_running(&turn.task_id).await {
                debug!(
                    task_id = %turn.task_id,
                    "reconcile: task does not desire a running sandbox, leaving turn for retry"
                );
                continue;
            }
            let in_live_sandbox = running.contains(turn.task_id.as_str());
            let agent_status = if in_live_sandbox {
                // Sandbox is up — only orphaned if the agent has gone terminal.
                // Unknown status ⇒ conservative skip (treat as possibly live).
                match agent_status_snapshot(self.connector.as_ref(), &turn.task_id).await {
                    Some(resp) if agent_turn_is_over(&resp.status) => Some(resp.status),
                    _ => {
                        debug!(task_id = %turn.task_id, "reconcile: turn still live, skipping");
                        continue;
                    }
                }
            } else {
                // The task runs in no pod ⇒ nothing can still be driving the turn.
                None
            };

            self.close_orphaned_turn(turn, in_live_sandbox, agent_status.as_deref())
                .await;
        }
    }

    /// Reconcile turns closed while a new run, whose ack and `taskTurn.active`
    /// were both lost, keeps working: the task would read idle.
    ///
    /// [`classify_live_run`] decides what the heartbeat means; a
    /// [`LiveRunVerdict::Contradiction`] is counted but never written.
    async fn reconcile_live_runs(&self, live_task_ids: &[String], settled: &[SettledTurn]) {
        if settled.is_empty() {
            return;
        }

        let running: HashSet<&str> = live_task_ids.iter().map(String::as_str).collect();

        for turn in settled {
            // No pod means nothing can be driving a run, whatever the activity
            // floor says.
            if !running.contains(turn.task_id.as_str()) {
                continue;
            }
            if !self.task_desires_running(&turn.task_id).await {
                continue;
            }

            let Some(status) = agent_status_snapshot(self.connector.as_ref(), &turn.task_id).await
            else {
                // Unreachable is not evidence of anything. Same conservative
                // reading as the orphaned-turn pass.
                continue;
            };

            match classify_live_run(
                &status.status,
                status.run_id.as_deref(),
                turn.run_id.as_deref(),
            ) {
                LiveRunVerdict::OpenTurn(run_id) => {
                    self.open_turn_for_live_run(turn, run_id).await;
                }
                LiveRunVerdict::Contradiction => {
                    warn!(
                        task_id = %turn.task_id,
                        turn_number = turn.turn_number,
                        run_id = ?turn.run_id,
                        agent_status = %status.status,
                        "reconcile: agent reports a run this turn already closed"
                    );
                    metrics::counter!("task_turns_disagreed_total").increment(1);
                }
                LiveRunVerdict::Ignore => {}
            }
        }
    }

    /// Allocate the turn a live run is working under; see
    /// [`TaskTurns::resolve_turn_for_run`], the sole allocator.
    async fn open_turn_for_live_run(&self, turn: &SettledTurn, run_id: &str) {
        match self
            .task_turns
            .resolve_turn_for_run(&turn.task_id, run_id)
            .await
        {
            Ok(turn_number) => {
                info!(
                    task_id = %turn.task_id,
                    turn_number,
                    run_id,
                    closed_turn = turn.turn_number,
                    "reconcile: opened a turn for a live run whose start was never recorded"
                );
                metrics::counter!("task_turns_reopened_total").increment(1);
            }
            Err(e) => {
                warn!(
                    task_id = %turn.task_id,
                    run_id,
                    error = %e,
                    "reconcile: failed to open a turn for a live run"
                );
            }
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

    /// Settle turns a later turn already superseded.
    ///
    /// A turn whose terminal event was lost before the next run began is left
    /// `interrupted` by [`TaskTurns::resolve_turn_for_run`], and nothing else
    /// closes it. It settles `completed` or `stopped`, with no `taskTurn` event.
    async fn reconcile_superseded_turns(&self, superseded: &[UnclosedTurn]) {
        for turn in superseded {
            let Some(turn_number) = turn.turn_number else {
                continue;
            };
            let status = match self
                .messages
                .last_message_is_completed_reply(&turn.task_id, Some(turn_number))
                .await
            {
                Ok(true) => TurnStatus::Completed,
                Ok(false) => TurnStatus::Stopped,
                Err(e) => {
                    warn!(
                        task_id = %turn.task_id,
                        error = %e,
                        "reconcile: failed to inspect superseded turn's last message; settling as stopped"
                    );
                    TurnStatus::Stopped
                }
            };

            if let Err(e) = self
                .task_turns
                .project_turn(
                    &turn.task_id,
                    turn_number,
                    turn.agent_profile.clone(),
                    TurnTransition::Terminal(status),
                    Utc::now(),
                )
                .await
            {
                warn!(
                    task_id = %turn.task_id,
                    turn_number,
                    error = %e,
                    "reconcile: failed to settle superseded turn"
                );
                continue;
            }

            metrics::counter!("task_turns_reconciled_total", "pass" => "superseded").increment(1);
            info!(
                task_id = %turn.task_id,
                turn_number,
                status = status.as_str(),
                "Settled a superseded open turn"
            );
        }
    }
}

/// What a live heartbeat means for a turn whose row says it already closed.
#[derive(Debug, PartialEq, Eq)]
enum LiveRunVerdict<'a> {
    /// A run is in flight that this row never covered — open its turn.
    OpenTurn(&'a str),
    /// The heartbeat and the row describe the same run and disagree about
    /// whether it ended. Neither can be trusted over the other here.
    Contradiction,
    /// Nothing is in flight, or nothing can be said about what is.
    Ignore,
}

/// Decide what a heartbeat means against the run its turn row recorded.
///
/// This is the whole of the decision; everything around it in the reconcile
/// loop is a lookup.
fn classify_live_run<'a>(
    agent_status: &str,
    live_run_id: Option<&'a str>,
    recorded_run_id: Option<&str>,
) -> LiveRunVerdict<'a> {
    if agent_turn_is_over(agent_status) {
        return LiveRunVerdict::Ignore;
    }
    match (live_run_id, recorded_run_id) {
        // A run that names itself, and it is not the one this row closed.
        (Some(live), Some(recorded)) if live != recorded => LiveRunVerdict::OpenTurn(live),
        // The row predates run attribution, so it can only be about earlier
        // work; the live run names itself and is the newer of the two.
        (Some(live), None) => LiveRunVerdict::OpenTurn(live),
        // Same run, or a heartbeat that cannot say which run it describes —
        // an older sandbox image. Neither is a lost open event.
        _ => LiveRunVerdict::Contradiction,
    }
}

/// What the reaper does with an orphaned turn the agent never answered.
#[derive(Debug, PartialEq, Eq)]
enum OrphanedTurnAction {
    /// First sighting: mark the numbered turn `interrupted` for recovery.
    Interrupt(i32),
    /// Interrupted, and still inside the recovery window.
    Wait,
    /// Recovery had its window, or the turn has no number to resume by.
    Close,
}

/// Decide an unanswered orphaned turn's fate from when it was interrupted.
///
/// An agent reporting `idle` under an open turn is not proof the run ended on
/// its own: a pod shutting down stops its agent before its `turn_interrupted`
/// event is delivered, so closing on first sight races the replacement pod.
fn orphaned_turn_action(turn: &UnclosedTurn, now: DateTime<Utc>) -> OrphanedTurnAction {
    let Some(turn_number) = turn.turn_number else {
        return OrphanedTurnAction::Close;
    };
    match turn.interrupted_at {
        None => OrphanedTurnAction::Interrupt(turn_number),
        Some(at) if now - at < chrono::Duration::seconds(ORPHANED_TURN_RECOVERY_WINDOW_SECS) => {
            OrphanedTurnAction::Wait
        }
        Some(_) => OrphanedTurnAction::Close,
    }
}

/// Whether a live agent status means the turn it was running has ended.
///
/// `waiting` does not: the run is parked mid-turn and resumes when the answer
/// lands. The sandbox under it may still be reclaimed — see
/// [`agent_holds_the_sandbox`].
fn agent_turn_is_over(status: &str) -> bool {
    matches!(status, "idle" | "stopped")
}

/// Live agent status straight from the sandbox, `None` when unreachable (no
/// client, gRPC error).
async fn agent_status_snapshot(
    connector: &dyn SandboxConnector,
    task_id: &str,
) -> Option<sandbox_proto::GetAgentStatusResponse> {
    let client = match connector.sandbox_client(task_id).await {
        Ok(c) => c,
        Err(e) => {
            debug!(task_id, error = %e, "reconcile: no sandbox client for status check");
            return None;
        }
    };
    match client.get_agent_status_for_task(Some(task_id)).await {
        Ok(resp) => Some(resp),
        Err(e) => {
            debug!(task_id, error = %e, "reconcile: get_agent_status failed");
            None
        }
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

/// Close a stuck turn: if the agent already delivered its final reply the
/// terminal event was merely lost, so close as `completed`; otherwise interrupt
/// it first and deliver a synthetic `agent.error` once
/// [`orphaned_turn_action`] says recovery had its chance.
impl TaskReaper {
    async fn close_orphaned_turn(
        &self,
        turn: &UnclosedTurn,
        in_live_sandbox: bool,
        agent_status: Option<&str>,
    ) {
        // False-negative guard: a lost terminal event isn't a lost result. If
        // the agent's final reply for this turn is already in the chat,
        // close it as `completed`, not `error`.
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
                    "reconcile: failed to inspect last message; treating the turn as unanswered"
                );
            }
        }

        match orphaned_turn_action(turn, Utc::now()) {
            OrphanedTurnAction::Interrupt(turn_number) => {
                self.interrupt_orphaned_turn(turn, turn_number, in_live_sandbox, agent_status)
                    .await;
            }
            OrphanedTurnAction::Wait => {
                debug!(
                    task_id = %turn.task_id,
                    turn_number = ?turn.turn_number,
                    "reconcile: orphaned turn still inside its recovery window"
                );
            }
            OrphanedTurnAction::Close => {
                info!(
                    task_id = %turn.task_id,
                    turn_number = ?turn.turn_number,
                    in_live_sandbox,
                    agent_status = ?agent_status,
                    "reconcile: no sandbox recovered the interrupted turn; closing it as error"
                );
                close_orphaned_turn_as_error(&self.agent_error, turn).await;
            }
        }
    }

    /// Mark an orphaned turn `interrupted` without a terminal event, so the
    /// replacement pod's `find_interrupted_turn` still resumes it.
    async fn interrupt_orphaned_turn(
        &self,
        turn: &UnclosedTurn,
        turn_number: i32,
        in_live_sandbox: bool,
        agent_status: Option<&str>,
    ) {
        if let Err(e) = self
            .task_turns
            .project_turn(
                &turn.task_id,
                turn_number,
                turn.agent_profile.clone(),
                TurnTransition::Interrupted,
                Utc::now(),
            )
            .await
        {
            warn!(
                task_id = %turn.task_id,
                turn_number,
                error = %e,
                "reconcile: failed to mark orphaned turn interrupted"
            );
            return;
        }

        if let Some(ref events) = self.events {
            events
                .publish_task_turn_event(
                    &turn.task_id,
                    TaskTurnSubType::Interrupted,
                    turn.agent_profile.as_deref(),
                    Some(turn_number),
                    serde_json::json!({
                        "message": "Turn interrupted — the agent stopped without reporting a result. \
                                    It resumes if the sandbox restarts.",
                        "reason": "reconciled_orphaned_turn",
                    }),
                )
                .await;
        }

        metrics::counter!("task_turns_interrupted_total").increment(1);
        info!(
            task_id = %turn.task_id,
            turn_number,
            in_live_sandbox,
            agent_status = ?agent_status,
            "Interrupted orphaned active turn; leaving it for sandbox recovery"
        );
    }

    /// Close a stuck turn whose agent already replied: project to `completed`,
    /// reopen the task, and emit `taskTurn/completed`.
    ///
    /// Projected directly, since a lost terminal event is exactly the failure
    /// this path guards against.
    async fn close_orphaned_turn_as_completed(&self, turn: &UnclosedTurn) {
        if let Some(turn_number) = turn.turn_number {
            if let Err(e) = self
                .task_turns
                .project_turn(
                    &turn.task_id,
                    turn_number,
                    turn.agent_profile.clone(),
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

        // Reopens unless closed mid-run, mirroring the agent terminal
        // handlers: automation never resurrects a Closed task.
        // reason: orphaned-turn reconcile spans all task owners; system ctx.
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
                    turn.agent_profile.as_deref(),
                    turn.turn_number,
                    serde_json::json!({ "reason": "reconciled_orphaned_turn_completed" }),
                )
                .await;
        }

        metrics::counter!("task_turns_reconciled_total", "pass" => "orphaned").increment(1);
        info!(
            task_id = %turn.task_id,
            turn_number = ?turn.turn_number,
            "Reconciled orphaned active turn as completed (agent had already replied)"
        );
    }
}

/// Force a stuck turn to `error` via a synthetic `agent.error` through the same
/// sink a real one takes: appends an error message, reopens the task, and
/// publishes `taskTurn/error`.
///
/// Free-standing so the reconcile delivery is testable without constructing a
/// full reaper.
async fn close_orphaned_turn_as_error(ports: &AgentErrorPorts, turn: &UnclosedTurn) {
    let event = reconcile_error_event(turn);
    crate::event_handler::agent_handlers::handle_agent_error(ports, &event).await;
    metrics::counter!("task_turns_reconciled_total", "pass" => "orphaned").increment(1);
    info!(
        task_id = %turn.task_id,
        turn_number = ?turn.turn_number,
        "Reconciled orphaned active turn"
    );
}

/// The synthetic `agent.error` that closes an orphaned turn.
///
/// The event id is deterministic per `(task, turn)`, so repeated reconcile
/// cycles upsert the same row instead of stacking duplicates.
fn reconcile_error_event(turn: &UnclosedTurn) -> AgentEvent {
    let turn_label = turn
        .turn_number
        .map_or_else(|| "unknown".to_string(), |n| n.to_string());
    AgentEvent {
        event_id: format!("reconcile-{}-{}", turn.task_id, turn_label),
        event_type: EventType::Agent,
        sub_type: "error".to_string(),
        task_id: turn.task_id.clone(),
        timestamp: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        is_error: true,
        agent_profile: turn.agent_profile.clone(),
        turn_number: turn.turn_number,
        payload: serde_json::json!({
            "error": "The agent stopped without reporting a result. The backend closed this turn so you can continue.",
            "reason": "reconciled_orphaned_turn",
        }),
    }
}

impl TaskReaper {
    /// Mirror of [`moatless_tasks::task::handlers::close_task`] for unattended
    /// use; every step logs on failure but never propagates, so the reaper
    /// loop never panics.
    ///
    /// Unlike [`moatless_tasks::task::closer::TaskCloser::close_single`],
    /// this lacks an already-Closed idempotence guard.
    async fn auto_close_task(&self, task_id: &str) {
        // reason: reaper acts across all task owners; system ctx bypasses per-user scope.
        #[allow(clippy::disallowed_methods)]
        let sys_ctx = ScopeContext::system();

        let agent_profile = match self.tasks.find_by_id(task_id, &sys_ctx).await {
            Ok(Some(row)) => Some(row.agent_profile),
            Ok(None) => {
                warn!(task_id, "auto-close: task vanished mid-cycle");
                return;
            }
            Err(e) => {
                warn!(task_id, error = %e, "auto-close: find_by_id failed");
                return;
            }
        };

        // Commit `Closed` before teardown: a concurrent `auto_provision` or
        // `get_pending_messages` then sees `Closed` and bails, so a teardown
        // failure below can't leave the task non-terminal.
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
        if let Err(e) = self.task_bindings.deactivate_for_task(task_id).await {
            warn!(task_id, error = %e, "auto-close: failed to deactivate bindings");
        }

        if let (Some(events), Some(at)) = (&self.events, agent_profile) {
            events
                .publish_task_event(
                    task_id,
                    TaskSubType::Closed,
                    Some(at.as_str()),
                    serde_json::json!({}),
                )
                .await;
        }
        info!(task_id, "Task auto-closed (settled past TTL)");
    }
}

#[cfg(test)]
mod tests;
