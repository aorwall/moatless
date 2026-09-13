use crate::exit::ExitCause;

/// What [`classify`](crate::classify) does after a [`Policy`] has judged a
/// task's early exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// Cancel every other supervised task and fail the process.
    pub fatal: bool,
    /// Record the task in [`SupervisorHealth`](crate::SupervisorHealth) for
    /// the process's own probe.
    pub degraded: bool,
}

/// What a process does when a supervised task terminates before shutdown.
///
/// Each process declares its own criticality enum and implements this trait
/// once per variant, so the logging text stays local to the process while
/// [`classify`](crate::classify) stays policy-agnostic and is the ledger's
/// only writer.
pub trait Policy: Copy + std::fmt::Debug + Send + 'static {
    fn on_early_exit(self, name: &'static str, cause: &ExitCause) -> Outcome;
}
