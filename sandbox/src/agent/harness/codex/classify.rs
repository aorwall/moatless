//! Failure classification for the Codex adapter.
//!
//! Maps Codex startup failures, stream transport errors, and terminal
//! `turn.failed` / `error` events into stable [`HarnessError`] facts so the
//! coordinator drives retry/credential policy without learning Codex-native
//! error shapes. The required stable classes are auth-refresh, unavailable
//! thread, native protocol, and unexpected exit (plus rate limiting).

use super::super::error::{ErrorClass, FailureStage, HarnessError, SessionImpact};

/// Upper bound on a diagnostic string carried in a [`HarnessError`].
const MAX_DIAGNOSTIC: usize = 500;

fn truncate_diagnostic(text: &str) -> String {
    if text.len() <= MAX_DIAGNOSTIC {
        return text.to_string();
    }
    let mut end = MAX_DIAGNOSTIC;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// A failure to spawn the Codex CLI. A launch failure is a configuration
/// problem, not a lost Session: a fresh spawn failing says nothing about a
/// resumable thread, so the impact is `Unknown` even when resuming.
pub(super) fn classify_startup_error(error: &std::io::Error) -> HarnessError {
    HarnessError::new(
        FailureStage::Startup,
        ErrorClass::InvalidConfiguration,
        SessionImpact::Unknown,
        truncate_diagnostic(&format!("failed to start Codex CLI: {error}")),
    )
}

/// A transport error from the Codex stdout stream: a parse failure is a native
/// protocol fault; anything else is an unexpected process exit.
pub(super) fn classify_stream_error(text: &str) -> HarnessError {
    let lower = text.to_lowercase();
    let class = if contains_any(&lower, &["parse", "failed to parse"]) {
        ErrorClass::NativeProtocol
    } else {
        ErrorClass::ProcessExit
    };
    HarnessError::new(
        FailureStage::Execution,
        class,
        SessionImpact::Unknown,
        truncate_diagnostic(text),
    )
}

/// A terminal `turn.failed` / `error` message from Codex. Auth-refresh, rate
/// limiting, and unavailable-thread failures get their stable classes; anything
/// else is treated as the CLI rejecting the input, with the Session left intact.
pub(super) fn classify_turn_error(text: &str) -> HarnessError {
    let lower = text.to_lowercase();
    let (class, impact) = if contains_any(
        &lower,
        &[
            "refresh_token_reused",
            "refresh token reused",
            "failed to refresh token",
            "401",
            "unauthorized",
            "invalid api key",
            "invalid_api_key",
            "credit balance",
        ],
    ) {
        (ErrorClass::Authentication, SessionImpact::Unknown)
    } else if contains_any(
        &lower,
        &[
            "overloaded",
            "rate_limit",
            "rate limit",
            "capacity",
            "server_error",
            "529",
            "503",
            "502",
        ],
    ) {
        (ErrorClass::RateLimit, SessionImpact::Intact)
    } else if contains_any(
        &lower,
        &[
            "no such thread",
            "thread not found",
            "session not found",
            "conversation not found",
            "unable to resume",
            "no conversation",
        ],
    ) {
        (ErrorClass::UnavailableSession, SessionImpact::Lost)
    } else {
        (ErrorClass::InvalidInput, SessionImpact::Intact)
    };
    HarnessError::new(
        FailureStage::Execution,
        class,
        impact,
        truncate_diagnostic(text),
    )
}
