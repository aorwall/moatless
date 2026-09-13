//! Private Agent-process port.
//!
//! This is an internal seam, not a public harness trait: production Claude and
//! Codex adapters (tickets 05/07) and the scripted mock (this ticket) implement
//! it, and the shared ordering driver consumes it. Keeping it `pub(crate)`
//! prevents a public universal harness trait from forming — callers receive the
//! concrete [`super::Harness`], never a process handle.

use tokio_util::sync::CancellationToken;

use super::error::HarnessError;
use super::signal::{Activity, ContextWindow, Input, InputId, RunTotals, SessionId};
use super::source::SourceEvent;

/// How an Agent process ended on its own.
///
/// Cancellation is not represented here: the driver requests it via
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
    /// The process is ready and its real Session identity is known; reports an
    /// execution opened by the most recently submitted input.
    ///
    /// `agent_turn_id` is the harness's native turn id when known at
    /// announcement (Codex's `turn/start` id, which a fork later cuts the
    /// thread on); Claude has none and reports `None`.
    SessionReady {
        session: SessionId,
        agent_turn_id: Option<String>,
    },
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
    /// What the harness reported about its own context window.
    ContextWindow(ContextWindow),
    /// The completed run's authoritative token/cost totals, emitted just before
    /// the terminal. The driver captures the most recent one and attaches it to
    /// the `Completed` outcome, so it never rides a `Failed` or `Cancelled`
    /// end.
    RunTotals(RunTotals),
    /// The current execution reached its single terminal while the process
    /// stays alive to host later executions. Persistent transports only; a
    /// one-shot process reports `Terminated` at the end of its single execution
    /// instead.
    ExecutionEnded { termination: ProcessTermination },
    /// The process ended on its own.
    Terminated { termination: ProcessTermination },
}

/// One Agent process implementation.
#[async_trait::async_trait]
pub(crate) trait AgentProcess: Send {
    /// Deliver input to the process. An error is a pre-start / input failure
    /// the driver surfaces as `InputRejected` (before execution start) or a
    /// failed terminal (after execution start).
    async fn submit(&mut self, input: Input) -> Result<(), HarnessError>;

    /// Whether input submitted while an execution is already active is injected
    /// into that execution (as a bare `InputAccepted`) rather than waiting for
    /// a later one.
    ///
    /// Consulted only by the one-execution-per-process driver; a process that
    /// opens a fresh execution per submit must leave this `false` so input
    /// queues instead of clobbering the running execution.
    fn accepts_injection(&self) -> bool {
        false
    }

    /// Request cancellation of the process tree. Idempotent; the process is
    /// expected to terminate afterwards.
    async fn cancel(&mut self);

    /// A handle that cancels the process tree without borrowing it.
    ///
    /// [`AgentProcess::cancel`] can't be reached while [`AgentProcess::submit`]
    /// still holds the process mutably, so a transport whose open blocks on the
    /// Agent (like Codex) must override this or a Stop cannot land until the
    /// open finishes; the default token nobody else holds does nothing, which
    /// is correct only when `submit` cannot block — the driver still records
    /// the Stop and applies it once `submit` lands.
    fn cancel_handle(&self) -> CancellationToken {
        CancellationToken::new()
    }

    /// Await the next raw event, or `None` when the process stream is closed
    /// (EOF) — an EOF with no prior `Terminated` is an unexpected exit.
    async fn next(&mut self) -> Option<ProcessEvent>;
}
