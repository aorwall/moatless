//! Failure classification for the Codex adapter.
//!
//! Maps Codex startup failures, stream transport errors, request rejections and
//! failed Turns into stable [`HarnessError`] facts so the coordinator drives
//! retry/credential policy without learning Codex-native error shapes.
//!
//! A failed Turn is classified from the **named** `codexErrorInfo` the server
//! reports, not by matching substrings against the message. The message is free
//! text that varies by provider — the same `-32600` steer rejection carries
//! three different sentences — and an stderr scan for the same markers also
//! matched a background model refresh unrelated to the Turn.

use tracing::warn;

use crate::agent::codex_app_server::RpcError;
use crate::agent::codex_app_server::protocol::{CodexErrorInfo, TurnError};

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

/// A transport error from the app-server stream: a framing failure is a native
/// protocol fault; anything else is an unexpected process exit.
pub(super) fn classify_stream_error(text: &str) -> HarnessError {
    let lower = text.to_lowercase();
    let class = if lower.contains("parse") {
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

/// Server overload, which the app-server reports as a reserved JSON-RPC code
/// rather than a `codexErrorInfo`.
const SERVER_OVERLOADED: i64 = -32001;

/// A rejected request. `thread/resume` is the one method whose rejection says
/// something about the Session itself — the thread could not be reopened — so it
/// reports the Session lost rather than letting the coordinator retry against a
/// thread that is not there.
pub(super) fn classify_request_error(method: &str, error: &RpcError) -> HarnessError {
    let (class, impact) = match error {
        RpcError::Rpc(info) if info.code == SERVER_OVERLOADED => {
            (ErrorClass::RateLimit, SessionImpact::Intact)
        }
        RpcError::Rpc(_) if method == "thread/resume" => {
            (ErrorClass::UnavailableSession, SessionImpact::Lost)
        }
        RpcError::Rpc(_) => (ErrorClass::InvalidInput, SessionImpact::Intact),
        RpcError::Decode(_) => (ErrorClass::NativeProtocol, SessionImpact::Unknown),
        RpcError::Transport(_) => (ErrorClass::ProcessExit, SessionImpact::Unknown),
    };
    HarnessError::new(
        FailureStage::Execution,
        class,
        impact,
        truncate_diagnostic(&format!("{method} failed: {error}")),
    )
}

/// A failed Turn, classified from its named cause.
///
/// An error with no `codexErrorInfo` at all, or one this build does not know,
/// takes the same conservative default as an explicit `other`: the input was
/// rejected and the Session is still usable. An unrecognised value is logged
/// with its raw JSON so a new upstream code is visible rather than silently
/// collapsing into that default.
pub(super) fn classify_turn_error(error: &TurnError) -> HarnessError {
    let (class, impact) = match &error.codex_error_info {
        Some(CodexErrorInfo::Unauthorized) => (ErrorClass::Authentication, SessionImpact::Unknown),
        Some(
            CodexErrorInfo::UsageLimitExceeded
            | CodexErrorInfo::ServerOverloaded
            | CodexErrorInfo::InternalServerError
            | CodexErrorInfo::ResponseTooManyFailedAttempts { .. },
        ) => (ErrorClass::RateLimit, SessionImpact::Intact),
        // A status code means a server answered, so the failure is the
        // provider's; its absence means the request never landed and the pod's
        // own egress is the suspect. Both retry — the class only says which.
        Some(
            CodexErrorInfo::HttpConnectionFailed { http_status_code }
            | CodexErrorInfo::ResponseStreamConnectionFailed { http_status_code }
            | CodexErrorInfo::ResponseStreamDisconnected { http_status_code },
        ) => (
            if http_status_code.is_some() {
                ErrorClass::RateLimit
            } else {
                ErrorClass::Connectivity
            },
            SessionImpact::Intact,
        ),
        Some(CodexErrorInfo::SandboxError) => {
            (ErrorClass::InvalidConfiguration, SessionImpact::Intact)
        }
        Some(CodexErrorInfo::ThreadRollbackFailed) => {
            (ErrorClass::UnavailableSession, SessionImpact::Lost)
        }
        Some(
            CodexErrorInfo::ActiveTurnNotSteerable { .. }
            | CodexErrorInfo::ContextWindowExceeded
            | CodexErrorInfo::SessionBudgetExceeded
            | CodexErrorInfo::BadRequest
            | CodexErrorInfo::CyberPolicy
            | CodexErrorInfo::Other,
        )
        | None => (ErrorClass::InvalidInput, SessionImpact::Intact),
        Some(CodexErrorInfo::Unrecognized(raw)) => {
            warn!(
                codex_error_info = %raw,
                "Unrecognized codexErrorInfo; classifying as invalid input"
            );
            (ErrorClass::InvalidInput, SessionImpact::Intact)
        }
    };
    HarnessError::new(
        FailureStage::Execution,
        class,
        impact,
        truncate_diagnostic(&turn_error_text(error)),
    )
}

/// A failed Turn's message, plus the `additionalDetails` the server sometimes
/// attaches — the provider's own explanation, which the message alone omits.
fn turn_error_text(error: &TurnError) -> String {
    match &error.additional_details {
        Some(details) => format!("{}: {details}", error.message),
        None => error.message.clone(),
    }
}

/// Whether a failed Turn means the Codex credential needs refreshing. The
/// coordinator owns everything after this: pull, compare against the on-disk
/// `auth.json`, retry only if genuinely different.
pub(super) fn is_expired_credential(error: &TurnError) -> bool {
    matches!(error.codex_error_info, Some(CodexErrorInfo::Unauthorized))
}
