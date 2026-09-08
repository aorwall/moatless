//! Completion watcher for commands registered through `moat cmd`.
//!
//! In subprocess mode each agent turn is a one-shot Claude CLI process that
//! exits when the turn's `result` arrives, so nothing re-triggers the agent when
//! work it left running finishes. This job fills that gap: armed at a clean turn
//! end over the commands still running, it polls the registry and injects a
//! synthetic user message as soon as one of them reaches a terminal state (the
//! same re-trigger path `ScheduleWakeup` uses).
//!
//! It wakes on the *first* command to finish rather than the last, so a build
//! running alongside a long-lived server still reports the moment it is done,
//! and it covers commands that died before the turn ended as well as those
//! still running — a command that fails in under a second is exactly the case
//! the agent has no other way to hear about.

use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info};

use crate::commands::{CommandRecord, CommandRegistry, CommandState};

use super::scheduled_job::{
    BackgroundWatchSnapshot, JobContext, JobKind, JobSnapshot, ScheduledJob,
};

/// How often the watcher re-checks the registry.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// How many command labels to keep in the status snapshot / wake prompt.
const MAX_COMMAND_PREVIEWS: usize = 5;

/// Log lines carried into the wake for a command that did not succeed.
const WAKE_LOG_LINES: usize = 20;

/// Bytes read from the end of the log to find those lines.
const WAKE_LOG_BYTES: u64 = 4096;

/// A watch armed over the commands still running at the end of a turn.
pub struct BackgroundWatchJob {
    id: String,
    registered_at_unix_ms: i64,
    watched_ids: Vec<String>,
    commands: Vec<String>,
    poll_interval: Duration,
    registry: Arc<CommandRegistry>,
}

