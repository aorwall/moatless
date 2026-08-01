//! Root supervision for the sandbox's long-lived tasks.
//!
//! Every server, durability loop, and lifecycle task the pod starts is
//! registered with the single [`Supervisor`] built in `main`, together with a
//! declared [`Criticality`]. The supervisor owns the [`JoinHandle`]s, so a task
//! that returns early or panics becomes a structured [`TaskExit`] instead of
//! vanishing into a discarded handle, and the declared policy — not the absence
//! of a handle — decides what that termination means:
//!
//! - [`Criticality::Fatal`] — cancel everything and fail the process. One pod
//!   runs one agent, so a sandbox that cannot do its job helps nobody by
//!   staying alive: exiting hands the pod straight back to the kubelet.
//! - [`Criticality::Liveness`] — keep serving the current turn, but report the
//!   pod as not live on `/livez` so Kubernetes restarts it (see below).
//! - [`Criticality::Optional`] — log and carry on.
//!
//! Termination *after* root cancellation is the normal shutdown path and is
//! never treated as a failure. [`Supervisor::run`] drives both phases: it
//! watches until a fatal exit or cancellation, then joins the remaining tasks
//! within [`SHUTDOWN_JOIN_DEADLINE`] and names the stragglers that outlived it.
//!
//! # Relationship to the backend's supervisor
//!
//! This is a deliberate duplicate of `backend/src/supervisor.rs`, not an
//! extraction: the sandbox does not depend on the backend's crates, and sharing
//! one would couple two independently-deployed binaries for ~250 lines. The
//! vocabulary is kept identical — `Criticality` declared at registration rather
//! than inferred from a dropped handle, the same `ExitCause`/`TaskExit` shape,
//! the same two-phase `run` — so a later extraction is mechanical.
//!
//! Two differences are load-bearing and are why this is not a copy:
//!
//! 1. **`Liveness`, not `Readiness`.** The backend's middle tier flips `/ready`
//!    so a replica sheds traffic to its peers; the sandbox has no peers and
//!    serves no `/ready`. Its `/health` is a *startup* probe only (see
//!    `backend/src/sandbox/k8s/pod_builder.rs`), so flipping it after startup
//!    changes nothing. `/livez` is the sandbox's only post-startup probe, and it
//!    is the one that makes a restart happen — which is the remedy a
//!    single-agent pod actually wants. Note it is opt-in
//!    (`K8S_SANDBOX_LIVENESS_ENABLED`): where a restart must happen regardless
//!    of that setting, the policy is [`Criticality::Fatal`].
//! 2. **Late registration.** The sandbox's durability loops are spawned from
//!    inside `init::run_init`, which is itself a supervised task, so
//!    registration cannot be a `Vec` filled before `run`. [`Registrar`] is a
//!    cloneable handle that feeds the running supervisor.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::{JoinError, JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Bounded time budget for joining every supervised task on shutdown.
///
/// This is the *only* shutdown join in the process and the last bounded step of
/// a teardown budget accounted against the pod's ~30s grace period; see the table on
/// `crate::shutdown::OUTBOX_FLUSH_TIMEOUT` before changing it.
///
/// Two seconds is enough: every task registered here either selects over the
/// lifecycle token or is already finished by the time shutdown runs, so the only
/// thing this waits out is an in-flight request or gRPC call. Stragglers are
/// reported by name, not waited for.
pub const SHUTDOWN_JOIN_DEADLINE: Duration = Duration::from_secs(2);

