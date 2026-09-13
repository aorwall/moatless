//! Root supervision for the sandbox's long-lived tasks: every server,
//! durability loop and lifecycle task registers here under a declared
//! [`Criticality`].
//!
//! `Fatal` cancels everything and fails the process, handing the pod back to
//! the kubelet; `Liveness` finishes the turn and reports not-live on `/livez`
//! (opt-in via `K8S_SANDBOX_LIVENESS_ENABLED`); `Optional` logs and carries on.
//! [`Registrar`] is cloneable: durability loops spawn from a supervised task.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::{JoinError, JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Bounded time budget for joining every supervised task on shutdown.
///
/// The last bounded step of the pod's ~30s teardown grace period — see
/// `crate::shutdown::OUTBOX_FLUSH_TIMEOUT` before changing it; stragglers past
/// the deadline are reported by name, not waited for.
pub const SHUTDOWN_JOIN_DEADLINE: Duration = Duration::from_secs(2);

/// What the root supervisor does when a supervised task terminates before
/// shutdown was requested. Declared at registration, not inferred from a
/// dropped handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criticality {
    /// The pod cannot do its job without this task. Its early termination
    /// cancels every other supervised task and fails `main`, so the container
    /// exits non-zero and the kubelet restarts it.
    Fatal,
    /// The pod can still finish the turn it is running, but it is no longer
    /// healthy: `/livez` reports unavailable so Kubernetes restarts it (when
    /// sandbox liveness probing is enabled).
    Liveness,
    /// Losing this task degrades one non-essential capability. Nothing else
    /// changes.
    Optional,
}

/// Whether a supervised task is expected to return on its own before shutdown.
/// Orthogonal to [`Criticality`]: initialization is fatal only when it panics,
/// while a long-running server treats even a clean return as failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Completion {
    /// The task runs until cancellation. Any termination before shutdown —
    /// including a clean `return` — applies the declared [`Criticality`].
    RunsUntilShutdown,
    /// The task has a finite job. A clean return is success; only a panic or an
    /// abort applies the declared [`Criticality`].
    Finite,
}

/// Why a supervised task stopped.
#[derive(Debug)]
pub enum ExitCause {
    /// The task's future returned on its own — e.g. a server's `serve` loop
    /// ended, or a loop hit a guard clause and bailed out.
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

/// Liveness view of supervised work, read by the `/livez` probe.
///
/// The set only grows: nothing here is restartable, so a pod that lost one
/// stays degraded until replaced; [`Criticality::Fatal`] exits are recorded
/// too, since `/livez` is still served during the bounded shutdown join.
#[derive(Debug, Default)]
pub struct SupervisorHealth {
    failed: Mutex<BTreeSet<&'static str>>,
}

impl SupervisorHealth {
    /// Names of supervised tasks that terminated early, in name order. Empty
    /// means every registered task is still running (or stopped at shutdown).
    pub fn failed_tasks(&self) -> Vec<&'static str> {
        self.guard().iter().copied().collect()
    }

    fn record(&self, name: &'static str) {
        self.guard().insert(name);
    }

    /// A poisoned lock only means a writer panicked mid-insert; the set is
    /// still a valid liveness view, and reporting *less* failure than reality
    /// is the wrong way to fail.
    fn guard(&self) -> MutexGuard<'_, BTreeSet<&'static str>> {
        self.failed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Supervised {
    name: &'static str,
    criticality: Criticality,
    completion: Completion,
    handle: JoinHandle<()>,
}

/// Cloneable registration handle for the root supervisor.
///
/// Held by `main`, and by `init::run_init`, which starts the durability loops
/// once the workspace exists — after `main` hands off to [`Supervisor::run`].
#[derive(Clone)]
pub struct Registrar {
    tx: mpsc::UnboundedSender<Supervised>,
    health: Arc<SupervisorHealth>,
}

impl Registrar {
    /// Take ownership of an already-spawned task that runs until cancellation.
    ///
    /// `name` identifies the task in logs, in the `/livez` view, and in the
    /// straggler report, so it should match how the task is known
    /// operationally.
    pub fn supervise(&self, name: &'static str, criticality: Criticality, handle: JoinHandle<()>) {
        self.register(name, criticality, Completion::RunsUntilShutdown, handle);
    }

    /// Take ownership of an already-spawned task with a finite job, whose clean
    /// return is success. Only a panic or abort applies `criticality`.
    pub fn supervise_finite(
        &self,
        name: &'static str,
        criticality: Criticality,
        handle: JoinHandle<()>,
    ) {
        self.register(name, criticality, Completion::Finite, handle);
    }

    /// The liveness view, for the `/livez` probe.
    pub fn health(&self) -> Arc<SupervisorHealth> {
        Arc::clone(&self.health)
    }

    fn register(
        &self,
        name: &'static str,
        criticality: Criticality,
        completion: Completion,
        handle: JoinHandle<()>,
    ) {
        info!(task = name, ?criticality, ?completion, "Supervising task");
        if self
            .tx
            .send(Supervised {
                name,
                criticality,
                completion,
                handle,
            })
            .is_err()
        {
            // The supervisor has finished its drain, which only happens once the
            // process is exiting. The task stays detached for the moments it has
            // left; say so rather than pretending it is owned.
            warn!(
                task = name,
                "Root supervisor is gone; task left unsupervised"
            );
        }
    }
}

/// The root supervisor: owns every long-lived task's join handle and applies
/// its declared [`Criticality`] when it terminates.
pub struct Supervisor {
    cancel: CancellationToken,
    registrar: Registrar,
    rx: mpsc::UnboundedReceiver<Supervised>,
}

impl Supervisor {
    /// Create a supervisor driven by the process-wide lifecycle token.
    ///
    /// The same token must be handed to every supervised task so root
    /// cancellation reaches all of them.
    pub fn new(cancel: CancellationToken) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            cancel,
            registrar: Registrar {
                tx,
                health: Arc::new(SupervisorHealth::default()),
            },
            rx,
        }
    }

