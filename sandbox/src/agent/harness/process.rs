//! Private Agent-process port.
//!
//! This is an internal seam, not a public harness trait: production Claude and
//! Codex adapters (tickets 05/07) and the scripted mock (this ticket) implement
//! it, and the shared ordering driver consumes it. Keeping it `pub(crate)`
//! prevents a public universal harness trait from forming — callers receive the
//! concrete [`super::Harness`], never a process handle.

use super::error::HarnessError;
use super::signal::{Activity, Input, InputId, SessionId};
use super::source::SourceEvent;

/// How an Agent process ended on its own.
///
/// Cancellation is not represented here: the driver requests cancellation via
/// [`AgentProcess::cancel`] and owns mapping the resulting termination to a
/// cancelled outcome, so a process never reports "cancelled" itself.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProcessTermination {
    /// The process finished its work normally.
    Completed,
    /// The process failed with a classified error.
    Failed(HarnessError),
}

/// A raw event from an Agent process, before the ordering driver applies the
/// external contract (execution-start gating, early-output buffering, and the
/// exactly-one-terminal guarantee).
// reason: the process/signal seam deliberately carries one canonical `SourceEvent`
// by value throughout (`Signal::Source` and the driver's early-buffer `Vec` do the
// same); boxing only the `Block` variant would be inconsistent and add an
// allocation per content block on the hot path.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProcessEvent {
    /// The process is ready and its real Session identity is known. Reports an
    /// execution opened by the most recently submitted input.
    SessionReady { session: SessionId },
    /// A harness-internal execution began (a CLI wakeup or Monitor): a known
    /// Session with no pending input behind it, so no `InputAccepted` follows.
    /// Persistent transports only.
    InternalStart { session: SessionId },
    /// The process accepted a unit of input. When it follows a `SessionReady`
    /// with no intervening acceptance it opens that execution; when it arrives
    /// while an execution is already active it is injected into that execution.
    InputAccepted { input: InputId },
    /// One native content block.
    Block(SourceEvent),
    /// Liveness / tool progress.
    Activity(Activity),
    /// The current execution reached its single terminal while the process stays
    /// alive to host later executions. Persistent transports only; a one-shot
    /// process reports `Terminated` at the end of its single execution instead.
    ExecutionEnded { termination: ProcessTermination },
    /// The process ended on its own.
    Terminated { termination: ProcessTermination },
}

/// One Agent process implementation.
#[async_trait::async_trait]
pub(crate) trait AgentProcess: Send {
    /// Deliver input to the process. An error is a pre-start / input failure the
    /// driver surfaces as `InputRejected` (before execution start) or a failed
    /// terminal (after execution start).
    async fn submit(&mut self, input: Input) -> Result<(), HarnessError>;

    /// Whether input submitted while an execution is already active is injected
    /// into that execution (reported as a bare `InputAccepted`) rather than
    /// having to wait for a later one.
    ///
    /// Consulted only by the one-execution-per-process driver, which otherwise
    /// queues mid-execution input. A process that opens a fresh execution per
    /// submit must leave this `false` so its input is queued instead of
    /// clobbering the running execution.
    fn accepts_injection(&self) -> bool {
        false
    }

    /// Request cancellation of the process tree. Idempotent; the process is
    /// expected to terminate afterwards.
    async fn cancel(&mut self);

    /// Await the next raw event, or `None` when the process stream is closed
    /// (EOF) — an EOF with no prior `Terminated` is an unexpected exit.
    async fn next(&mut self) -> Option<ProcessEvent>;
}
