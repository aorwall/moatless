//! Root supervision for the backend's long-lived tasks.
//!
//! Every server and background worker the process starts is registered with the
//! single [`Supervisor`] built in `main`, together with a declared
//! [`Criticality`]. The supervisor owns the [`JoinHandle`]s, so a task that
//! returns early or panics becomes a structured [`TaskExit`] instead of
//! vanishing into a discarded handle, and the declared policy — not the absence
//! of a handle — decides what that termination means:
//!
//! - [`Criticality::Fatal`] — cancel everything and fail the process.
//! - [`Criticality::Readiness`] — keep serving, but flip `/ready` to
//!   unavailable so Kubernetes sheds traffic to a healthy replica.
//! - [`Criticality::Optional`] — log and carry on.
//!
//! Termination *after* root cancellation is the normal shutdown path and is
//! never treated as a failure. [`Supervisor::run`] drives both phases: it
//! watches until a fatal exit or cancellation, then joins the remaining tasks
//! within a bounded deadline and names the stragglers that outlived it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::task::{JoinError, JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// What the root supervisor does when a supervised task terminates before
/// shutdown was requested.
///
/// Declared at registration so an optional worker's exit policy is visible in
/// the composition root rather than inferred from a dropped handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criticality {
    /// The process cannot do its job without this task. Its early termination
    /// cancels every other supervised task and fails `main`.
    Fatal,
    /// The replica can still serve in-flight work, but it is no longer a
    /// healthy target: `/ready` reports unavailable until the process restarts.
    ///
    /// Chosen over [`Fatal`](Self::Fatal) for workers whose only non-shutdown
    /// exit is a panic, because a panic is usually input-triggered and a
    /// restart re-reads the same input — failing the process would trade one
    /// stopped worker for a crash loop that also takes request serving down.
    Readiness,
    /// Losing this task degrades one non-essential capability. Nothing else
    /// changes.
    Optional,
}

/// Why a supervised task stopped.
#[derive(Debug)]
pub enum ExitCause {
    /// The task's future returned on its own — e.g. a server's `serve` loop
    /// ended, or a worker hit a guard clause and bailed out.
    EarlyReturn,
    /// The task panicked. Carries the panic message.
    Panic(String),
    /// The runtime aborted the task.
    Aborted,
}

impl std::fmt::Display for ExitCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EarlyReturn => f.write_str("returned early"),
            Self::Panic(message) => write!(f, "panicked: {message}"),
            Self::Aborted => f.write_str("was aborted"),
        }
    }
}

/// A supervised task terminated before shutdown was requested.
///
/// Returned by [`Supervisor::run`] for a [`Criticality::Fatal`] task, which
/// makes it `main`'s error and so the process's non-zero exit.
#[derive(Debug, thiserror::Error)]
#[error("supervised task `{name}` terminated before shutdown: {cause}")]
pub struct TaskExit {
    /// Registration name of the task that stopped.
    pub name: &'static str,
    /// How it stopped.
    pub cause: ExitCause,
}

/// Readiness view of supervised work, shared with the `/ready` probe.
///
/// Records the [`Criticality::Readiness`] tasks that terminated early. The set
/// only grows, and nothing here replaces the replica: liveness (`/health`) keeps
/// answering, so Kubernetes will not restart an unready-but-live pod. Leader-
/// elected work (reconciler, reaper, Codex refresher, cron, pod watcher) fails
/// over on its own once the lease expires, but a per-replica consumer stays
/// stopped until an operator replaces the pod. That replacement is deliberately
/// not automatic — see [`Criticality::Readiness`] — so it needs an alert on the
/// `"component": "supervisor"` readiness body or on the task's error log line.
#[derive(Debug, Default)]
pub struct SupervisorHealth {
    failed: Mutex<BTreeSet<&'static str>>,
}

