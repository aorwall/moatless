//! Root supervision for the backend's long-lived tasks.
//!
//! Every server and background worker is registered with the single
//! [`Supervisor`] built in `main` under a declared [`Criticality`].
//!
//! The supervisor owns each [`JoinHandle`], so a task that returns early or
//! panics becomes a structured [`TaskExit`] instead of vanishing into a
//! discarded handle. Only the policy and the registration front-end are
//! local; the rest comes from `moatless_supervisor`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use moatless_supervisor::{Completion, Joined, Outcome, classify, drain, watch};
pub use moatless_supervisor::{ExitCause, SupervisorHealth, TaskExit};
use tokio::task::{JoinHandle, JoinSet};
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
    /// The replica can still serve in-flight work, but `/ready` reports
    /// unavailable until it restarts; `/health` keeps answering, so Kubernetes
    /// leaves restarting an unready-but-live pod to an operator.
    ///
    /// Chosen over [`Fatal`](Self::Fatal): failing on a panic would trade one
    /// stopped worker for a crash loop that also takes serving down.
    Readiness,
    /// Losing this task degrades one non-essential capability. Nothing else
    /// changes.
    Optional,
}

impl moatless_supervisor::Policy for Criticality {
    fn on_early_exit(self, name: &'static str, cause: &ExitCause) -> Outcome {
        match self {
            Self::Fatal => {
                error!(task = name, %cause, "Critical task terminated; shutting the process down");
                Outcome {
                    fatal: true,
                    degraded: false,
                }
            }
            Self::Readiness => {
                error!(task = name, %cause, "Required task terminated; reporting not ready");
                Outcome {
                    fatal: false,
                    degraded: true,
                }
            }
            Self::Optional => {
                warn!(task = name, %cause, "Optional task terminated; continuing without it");
                Outcome {
                    fatal: false,
                    degraded: false,
                }
            }
        }
    }
}

type Supervised = moatless_supervisor::Supervised<Criticality>;

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
    /// straggler report, so it should match how the task is known
    /// operationally.
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
            completion: Completion::RunsUntilShutdown,
            handle,
        });
    }

    /// Watch the supervised tasks until one [`Criticality::Fatal`] task
    /// terminates early or the root token is cancelled, then cancel and join
    /// the rest within `join_deadline`.
    ///
    /// Returns the fatal exit that ended the process, if there was one; a task
    /// that outlives the deadline is reported by name and left running rather
    /// than hung on.
    pub async fn run(self, join_deadline: Duration) -> Result<(), TaskExit> {
        let Self {
            cancel,
            health,
            tasks,
        } = self;

        let mut outstanding: BTreeSet<&'static str> = BTreeSet::new();
        let mut watching: JoinSet<Joined<Criticality>> = JoinSet::new();
        for task in tasks {
            watch(task, &mut watching, &mut outstanding);
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

        drain(watching, &health, outstanding, join_deadline).await;

        match fatal {
            Some(exit) => Err(exit),
            None => Ok(()),
        }
    }
}
