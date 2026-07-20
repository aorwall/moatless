//! Background-completion watcher for `run_in_background` jobs.
//!
//! In subprocess mode each agent turn is a one-shot Claude CLI process that
//! exits when the turn's `result` arrives. A `Bash(run_in_background)` job the
//! CLI spawned is reparented to the sandbox process (container PID 1) and keeps
//! running, but the CLI's own "you'll be notified when it completes" message
//! dies with the subprocess — so nothing re-triggers the agent. This job fills
//! that gap: armed at a clean turn end when live descendant processes remain,
//! it polls the specific processes captured at arm time and, once they are all
//! gone, injects a synthetic user message so the agent picks their output back
//! up (the same re-trigger path `ScheduleWakeup` uses).
//!
//! Watching the *specific* PIDs (rather than "any descendant") means a later
//! turn's CLI subprocess never keeps the watch alive, and re-arming at each
//! turn end is latest-wins so the snapshot stays fresh.

use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;
use tracing::{debug, info};

use crate::process_tree::{self, BackgroundProcess};

use super::scheduled_job::{
    BackgroundWatchSnapshot, JobContext, JobKind, JobSnapshot, ScheduledJob,
};

/// How often the watcher re-checks whether its processes are still alive.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Settle window before the watch commits to its process set. The per-turn CLI
/// subprocess emits its `result` just before exiting, so a snapshot taken right
/// at turn end can momentarily include the CLI's own (exiting) processes. After
/// this delay those have gone, leaving only genuine background work — if nothing
/// remains, the watch exits without ever waking the agent.
const CONFIRM_DELAY: Duration = Duration::from_secs(3);

/// Hard cap on how long a single watch runs before giving up without waking the
/// agent. Mirrors the backend's `busy_extension_max_hours` default so a stuck
/// daemon can't hold a watch (or defer idle-reaping) indefinitely.
const MAX_WATCH_DURATION: Duration = Duration::from_hours(4);

/// How many command previews to keep in the status snapshot / wake prompt.
const MAX_COMMAND_PREVIEWS: usize = 5;

/// Which of the given PIDs are still live.
///
/// Injectable because the real implementation reads `/proc`: on a non-Linux
/// host every read fails soft and reports "gone", so a test driving real
/// processes would see them dead from the start and never exercise the watch.
pub(crate) trait ProcessLiveness: Send + Sync + 'static {
    /// The subset of `pids` still live — used once, to settle the watch set.
    fn live_pids(&self, pids: &[u32]) -> Vec<u32>;

    /// Whether any of `pids` is still live. Split from [`live_pids`] because
    /// the poll loop only needs the answer, not the set.
    fn any_live(&self, pids: &[u32]) -> bool {
        !self.live_pids(pids).is_empty()
    }
}

/// Production implementation, backed by `/proc`.
struct ProcLiveness;

impl ProcessLiveness for ProcLiveness {
    fn live_pids(&self, pids: &[u32]) -> Vec<u32> {
        process_tree::live_pids(pids)
    }

    fn any_live(&self, pids: &[u32]) -> bool {
        // Short-circuits and allocates nothing — this runs on every poll tick.
        process_tree::any_pid_live(pids)
    }
}

/// A watch armed over the background processes live at the end of a turn.
pub struct BackgroundWatchJob {
    id: String,
    registered_at_unix_ms: i64,
    /// PIDs captured at arm time — the watch completes when none remain live.
    watched_pids: Vec<u32>,
    /// Command previews for status display and the wake prompt.
    commands: Vec<String>,
    poll_interval: Duration,
    confirm_delay: Duration,
    deadline: Instant,
    liveness: Arc<dyn ProcessLiveness>,
}

impl BackgroundWatchJob {
    /// Snapshot the sandbox's live descendants and arm a watch over them.
    /// Returns `None` when there is no background work to watch.
    ///
    /// Reads `/proc` synchronously — call from a blocking context
    /// (`spawn_blocking`) so it never stalls the async runtime.
    pub fn arm(sandbox_pid: u32) -> Option<Self> {
        let procs = process_tree::live_descendants(sandbox_pid);
        Self::from_processes(procs)
    }

    /// Build a watch from an already-captured process list (unit-test seam).
    fn from_processes(procs: Vec<BackgroundProcess>) -> Option<Self> {
        if procs.is_empty() {
            return None;
        }
        let watched_pids = procs.iter().map(|p| p.pid).collect();
        let commands = procs
            .iter()
            .take(MAX_COMMAND_PREVIEWS)
            .map(|p| p.command.clone())
            .collect();
        Some(Self {
            id: format!("bgwatch-{}", uuid::Uuid::new_v4()),
            registered_at_unix_ms: unix_now_ms(),
            watched_pids,
            commands,
            poll_interval: POLL_INTERVAL,
            confirm_delay: CONFIRM_DELAY,
            deadline: Instant::now() + MAX_WATCH_DURATION,
            liveness: Arc::new(ProcLiveness),
        })
    }

    /// Test-only constructor with explicit timings, no deadline cap, and an
    /// injected liveness source.
    #[cfg(test)]
    pub(crate) fn new_for_test(
        watched_pids: Vec<u32>,
        poll_interval: Duration,
        confirm_delay: Duration,
        liveness: Arc<dyn ProcessLiveness>,
    ) -> Self {
        Self {
            id: format!("bgwatch-test-{}", watched_pids.len()),
            registered_at_unix_ms: unix_now_ms(),
            watched_pids,
            commands: vec!["sleep".to_string()],
            poll_interval,
            confirm_delay,
            deadline: Instant::now() + Duration::from_hours(1),
            liveness,
        }
    }