impl BackgroundWatchJob {
    /// Arm a watch over every command that still owes the agent an outcome.
    /// `None` when none does, so a turn that left no work behind registers
    /// nothing.
    ///
    /// The list is the registry's unreported set, not its running set: a
    /// command that already finished has to be watched (it wakes on the first
    /// poll tick, which `tokio::time::interval` fires at once), and one the
    /// agent killed must not be, even while its process is still winding down.
    pub async fn arm(registry: Arc<CommandRegistry>) -> Option<Self> {
        let owed: Vec<CommandRecord> = registry.unreported().await;
        if owed.is_empty() {
            return None;
        }
        Some(Self {
            id: format!("bgwatch-{}", uuid::Uuid::new_v4()),
            registered_at_unix_ms: unix_now_ms(),
            watched_ids: owed.iter().map(|c| c.id.clone()).collect(),
            commands: owed
                .iter()
                .take(MAX_COMMAND_PREVIEWS)
                .map(|c| c.label.clone())
                .collect(),
            poll_interval: POLL_INTERVAL,
            registry,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Message injected once a watched command finishes. A command that did not
    /// succeed carries the end of its log: the failure is the reason the agent
    /// is being woken, so making it read the log itself wastes the turn the
    /// wake just bought.
    fn wake_prompt(finished: &[CommandRecord]) -> String {
        let lines: Vec<String> = finished
            .iter()
            .map(|c| {
                let (outcome, failed) = match (c.state, c.exit_code) {
                    (CommandState::TimedOut, _) => {
                        (format!("timed out after {}s", c.timeout.as_secs()), true)
                    }
                    (CommandState::Killed, _) => ("killed".to_string(), true),
                    (_, Some(0)) => ("succeeded".to_string(), false),
                    (_, Some(code)) => (format!("failed with exit {code}"), true),
                    (_, None) => ("ended without an exit code".to_string(), true),
                };
                let head = format!(
                    "- `{}` ({}) {} — output: {}",
                    c.id,
                    c.label,
                    outcome,
                    c.log_path.display()
                );
                match failed
                    .then(|| crate::commands::log_tail(&c.log_path, WAKE_LOG_LINES, WAKE_LOG_BYTES))
                    .flatten()
                {
                    Some(tail) => format!("{head}\n\n```\n{tail}\n```"),
                    None => head,
                }
            })
            .collect();
        let noun = if finished.len() == 1 {
            "command has"
        } else {
            "commands have"
        };
        format!(
            "A background {noun} finished:\n{}\n\nRead the output and continue the task.",
            lines.join("\n")
        )
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Number of commands this watch is waiting on.
    pub fn command_count(&self) -> usize {
        self.watched_ids.len()
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
            command_count: self.watched_ids.len() as u32,
            commands: self.commands.clone(),
        })
    }

    async fn run(self: Box<Self>, ctx: JobContext) {
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
                    // Re-read `reported` rather than trusting the armed set: a
                    // later turn can kill or read back a command this watch is
                    // still waiting on, and waking for it then would announce
                    // an outcome the agent already has.
                    let owed = self.registry.unreported_among(&self.watched_ids).await;
                    if owed.is_empty() {
                        debug!(
                            id = %self.id,
                            "Every watched command was reported elsewhere; nothing left to wake for"
                        );
                        return;
                    }
                    let finished: Vec<CommandRecord> =
                        owed.into_iter().filter(|c| c.state.is_terminal()).collect();
                    if !finished.is_empty() {
                        info!(
                            id = %self.id,
                            count = finished.len(),
                            "Background commands finished; waking the agent"
                        );
                        let reported: Vec<String> =
                            finished.iter().map(|c| c.id.clone()).collect();
                        self.registry.mark_reported(&reported).await;
                        ctx.injector.inject(Self::wake_prompt(&finished));
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
    use crate::commands::CommandSpec;
    use std::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    use super::super::scheduled_job::JobInjector;

    struct CapturingInjector(Arc<Mutex<Vec<String>>>);

    impl JobInjector for CapturingInjector {
        fn inject(&self, text: String) {
            self.0.lock().expect("lock").push(text);
        }
    }

    fn registry() -> Arc<CommandRegistry> {
        let dir = std::env::temp_dir().join(format!("bgwatch-{}", uuid::Uuid::new_v4()));
        Arc::new(CommandRegistry::new(Duration::from_mins(1), dir))
    }

    fn context(injected: &Arc<Mutex<Vec<String>>>) -> JobContext {
        JobContext {
            injector: Arc::new(CapturingInjector(Arc::clone(injected))),
            job_cancel: CancellationToken::new(),
            run_cancel: CancellationToken::new(),
        }
    }

    async fn spawn(reg: &CommandRegistry, command: &str) -> String {
        reg.run(CommandSpec {
            command: command.to_string(),
            cwd: None,
            timeout: None,
            label: None,
        })
        .await
        .expect("spawn")
        .id
    }

    async fn settle(reg: &CommandRegistry, id: &str) {
        for _ in 0..100 {
            if reg.get(id).await.expect("entry").state.is_terminal() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("command never finished");
    }

    #[tokio::test]
    async fn arms_nothing_when_no_command_is_running() {
        let reg = registry();
        assert!(BackgroundWatchJob::arm(reg).await.is_none());
    }

    #[tokio::test]
    async fn arms_over_running_commands_and_unreported_finished_ones() {
        let reg = registry();
        spawn(&reg, "sleep 30").await;
        let done = spawn(&reg, "true").await;
        settle(&reg, &done).await;

        let job = BackgroundWatchJob::arm(Arc::clone(&reg))
            .await
            .expect("armed");
        assert_eq!(job.command_count(), 2);
    }

    #[tokio::test]
    async fn arms_over_running_commands_only_once_the_finished_one_was_read() {
        let reg = registry();
        spawn(&reg, "sleep 30").await;
        let done = spawn(&reg, "true").await;
        settle(&reg, &done).await;
        reg.read_for_agent(None).await.expect("agent reads it back");

        let job = BackgroundWatchJob::arm(Arc::clone(&reg))
            .await
            .expect("armed");
        assert_eq!(job.command_count(), 1);
    }

    /// The stranding bug: a command with a bad path dies in under a second, so
    /// it is already terminal when the turn ends and the watch arms. Watching
    /// only what still runs armed nothing, and the agent that ended its turn
    /// expecting a wake was never re-triggered — the sandbox just idled out.
    #[tokio::test]
    async fn wakes_for_a_command_that_died_before_the_watch_armed() {
        let reg = registry();
        let done = spawn(&reg, "./definitely-not-here").await;
        settle(&reg, &done).await;

        let job = BackgroundWatchJob::arm(Arc::clone(&reg))
            .await
            .expect("a finished command the agent never saw must still arm")
            .with_poll_interval(Duration::from_millis(20));

        let injected = Arc::new(Mutex::new(Vec::new()));
        Box::new(job).run(context(&injected)).await;

        let messages = injected.lock().expect("lock");
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains(&done), "prompt was {:?}", messages[0]);
    }

    /// `kill` marks the command reported while the process is still winding
    /// down, so it is still `Running` when the turn ends. Keying the watch off
    /// state rather than `reported` would pick it back up and wake the agent
    /// about the death it had just ordered.
    #[tokio::test]
    async fn does_not_watch_a_killed_command_still_winding_down() {
        let reg = registry();
        let id = spawn(&reg, "sleep 30").await;
        reg.kill(&id).await.expect("kill");
        assert_eq!(
            reg.get(&id).await.expect("entry").state,
            crate::commands::CommandState::Running,
            "this test is only meaningful before the supervisor records the kill"
        );

        assert!(BackgroundWatchJob::arm(Arc::clone(&reg)).await.is_none());
        settle(&reg, &id).await;
        assert!(BackgroundWatchJob::arm(reg).await.is_none());
    }

    /// A `Send` does not cancel an armed watch, so a later turn can kill a
    /// command this watch is still polling. The command is reported at the kill
    /// request; the watch has to notice that rather than waking when the
    /// supervisor later records `Killed`.
    #[tokio::test]
    async fn a_kill_after_the_watch_armed_does_not_wake() {
        let reg = registry();
        let id = spawn(&reg, "sleep 30").await;
        let job = BackgroundWatchJob::arm(Arc::clone(&reg))
            .await
            .expect("armed")
            .with_poll_interval(Duration::from_millis(20));

        let injected = Arc::new(Mutex::new(Vec::new()));
        let handle = tokio::spawn(Box::new(job).run(context(&injected)));
        tokio::time::sleep(Duration::from_millis(60)).await;
        reg.kill(&id).await.expect("kill");

        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("the watch must stop once nothing owes a report")
            .expect("watch task");
        assert!(injected.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn a_wake_reports_its_commands_so_the_next_turn_does_not_repeat_it() {
        let reg = registry();
        let done = spawn(&reg, "exit 9").await;
        settle(&reg, &done).await;

        let job = BackgroundWatchJob::arm(Arc::clone(&reg))
            .await
            .expect("armed")
            .with_poll_interval(Duration::from_millis(20));
        let injected = Arc::new(Mutex::new(Vec::new()));
        Box::new(job).run(context(&injected)).await;

        assert!(
            BackgroundWatchJob::arm(Arc::clone(&reg)).await.is_none(),
            "the wake already told the agent; arming again would loop"
        );
    }

    #[tokio::test]
    async fn wakes_when_a_watched_command_finishes() {
        let reg = registry();
        spawn(&reg, "sleep 0.2").await;
        let job = BackgroundWatchJob::arm(Arc::clone(&reg))
            .await
            .expect("armed")
            .with_poll_interval(Duration::from_millis(20));

        let injected = Arc::new(Mutex::new(Vec::new()));
        Box::new(job).run(context(&injected)).await;

        let messages = injected.lock().expect("lock");
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].contains("succeeded"),
            "prompt was {:?}",
            messages[0]
        );
    }

    #[tokio::test]
    async fn wakes_on_the_first_command_to_finish() {
        let reg = registry();
        spawn(&reg, "sleep 30").await;
        spawn(&reg, "sleep 0.2").await;
        let job = BackgroundWatchJob::arm(Arc::clone(&reg))
            .await
            .expect("armed")
            .with_poll_interval(Duration::from_millis(20));
        assert_eq!(job.command_count(), 2);

        let injected = Arc::new(Mutex::new(Vec::new()));
        Box::new(job).run(context(&injected)).await;

        let messages = injected.lock().expect("lock");
        assert_eq!(
            messages.len(),
            1,
            "a long-lived job must not delay the wake"
        );
    }

    #[tokio::test]
    async fn wake_prompt_names_the_log_and_a_failure() {
        let reg = registry();
        spawn(&reg, "exit 3").await;
        let job = BackgroundWatchJob::arm(Arc::clone(&reg))
            .await
            .expect("armed")
            .with_poll_interval(Duration::from_millis(20));

        let injected = Arc::new(Mutex::new(Vec::new()));
        Box::new(job).run(context(&injected)).await;

        let messages = injected.lock().expect("lock");
        assert!(
            messages[0].contains("failed with exit 3"),
            "{:?}",
            messages[0]
        );
        assert!(messages[0].contains(".log"), "{:?}", messages[0]);
    }

    /// The failure is why the agent is being woken, so the wake carries it.
    #[tokio::test]
    async fn wake_prompt_carries_the_output_of_a_failure() {
        let reg = registry();
        spawn(&reg, ">&2 echo 'boom: no such file'; exit 2").await;
        let job = BackgroundWatchJob::arm(Arc::clone(&reg))
            .await
            .expect("armed")
            .with_poll_interval(Duration::from_millis(20));

        let injected = Arc::new(Mutex::new(Vec::new()));
        Box::new(job).run(context(&injected)).await;

        let messages = injected.lock().expect("lock");
        assert!(
            messages[0].contains("boom: no such file"),
            "prompt was {:?}",
            messages[0]
        );
    }

    #[tokio::test]
    async fn wake_prompt_does_not_quote_the_log_of_a_success() {
        let reg = registry();
        spawn(&reg, "echo 'build ok'").await;
        let job = BackgroundWatchJob::arm(Arc::clone(&reg))
            .await
            .expect("armed")
            .with_poll_interval(Duration::from_millis(20));

        let injected = Arc::new(Mutex::new(Vec::new()));
        Box::new(job).run(context(&injected)).await;

        let messages = injected.lock().expect("lock");
        assert!(messages[0].contains("succeeded"), "{:?}", messages[0]);
        assert!(
            !messages[0].contains("```"),
            "a success needs no log dump: {:?}",
            messages[0]
        );
    }

    #[tokio::test]
    async fn cancellation_stops_the_watch_without_waking() {
        let reg = registry();
        spawn(&reg, "sleep 30").await;
        let job = BackgroundWatchJob::arm(Arc::clone(&reg))
            .await
            .expect("armed")
            .with_poll_interval(Duration::from_millis(20));

        let injected = Arc::new(Mutex::new(Vec::new()));
        let ctx = context(&injected);
        let cancel = ctx.job_cancel.clone();
        let handle = tokio::spawn(Box::new(job).run(ctx));
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
        handle.await.expect("watch task");

        assert!(injected.lock().expect("lock").is_empty());
    }
}