impl SupervisorHealth {
    /// Names of readiness-critical tasks that terminated early, in name order.
    /// Empty means every such task is still running.
    pub fn failed_tasks(&self) -> Vec<&'static str> {
        self.guard().iter().copied().collect()
    }

    fn record(&self, name: &'static str) {
        self.guard().insert(name);
    }

    /// A poisoned lock only means a writer panicked mid-insert; the set is
    /// still a valid readiness view, and reporting *less* failure than reality
    /// is the wrong way to fail.
    fn guard(&self) -> MutexGuard<'_, BTreeSet<&'static str>> {
        self.failed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Supervised {
    name: &'static str,
    criticality: Criticality,
    handle: JoinHandle<()>,
}

/// The root supervisor: owns every long-lived task's join handle and applies
/// its declared [`Criticality`] when it terminates.
pub struct Supervisor {
    cancel: CancellationToken,
    health: Arc<SupervisorHealth>,
    tasks: Vec<Supervised>,
}

impl Supervisor {
    /// Create a supervisor driven by the process-wide cancellation token.
    ///
    /// The same token must be handed to every supervised task so root
    /// cancellation reaches all of them.
    pub fn new(cancel: CancellationToken) -> Self {
        Self {
            cancel,
            health: Arc::new(SupervisorHealth::default()),
            tasks: Vec::new(),
        }
    }

    /// The readiness view to wire into the `/ready` probe.
    pub fn health(&self) -> Arc<SupervisorHealth> {
        Arc::clone(&self.health)
    }

    /// Take ownership of an already-spawned task under a declared policy.
    ///
    /// `name` identifies the task in logs, in the readiness body, and in the
    /// straggler report, so it should match how the task is known operationally.
    pub fn supervise(
        &mut self,
        name: &'static str,
        criticality: Criticality,
        handle: JoinHandle<()>,
    ) {
        info!(task = name, ?criticality, "Supervising background task");
        self.tasks.push(Supervised {
            name,
            criticality,
            handle,
        });
    }

    /// Watch the supervised tasks until one [`Criticality::Fatal`] task
    /// terminates early or the root token is cancelled, then cancel and join
    /// the rest within `join_deadline`.
    ///
    /// Returns the fatal exit that ended the process, if there was one. Tasks
    /// that outlive the deadline are reported by name and left running — the
    /// process exits rather than hanging on them.
    pub async fn run(self, join_deadline: Duration) -> Result<(), TaskExit> {
        let Self {
            cancel,
            health,
            tasks,
        } = self;

        let mut outstanding: BTreeSet<&'static str> = tasks.iter().map(|t| t.name).collect();
        let mut watching: JoinSet<(&'static str, Criticality, Result<(), JoinError>)> =
            JoinSet::new();
        for task in tasks {
            let Supervised {
                name,
                criticality,
                handle,
            } = task;
            watching.spawn(async move { (name, criticality, handle.await) });
        }
        info!(tasks = outstanding.len(), "Root supervisor running");

        let mut fatal: Option<TaskExit> = None;
        while !outstanding.is_empty() && fatal.is_none() && !cancel.is_cancelled() {
            tokio::select! {
                () = cancel.cancelled() => break,
                joined = watching.join_next() => {
                    let Some(joined) = joined else { break };
                    fatal = classify(joined, cancel.is_cancelled(), &health, &mut outstanding);
                }
            }
        }

        // Root cancellation reaches every owned task, whether shutdown was
        // requested by a signal or forced by a fatal exit.
        cancel.cancel();

        let drain = async {
            while let Some(joined) = watching.join_next().await {
                classify(joined, true, &health, &mut outstanding);
            }
        };
        if tokio::time::timeout(join_deadline, drain).await.is_err() {
            warn!(
                stragglers = ?outstanding,
                deadline_secs = join_deadline.as_secs_f64(),
                "Shutdown deadline exceeded; supervised tasks are still running"
            );
        } else {
            info!("All supervised tasks joined");
        }

        match fatal {
            Some(exit) => Err(exit),
            None => Ok(()),
        }
    }
}

/// Apply a terminated task's declared policy, and report it as fatal when that
/// policy says the process must stop.
fn classify(
    joined: Result<(&'static str, Criticality, Result<(), JoinError>), JoinError>,
    shutting_down: bool,
    health: &SupervisorHealth,
    outstanding: &mut BTreeSet<&'static str>,
) -> Option<TaskExit> {
    // The wrapper future only awaits a handle, so it cannot fail on its own —
    // reaching here means the whole supervisor was torn down mid-await.
    let Ok((name, criticality, result)) = joined else {
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

    match criticality {
        Criticality::Fatal => {
            error!(task = name, %cause, "Critical task terminated; shutting the process down");
            Some(TaskExit { name, cause })
        }
        Criticality::Readiness => {
            error!(task = name, %cause, "Required task terminated; reporting not ready");
            health.record(name);
            None
        }
        Criticality::Optional => {
            warn!(task = name, %cause, "Optional task terminated; continuing without it");
            None
        }
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
