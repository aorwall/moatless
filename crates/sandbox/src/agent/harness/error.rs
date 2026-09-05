//! Stable harness error facts.
//!
//! The external seam classifies failures with a stable [`ErrorClass`] and states
//! [`SessionImpact`] separately from the class. It never exposes a `retryable`
//! policy boolean — the coordinator maps these facts to retry policy (the harness-outcome boundary).

use std::fmt;

/// The lifecycle stage at which a harness failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureStage {
    /// Before the Agent process was ready and its Session identity was known.
    Startup,
    /// While delivering submitted input to the Agent process.
    Input,
    /// After execution started, while producing output.
    Execution,
    /// While terminating the Agent process.
    Termination,
}

/// Stable, provider-independent failure classification.
///
/// These are the provider-independent classes exposed at the external seam, so
/// the coordinator's retry policy never branches on Claude- or Codex-specific
/// errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Credentials were rejected or could not be refreshed.
    Authentication,
    /// The provider rate-limited the request.
    RateLimit,
    /// The provider could not be reached at all: DNS resolution, TCP connect, or
    /// TLS failed before any request was served. Distinct from [`Self::RateLimit`]
    /// (the provider answered) and from [`Self::InvalidInput`] (the request was
    /// understood and rejected), because the sandbox's own egress is the suspect
    /// and the condition is usually transient.
    Connectivity,
    /// A requested Session could not be resumed or was not found.
    UnavailableSession,
    /// Model, mode, or launch configuration was invalid.
    InvalidConfiguration,
    /// The submitted input was malformed or unsupported.
    InvalidInput,
    /// Native protocol output could not be parsed.
    NativeProtocol,
    /// The Agent process exited unexpectedly (includes unexpected EOF).
    ProcessExit,
    /// The Agent process stopped responding.
    UnresponsiveProcess,
    /// A harness invariant was violated (a bug, not a provider condition).
    Internal,
}

/// Whether the CLI Session survived a failure.
///
/// Stated separately from [`ErrorClass`] so the coordinator can decide whether
/// resume is still possible without inspecting the class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionImpact {
    /// The Session is still usable for resume.
    Intact,
    /// The Session is gone; a fresh Session is required to continue.
    Lost,
    /// The Session state is unknown after the failure.
    Unknown,
}

/// A classified harness failure: stage, stable class, Session impact, and a
/// bounded diagnostic string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessError {
    pub stage: FailureStage,
    pub class: ErrorClass,
    pub session_impact: SessionImpact,
    pub diagnostic: String,
}

impl HarnessError {
    pub fn new(
        stage: FailureStage,
        class: ErrorClass,
        session_impact: SessionImpact,
        diagnostic: impl Into<String>,
    ) -> Self {
        Self {
            stage,
            class,
            session_impact,
            diagnostic: diagnostic.into(),
        }
    }
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?}/{:?} (session: {:?}): {}",
            self.stage, self.class, self.session_impact, self.diagnostic
        )
    }
}

impl std::error::Error for HarnessError {}
