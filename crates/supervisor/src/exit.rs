use std::fmt;

/// Whether a supervised task is expected to return on its own before shutdown.
///
/// Split from the process's own criticality because the two are orthogonal:
/// initialization is fatal *when it panics* and entirely normal when it
/// returns, and folding that into the criticality would mean either treating a
/// completed init as a crash or treating a crashed server as routine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    /// The task runs until cancellation. Any termination before shutdown —
    /// including a clean `return` — applies the declared criticality.
    RunsUntilShutdown,
    /// The task has a finite job. A clean return is success; only a panic or
    /// an abort applies the declared criticality.
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

impl fmt::Display for ExitCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EarlyReturn => f.write_str("returned early"),
            Self::Panic(message) => write!(f, "panicked: {message}"),
            Self::Aborted => f.write_str("was aborted"),
        }
    }
}

/// A supervised task terminated before shutdown was requested.
///
/// Returned by a process's `Supervisor::run` for a task whose [`Policy`](crate::Policy)
/// reports the exit as fatal, which makes it `main`'s error and so the
/// process's non-zero exit.
#[derive(Debug, thiserror::Error)]
#[error("supervised task `{name}` terminated before shutdown: {cause}")]
pub struct TaskExit {
    /// Registration name of the task that stopped.
    pub name: &'static str,
    /// How it stopped.
    pub cause: ExitCause,
}