    /// A registration handle. Clone it into anything that spawns supervised
    /// work.
    pub fn registrar(&self) -> Registrar {
        self.registrar.clone()
    }

    /// The liveness view to wire into the `/livez` probe.
    pub fn health(&self) -> Arc<SupervisorHealth> {
        self.registrar.health()
    }

    /// Watch every task until a [`Criticality::Fatal`] exit or cancellation,
    /// join the rest within `join_deadline`, and return any fatal exit.
    ///
    /// `teardown` runs once, before cancelling, only on a fatal exit — running
    /// it after would let `EventOutbox::flush` no-op and drop terminal events.
    pub async fn run(
        self,
        join_deadline: Duration,
        teardown: impl std::future::Future<Output = ()>,
    ) -> Result<(), TaskExit> {
        let Self {
            cancel,
            registrar,
            mut rx,
        } = self;
        let health = registrar.health();
        // Keep `registrar` alive for the whole run so the registration channel
        // never closes under a late registrar clone.

        let mut outstanding: BTreeSet<&'static str> = BTreeSet::new();
        let mut watching: JoinSet<Joined> = JoinSet::new();
        let mut fatal: Option<TaskExit> = None;

        info!("Root supervisor running");
        while fatal.is_none() && !cancel.is_cancelled() {
            tokio::select! {
                () = cancel.cancelled() => break,
                Some(task) = rx.recv() => watch(task, &mut watching, &mut outstanding),
                Some(joined) = watching.join_next(), if !watching.is_empty() => {
                    fatal = classify(joined, cancel.is_cancelled(), &health, &mut outstanding);
                }
            }
        }

        // A fatal exit means *we* are initiating shutdown, so no signal handler
        // has run and nothing has flushed anything. Run the graceful teardown
        // here, before cancelling — see this method's docs for why the order is
        // load-bearing rather than cosmetic. `teardown` cancels the token
        // itself, which makes the `cancel()` below the same idempotent no-op it
        // already is on the signal path.
        //
        // Skipped when the token is already cancelled: that is the signal path
        // (or a signal that raced this fatal exit), where teardown has run or is
        // running, and a second concurrent teardown would only duplicate work.
        if fatal.is_some() && !cancel.is_cancelled() {
            info!("Fatal task exit; running graceful teardown before cancelling");
            teardown.await;
        }

        // Root cancellation reaches every owned task, whether shutdown was
        // requested by a signal or forced by a fatal exit.
        cancel.cancel();

        // A task registered in the instant before cancellation is still owned:
        // take it too rather than leaving one detached handle behind.
        while let Ok(task) = rx.try_recv() {
            watch(task, &mut watching, &mut outstanding);
        }

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

type Joined = (&'static str, Criticality, Completion, Result<(), JoinError>);

fn watch(
    task: Supervised,
    watching: &mut JoinSet<Joined>,
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
fn classify(
    joined: Result<Joined, JoinError>,
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

    match criticality {
        Criticality::Fatal => {
            error!(task = name, %cause, "Critical task terminated; shutting the process down");
            health.record(name);
            Some(TaskExit { name, cause })
        }
        Criticality::Liveness => {
            error!(task = name, %cause, "Required task terminated; reporting the pod as not live");
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
