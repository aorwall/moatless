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
//! running alongside a long-lived server still reports the moment it is done.

use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info};

use crate::commands::{CommandRegistry, CommandState, ManagedCommand};

use super::scheduled_job::{
    BackgroundWatchSnapshot, JobContext, JobKind, JobSnapshot, ScheduledJob,
};

/// How often the watcher re-checks the registry.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// How many command labels to keep in the status snapshot / wake prompt.
const MAX_COMMAND_PREVIEWS: usize = 5;

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
    /// Arm a watch over every command still running. `None` when there is none,
    /// so a turn that left no work behind registers nothing.
    pub async fn arm(registry: Arc<CommandRegistry>) -> Option<Self> {
        let running: Vec<ManagedCommand> = registry
            .list()
            .await
            .into_iter()
            .filter(|c| !c.state.is_terminal())
            .collect();
        if running.is_empty() {
            return None;
        }
        Some(Self {
            id: format!("bgwatch-{}", uuid::Uuid::new_v4()),
            registered_at_unix_ms: unix_now_ms(),
            watched_ids: running.iter().map(|c| c.id.clone()).collect(),
            commands: running
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

    /// Message injected once a watched command finishes.
    fn wake_prompt(finished: &[ManagedCommand]) -> String {
        let lines: Vec<String> = finished
            .iter()
            .map(|c| {
                let outcome = match (c.state, c.exit_code) {
                    (CommandState::TimedOut, _) => {
                        format!("timed out after {}s", c.timeout.as_secs())
                    }
                    (CommandState::Killed, _) => "killed".to_string(),
                    (_, Some(0)) => "succeeded".to_string(),
                    (_, Some(code)) => format!("failed with exit {code}"),
                    (_, None) => "ended without an exit code".to_string(),
                };
                format!(
                    "- `{}` ({}) {} — output: {}",
                    c.id,
                    c.label,
                    outcome,
                    c.log_path.display()
                )
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
                    let finished = self.registry.finished(&self.watched_ids).await;
                    if !finished.is_empty() {
                        info!(
                            id = %self.id,
                            count = finished.len(),
                            "Background commands finished; waking the agent"
                        );
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

    #[tokio::test]
    async fn arms_nothing_when_no_command_is_running() {
        let reg = registry();
        assert!(BackgroundWatchJob::arm(reg).await.is_none());
    }

    #[tokio::test]
    async fn arms_over_running_commands_only() {
        let reg = registry();
        spawn(&reg, "sleep 30").await;
        let done = spawn(&reg, "true").await;
        // Let the finished one settle into a terminal state.
        for _ in 0..100 {
            if reg.get(&done).await.expect("entry").state.is_terminal() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let job = BackgroundWatchJob::arm(Arc::clone(&reg))
            .await
            .expect("armed");
        assert_eq!(job.command_count(), 1);
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
        assert_eq!(messages.len(), 1, "a long-lived job must not delay the wake");
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
        assert!(messages[0].contains("failed with exit 3"), "{:?}", messages[0]);
        assert!(messages[0].contains(".log"), "{:?}", messages[0]);
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
