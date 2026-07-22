//! The command/signal vocabulary at the external harness seam.
//!
//! The coordinator submits [`Input`], cancels the active execution, and consumes
//! an ordered stream of [`Signal`]s. The normative ordering (from the design
//! prototype) is:
//!
//! ```text
//! Input opening a new execution:
//!   submit(input)
//!   → ExecutionStarted(session, cause=input)
//!   → InputAccepted(input_id)
//!   → Source*
//!   → ExecutionFinished(outcome)
//!
//! Input joining an active compatible execution:
//!   submit(input)
//!   → InputAccepted(input_id)
//!   → Source*                         (no new ExecutionStarted)
//!
//! Harness-internal execution:
//!   ExecutionStarted(session, cause=harness-internal)
//!   → Source*
//!   → ExecutionFinished(outcome)
//! ```

use crate::agent::types::ResumeSession;

use super::error::HarnessError;
use super::source::SourceEvent;

/// Correlation identity for a submitted input (a pending-Message batch). The
/// coordinator uses it to acknowledge the batch once it is accepted or rejected.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InputId(pub String);

/// Resumable CLI conversation identity, owned by the harness and reported to the
/// coordinator on [`Signal::ExecutionStarted`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

/// Input submitted to the harness. Turn configuration (model, mode, resume,
/// fresh start) rides along, but it configures the Turn — it never selects the
/// harness implementation, which is fixed at construction (ADR-0004).
#[derive(Debug, Clone)]
pub struct Input {
    pub id: InputId,
    pub text: String,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub resume: Option<ResumeSession>,
    /// Begin a new Session instead of resuming or injecting into a running one.
    pub start_fresh: bool,
}

impl Input {
    /// Build a plain text input with no Turn configuration overrides.
    pub fn text(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: InputId(id.into()),
            text: text.into(),
            model: None,
            mode: None,
            resume: None,
            start_fresh: false,
        }
    }
}

/// Why an execution started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// Opened by submitted input; carries the correlation identity.
    Input(InputId),
    /// Started by the harness itself (a wakeup or Monitor), with no pending
    /// Message behind it — so no [`Signal::InputAccepted`] follows.
    Internal,
}

/// The single terminal outcome of a started execution.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The execution completed normally.
    Completed,
    /// The execution was cancelled.
    Cancelled,
    /// The execution failed with a classified error.
    Failed(HarnessError),
}

/// Why a submitted input never opened an execution.
///
/// A pre-start rejection is coordinator-safe: because no [`Signal::ExecutionStarted`]
/// was emitted, the coordinator never minted a Run, so there is no Run to close.
#[derive(Debug, Clone, PartialEq)]
pub enum Rejection {
    /// The input was cancelled before its execution started.
    Cancelled,
    /// The input failed before its execution started, with a classified error.
    Failed(HarnessError),
}

/// Liveness / tool-progress activity. Keeps an active Turn visibly alive without
/// becoming Message content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub session: SessionId,
    pub detail: String,
}

/// A normalized signal emitted by the harness. This is the entire external
/// output vocabulary; the coordinator reacts only to these.
#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    /// An Agent process is ready and its Session identity is known. The
    /// coordinator mints the Run when it receives this. Emitted at most once per
    /// execution and never after a source event.
    ExecutionStarted { session: SessionId, cause: Cause },
    /// The Agent process accepted a submitted input. The coordinator
    /// acknowledges the pending Message only after this, against the current Run.
    InputAccepted { input: InputId },
    /// One native content block, in native order.
    Source(SourceEvent),
    /// Liveness / tool progress; never Message content.
    Activity(Activity),
    /// The single terminal outcome of a started execution. Exactly one is
    /// emitted per [`Signal::ExecutionStarted`].
    ExecutionFinished { outcome: Outcome },
    /// A submitted input that never opened an execution.
    InputRejected { input: InputId, reason: Rejection },
}
