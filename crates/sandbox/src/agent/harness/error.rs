//! Stable harness error facts.
//!
//! The external seam classifies failures with a stable [`ErrorClass`] and
//! states [`SessionImpact`] separately from the class. It never exposes a
//! `retryable` policy boolean — the coordinator maps these facts to retry
//! policy (the harness-outcome boundary).

use std::fmt;

use chrono::{DateTime, SecondsFormat, Utc};

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
/// Exposed at the external seam so the coordinator's retry policy never
/// branches on Claude- or Codex-specific errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Credentials were rejected or could not be refreshed.
    Authentication,
    /// The provider rate-limited the request.
    RateLimit,
    /// The account's usage allowance is spent until a reset hours or days
    /// away. Distinct from [`Self::RateLimit`]: retrying inside that window
    /// only repeats the refusal.
    UsageLimit,
    /// A 429 naming no cause: a transient throttle, or a hub's cooldown for a
    /// pool whose every account is spent, which nothing here tells apart.
    /// Retried like [`Self::RateLimit`], then reported as a usage limit, so a
    /// Loop falls back only once the throttle outlasts the backoff.
    ProviderThrottle,
    /// The provider answered that it has nothing to serve the model with, such
    /// as a hub's `auth_unavailable` when no pooled account is usable. Retried
    /// like [`Self::RateLimit`].
    ProviderUnavailable,
    /// The provider could not be reached at all: DNS resolution, TCP connect,
    /// or TLS failed before any request was served.
    ///
    /// Distinct from [`Self::RateLimit`] (provider answered) and
    /// [`Self::InvalidInput`] (request understood and rejected): the
    /// sandbox's own egress is the suspect, usually transient.
    Connectivity,
    /// A requested Session could not be resumed or was not found.
    UnavailableSession,
    /// Model, mode, or launch configuration was invalid.
    InvalidConfiguration,
    /// The submitted input was malformed or unsupported.
    InvalidInput,
    /// Native protocol output could not be parsed.
    NativeProtocol,
    /// The Agent process ended the run without producing it: an unexpected
    /// exit, an unexpected EOF, or a terminal result that reports no turns.
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
    /// When the provider says a [`ErrorClass::UsageLimit`] lifts, if it said.
    pub resets_at: Option<DateTime<Utc>>,
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
            resets_at: None,
        }
    }

    /// A usage-limit failure. The reset time, when known, is appended to the
    /// diagnostic as well, because the diagnostic is the only text the task
    /// and the UI show.
    pub fn usage_limit(
        stage: FailureStage,
        session_impact: SessionImpact,
        diagnostic: &str,
        resets_at: Option<DateTime<Utc>>,
    ) -> Self {
        let diagnostic = match resets_at {
            Some(at) => format!(
                "{diagnostic} (usage limit resets at {})",
                at.to_rfc3339_opts(SecondsFormat::Secs, true)
            ),
            None => diagnostic.to_string(),
        };
        Self {
            resets_at,
            ..Self::new(stage, ErrorClass::UsageLimit, session_impact, diagnostic)
        }
    }
}

/// Whether lowercased failure text is the provider declining the content.
pub(crate) fn is_safety_refusal(lower: &str) -> bool {
    lower.contains("safeguards flagged")
}

/// Whether lowercased failure text is a hub with no account to serve from.
pub(crate) fn is_provider_unavailable(lower: &str) -> bool {
    lower.contains("auth_unavailable") || lower.contains("no auth available")
}

/// Whether failure text names one of `codes` as a standalone number.
///
/// A bare `contains("500")` also matches "took 1500ms", "port 5000" or a
/// request id, and would misclassify an unrelated failure; deliberately, a
/// code counts only when no letter, digit, `_` or decimal point touches it.
pub(crate) fn names_status(text: &str, codes: &[&str]) -> bool {
    let bytes = text.as_bytes();
    // A neighbour joins the code when it is a letter, digit or `_`, or a `.`
    // with a digit on its far side.
    let joins = |next: Option<&u8>, far: Option<&u8>| match next {
        Some(b) if b.is_ascii_alphanumeric() || *b == b'_' => true,
        Some(b'.') => far.is_some_and(u8::is_ascii_digit),
        _ => false,
    };
    codes.iter().any(|code| {
        text.match_indices(code).any(|(start, _)| {
            let end = start + code.len();
            let before = start.checked_sub(1).and_then(|i| bytes.get(i));
            let before_far = start.checked_sub(2).and_then(|i| bytes.get(i));
            !(joins(before, before_far) || joins(bytes.get(end), bytes.get(end + 1)))
        })
    })
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

#[cfg(test)]
mod tests {
    use super::names_status;

    #[test]
    fn a_status_code_matches_only_as_a_standalone_number() {
        for text in [
            "401 invalid api key",
            "api error: 503 {\"type\":\"error\"}",
            "http status 500.",
            "request failed (status=502)",
            "error: 401",
        ] {
            assert!(
                names_status(text, &["401", "500", "502", "503"]),
                "{text:?}"
            );
        }
        for text in [
            "took 1500ms",
            "listening on port 5000",
            "request id req_401abc",
            "version 1.500.2",
            "model claude-3-5-sonnet-20240401",
            "line 4015 of output",
        ] {
            assert!(
                !names_status(text, &["401", "500", "502", "503"]),
                "{text:?}"
            );
        }
    }
}