    /// Human-readable message injected once the watched processes finish.
    fn wake_prompt(&self) -> String {
        let count = self.watched_pids.len();
        let noun = if count == 1 { "job" } else { "jobs" };
        let commands = if self.commands.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.commands.join("; "))
        };
        format!(
            "A background process you started has finished. {count} background {noun}{commands} \
             that {verb} running when your last turn ended {verb} no longer active. Review the \
             output (for example with the BashOutput tool, or by reading any log file you \
             redirected output to) and continue the task.",
            verb = if count == 1 { "was" } else { "were" },
        )
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Number of background processes this watch is waiting on.
    pub fn process_count(&self) -> usize {
        self.watched_pids.len()
    }
}

#[async_trait::async_trait]
impl ScheduledJob for BackgroundWatchJob {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> JobKind {
        JobKind::BackgroundWatch
    }

    fn snapshot(&self) -> JobSnapshot {
        JobSnapshot::BackgroundWatch(BackgroundWatchSnapshot {
            id: self.id.clone(),
            registered_at_unix_ms: self.registered_at_unix_ms,
            process_count: self.watched_pids.len() as u32,
            commands: self.commands.clone(),
        })
    }

    async fn run(self: Box<Self>, ctx: JobContext) {
        // Settle window: let the just-exited CLI subprocess drop out of the
        // captured set before committing to watch anything.
        tokio::select! {
            () = ctx.job_cancel.cancelled() => return,
            () = ctx.run_cancel.cancelled() => return,
            () = tokio::time::sleep(self.confirm_delay) => {}
        }

        let initial = self.watched_pids.clone();
        let settle_liveness = Arc::clone(&self.liveness);
        let watched =
            tokio::task::spawn_blocking(move || settle_liveness.live_pids(&initial))
                .await
                .unwrap_or_default();
        if watched.is_empty() {
            debug!(
                id = %self.id,
                "No background processes survived the settle window; not waking the agent"
            );
            return;
        }

        let mut ticker = tokio::time::interval(self.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = ctx.job_cancel.cancelled() => {
                    debug!(id = %self.id, "Background watch cancelled (job token)");
                    return;
                }
                () = ctx.run_cancel.cancelled() => {
                    debug!(id = %self.id, "Background watch cancelled (run token)");
                    return;
                }
                _ = ticker.tick() => {
                    if Instant::now() >= self.deadline {
                        info!(
                            id = %self.id,
                            "Background watch hit max duration; stopping without waking the agent"
                        );
                        return;
                    }
                    let pids = watched.clone();
                    let poll_liveness = Arc::clone(&self.liveness);
                    let still_running =
                        tokio::task::spawn_blocking(move || poll_liveness.any_live(&pids))
                            .await
                            .unwrap_or(false);
                    if !still_running {
                        info!(
                            id = %self.id,
                            count = watched.len(),
                            "Background processes finished; waking the agent"
                        );
                        ctx.injector.inject(self.wake_prompt());
                        return;
                    }
                }
            }
        }
    }
}

fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_returns_none_without_processes() {
        assert!(BackgroundWatchJob::from_processes(vec![]).is_none());
    }

    #[test]
    fn arm_captures_pids_and_commands() {
        let job = BackgroundWatchJob::from_processes(vec![
            BackgroundProcess {
                pid: 42,
                command: "cargo test --no-run".to_string(),
            },
            BackgroundProcess {
                pid: 43,
                command: "sleep 30".to_string(),
            },
        ])
        .expect("processes present");

        let JobSnapshot::BackgroundWatch(snap) = job.snapshot() else {
            panic!("expected background-watch snapshot");
        };
        assert_eq!(snap.process_count, 2);
        assert_eq!(snap.commands.len(), 2);
        assert_eq!(job.kind(), JobKind::BackgroundWatch);
        assert!(job.wake_prompt().contains("cargo test --no-run"));
        assert!(job.wake_prompt().contains("jobs"));
    }

    #[test]
    fn command_previews_are_capped() {
        let procs: Vec<BackgroundProcess> = (0..10)
            .map(|i| BackgroundProcess {
                pid: 100 + i,
                command: format!("proc-{i}"),
            })
            .collect();
        let job = BackgroundWatchJob::from_processes(procs).expect("processes present");
        let JobSnapshot::BackgroundWatch(snap) = job.snapshot() else {
            panic!("expected background-watch snapshot");
        };
        assert_eq!(snap.process_count, 10, "count reflects all watched pids");
        assert_eq!(
            snap.commands.len(),
            MAX_COMMAND_PREVIEWS,
            "previews are capped"
        );
    }

    #[test]
    fn single_job_prompt_uses_singular_grammar() {
        let job = BackgroundWatchJob::from_processes(vec![BackgroundProcess {
            pid: 7,
            command: "sleep 5".to_string(),
        }])
        .expect("process present");
        let prompt = job.wake_prompt();
        assert!(prompt.contains("1 background job"));
        assert!(prompt.contains("was running"));
    }
}
