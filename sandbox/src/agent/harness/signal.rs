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

use std::collections::HashMap;

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
/// harness implementation, which is fixed at construction (the fixed-harness decision).
///
/// Credential (`env`) and `system_prompt` resolution is the coordinator's job,
/// and both can change between Turns (credential rotation, prompt refresh), so
/// they ride on each Input rather than being baked into the construction-fixed
/// harness config.
#[derive(Debug, Clone)]
pub struct Input {
    pub id: InputId,
    pub text: String,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub resume: Option<ResumeSession>,
    /// Begin a new Session instead of resuming or injecting into a running one.
    pub start_fresh: bool,
    /// Resolved subprocess environment for this Turn (credentials the coordinator
    /// refreshed before submit).
    pub env: Option<HashMap<String, String>>,
    /// Resolved append-system-prompt for this Turn.
    pub system_prompt: Option<String>,
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
            env: None,
            system_prompt: None,
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

/// What the harness reported about its own context window.
///
/// Normalized here so neither the backend nor either client learns the
/// difference between the two wires: Claude reports a per-model `contextWindow`
/// on its terminal result, Codex a `modelContextWindow` on a token-usage
/// notification. `max_tokens` is `None` where the harness reported none, and the
/// meter then shows a count rather than a percentage of a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextWindow {
    pub used_tokens: i64,
    pub max_tokens: Option<i64>,
}

/// A normalized signal emitted by the harness. This is the entire external
/// output vocabulary; the coordinator reacts only to these.
// reason: the harness seam deliberately carries one canonical `SourceEvent` by
// value throughout (`ProcessEvent::Block` and the driver's early-buffer `Vec` do
// the same); boxing only the `Source` variant would be inconsistent and add an
// allocation per content block on the hot path.
#[allow(clippy::large_enum_variant)]
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
    /// What the harness reported about its own context window.
    ContextWindow(ContextWindow),
    /// The single terminal outcome of a started execution. Exactly one is
    /// emitted per [`Signal::ExecutionStarted`].
    ExecutionFinished { outcome: Outcome },
    /// A submitted input that never opened an execution.
    InputRejected { input: InputId, reason: Rejection },
}
