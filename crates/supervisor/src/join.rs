use std::collections::BTreeSet;
use std::time::Duration;

use tokio::task::{JoinError, JoinHandle, JoinSet};
use tracing::{info, warn};

use crate::exit::{Completion, ExitCause, TaskExit};
use crate::health::SupervisorHealth;
use crate::policy::Policy;

/// A task handed to the supervisor, with its declared policy and completion
/// expectation.
pub struct Supervised<C> {
    pub name: &'static str,
    pub criticality: C,
    pub completion: Completion,
    pub handle: JoinHandle<()>,
}

/// A supervised task's outcome, carried through the `JoinSet` wrapper future.
pub type Joined<C> = (&'static str, C, Completion, Result<(), JoinError>);

/// Move a task into `watching` and record it in `outstanding`.
pub fn watch<C: Policy>(
    task: Supervised<C>,
    watching: &mut JoinSet<Joined<C>>,
    outstanding: &mut BTreeSet<&'static str>,
) {
    let Supervised {
        name,
        criticality,
        completion,
        handle,
    } = task;
    outstanding.insert(name);
    watching.spawn(async move { (name, criticality, completion, handle.await) });
}

/// Apply a terminated task's declared policy, and report it as fatal when that
/// policy says the process must stop.
pub fn classify<C: Policy>(
    joined: Result<Joined<C>, JoinError>,
    shutting_down: bool,
    health: &SupervisorHealth,
    outstanding: &mut BTreeSet<&'static str>,
) -> Option<TaskExit> {
    // The wrapper future only awaits a handle, so it cannot fail on its own —
    // reaching here means the whole supervisor was torn down mid-await.
    let Ok((name, criticality, completion, result)) = joined else {
        warn!("Supervisor lost track of a task while joining");
        return None;
    };
    outstanding.remove(name);

    let cause = match result {
        Ok(()) => ExitCause::EarlyReturn,
        Err(join_error) if join_error.is_panic() => ExitCause::Panic(panic_message(join_error)),
        Err(_) => ExitCause::Aborted,
    };

    if shutting_down {
        match cause {
            ExitCause::EarlyReturn => info!(task = name, "Supervised task stopped"),
            cause => warn!(task = name, %cause, "Supervised task ended abnormally during shutdown"),
        }
        return None;
    }

    // A finite task doing exactly what it was spawned to do is not a failure —
    // only the abnormal endings carry its criticality.
    if matches!(completion, Completion::Finite) && matches!(cause, ExitCause::EarlyReturn) {
        info!(task = name, "Supervised task completed");
        return None;
    }

    let outcome = criticality.on_early_exit(name, &cause);
    if outcome.degraded {
        health.record(name);
    }
    if outcome.fatal {
        Some(TaskExit { name, cause })
    } else {
        None
    }
}

/// Join every task in `watching` within `join_deadline`, applying each one's
/// policy as it completes. Stragglers are reported by name and left running —
/// the caller returns rather than hanging on them.
pub async fn drain<C: Policy>(
    mut watching: JoinSet<Joined<C>>,
    health: &SupervisorHealth,
    mut outstanding: BTreeSet<&'static str>,
    join_deadline: Duration,
) {
    let join_remaining = async {
        while let Some(joined) = watching.join_next().await {
            classify(joined, true, health, &mut outstanding);
        }
    };
    if tokio::time::timeout(join_deadline, join_remaining).await.is_err() {
        warn!(
            stragglers = ?outstanding,
            deadline_secs = join_deadline.as_secs_f64(),
            "Shutdown deadline exceeded; supervised tasks are still running"
        );
    } else {
        info!("All supervised tasks joined");
    }
}

fn panic_message(join_error: JoinError) -> String {
    let payload = join_error.into_panic();
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::Outcome;

    #[derive(Debug, Clone, Copy)]
    struct Fatal;

    impl Policy for Fatal {
        fn on_early_exit(self, _: &'static str, _: &ExitCause) -> Outcome {
            Outcome {
                fatal: true,
                degraded: false,
            }
        }
    }

    fn joined(completion: Completion) -> Joined<Fatal> {
        ("task", Fatal, completion, Ok(()))
    }

    #[test]
    fn a_finite_task_returning_cleanly_is_not_a_failure() {
        let mut outstanding = BTreeSet::from(["task"]);
        let health = SupervisorHealth::default();
        assert!(
            classify(Ok(joined(Completion::Finite)), false, &health, &mut outstanding).is_none()
        );
        assert!(outstanding.is_empty());
    }

    #[test]
    fn a_run_until_shutdown_task_returning_early_applies_the_policy() {
        let mut outstanding = BTreeSet::from(["task"]);
        let health = SupervisorHealth::default();
        let exit = classify(
            Ok(joined(Completion::RunsUntilShutdown)),
            false,
            &health,
            &mut outstanding,
        );
        assert!(exit.is_some());
    }

    #[test]
    fn nothing_is_fatal_once_shutdown_has_started() {
        let mut outstanding = BTreeSet::from(["task"]);
        let health = SupervisorHealth::default();
        assert!(
            classify(
                Ok(joined(Completion::RunsUntilShutdown)),
                true,
                &health,
                &mut outstanding
            )
            .is_none()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_deadline_that_expires_returns_rather_than_hangs() {
        let mut watching: JoinSet<Joined<Fatal>> = JoinSet::new();
        watching.spawn(std::future::pending());
        let outstanding = BTreeSet::from(["straggler"]);
        let health = SupervisorHealth::default();

        // The spawned task never resolves, so this only returns if the
        // deadline timeout fires instead of `drain` waiting on it forever.
        // Paused time auto-advances to the deadline since nothing else here
        // makes progress.
        drain(watching, &health, outstanding, Duration::from_millis(50)).await;
    }
}
