//! Root supervision for the sandbox's long-lived tasks: every server,
//! durability loop and lifecycle task registers here under a declared
//! [`Criticality`].
//!
//! `Fatal` cancels everything and fails the process, handing the pod back to
//! the kubelet; `Liveness` finishes the turn and reports not-live on `/livez`
//! (opt-in via `K8S_SANDBOX_LIVENESS_ENABLED`); `Optional` logs and carries on.
//! Only the policy and the channel-based registration front-end are local; the
//! rest comes from `moatless_supervisor`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

pub use moatless_supervisor::SupervisorHealth;
use moatless_supervisor::{
    Completion, ExitCause, Joined, Outcome, TaskExit, classify, drain, watch,
};
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};
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

impl moatless_supervisor::Policy for Criticality {
    /// [`Fatal`](Self::Fatal) is `degraded` too, even though the process is
    /// already on its way out: during the bounded shutdown join the HTTP
    /// server is still answering probes, and answering them with 200 would
    /// claim a liveness the process no longer has.
    fn on_early_exit(self, name: &'static str, cause: &ExitCause) -> Outcome {
        match self {
            Self::Fatal => {
                error!(task = name, %cause, "Critical task terminated; shutting the process down");
                Outcome {
                    fatal: true,
                    degraded: true,
                }
            }
            Self::Liveness => {
                error!(task = name, %cause, "Required task terminated; reporting the pod as not live");
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
        let mut watching: JoinSet<Joined<Criticality>> = JoinSet::new();
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

        drain(watching, &health, outstanding, join_deadline).await;

        match fatal {
            Some(exit) => Err(exit),
            None => Ok(()),
        }
    }
}