/// What the root supervisor does when a supervised task terminates before
/// shutdown was requested.
///
/// Declared at registration so a task's exit policy is visible in the
/// composition root rather than inferred from a dropped handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criticality {
    /// The pod cannot do its job without this task. Its early termination
    /// cancels every other supervised task and fails `main`, so the container
    /// exits non-zero and the kubelet restarts it.
    ///
    /// Chosen over [`Liveness`](Self::Liveness) whenever the loss makes the pod
    /// unreachable or unable to make progress at all, because `/livez` probing
    /// is opt-in and a `Liveness` policy is inert when it is off.
    Fatal,
    /// The pod can still finish the turn it is running, but it is no longer
    /// healthy: `/livez` reports unavailable so Kubernetes restarts it (when
    /// sandbox liveness probing is enabled).
    ///
    /// Chosen over [`Fatal`](Self::Fatal) when an immediate process exit would
    /// destroy an in-flight agent turn to fix something the *next* turn needs —
    /// a restart under SIGTERM runs the graceful teardown, an exit does not.
    Liveness,
    /// Losing this task degrades one non-essential capability. Nothing else
    /// changes.
    Optional,
}

/// Whether a supervised task is expected to return on its own before shutdown.
///
/// Split from [`Criticality`] because the two are orthogonal: initialization is
/// fatal *when it panics* and entirely normal when it returns, and folding that
/// into the criticality would mean either treating a completed init as a crash
/// or treating a crashed server as routine.
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
/// Records every non-[`Optional`](Criticality::Optional) task that terminated
/// early. The set only grows: nothing here is restartable, so a pod that has
/// lost one of these stays degraded until it is replaced.
///
/// [`Criticality::Fatal`] exits are recorded too, even though the process is
/// already on its way out. During the bounded shutdown join the HTTP server is
/// still answering probes, and answering them with 200 would claim a liveness
/// the process no longer has.
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
/// Held by `main` and by anything that spawns supervised work after `main` has
/// handed control to [`Supervisor::run`] — in practice `init::run_init`, which
/// starts the durability loops only once the workspace exists.
#[derive(Clone)]
pub struct Registrar {
    tx: mpsc::UnboundedSender<Supervised>,
    health: Arc<SupervisorHealth>,
}

impl Registrar {
    /// Take ownership of an already-spawned task that runs until cancellation.
    ///
    /// `name` identifies the task in logs, in the `/livez` view, and in the
    /// straggler report, so it should match how the task is known operationally.
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

/// The root supervisor: owns every long-lived task's join handle and applies its
/// declared [`Criticality`] when it terminates.
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

    /// A registration handle. Clone it into anything that spawns supervised work.
    pub fn registrar(&self) -> Registrar {
        self.registrar.clone()
    }

    /// The liveness view to wire into the `/livez` probe.
    pub fn health(&self) -> Arc<SupervisorHealth> {
        self.registrar.health()
    }

    /// Watch the supervised tasks until one [`Criticality::Fatal`] task
    /// terminates early or the root token is cancelled, then cancel and join the
    /// rest within `join_deadline`.
    ///
    /// Returns the fatal exit that ended the process, if there was one. Tasks
    /// that outlive the deadline are reported by name and left running — the
    /// process exits rather than hanging on them, which is the whole point of
    /// bounding the join.
    ///
    /// # `teardown`
    ///
    /// The graceful teardown to run when *this supervisor* initiates shutdown —
    /// i.e. on a fatal exit, where no signal handler has run and nothing else
    /// will flush anything. It is awaited **before** the root token is
    /// cancelled, and it is expected to cancel the token itself (in the sandbox
    /// it is [`crate::shutdown::perform_shutdown`], whose last statement does).
    ///
    /// The ordering is not incidental. Teardown flushes the event outbox, and
    /// `EventOutbox::flush` early-returns the moment its drainer has stopped, so
    /// cancelling first would make the flush an instant no-op that drops the
    /// pod's terminal events while looking entirely correct. That is the same
    /// inversion the signal path is careful to avoid, so the two paths run in
    /// the same order: teardown, then cancel, then join.
    ///
    /// It is *not* run when the loop ended because the token was already
    /// cancelled: that is the signal path, and teardown has already happened.
    /// The future is only polled when needed, so passing one that borrows state
    /// costs nothing on the common path.
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
