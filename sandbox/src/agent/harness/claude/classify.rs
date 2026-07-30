//! Failure classification for the Claude subprocess adapter.
//!
//! Maps SDK transport errors and terminal `ResultMessage`s into stable
//! [`HarnessError`] facts (class + Session impact + stage) so the coordinator
//! drives policy without learning Claude-native error shapes.

use claude_agent_sdk::types::messages::ResultMessage;
use claude_agent_sdk::SdkError;

use super::super::error::{ErrorClass, FailureStage, HarnessError, SessionImpact};
use super::super::process::ProcessTermination;

/// Upper bound on a diagnostic string carried in a [`HarnessError`].
const MAX_DIAGNOSTIC: usize = 500;

fn truncate_diagnostic(text: &str) -> String {
    if text.len() <= MAX_DIAGNOSTIC {
        text.to_string()
    } else {
        format!("{}…", &text[..MAX_DIAGNOSTIC])
    }
}

/// Whether a startup failure means the resume-target transcript is unavailable
/// (a stale session id after a pod restart), the case the coordinator recovers
/// from by starting fresh.
fn is_session_unavailable(error: &SdkError) -> bool {
    matches!(
        error,
        SdkError::TmuxSessionDied { .. }
            | SdkError::SessionStartTimeout { .. }
            | SdkError::ProcessExit { .. }
            | SdkError::TransportClosed
            | SdkError::JsonlNotFound { .. }
    )
}

fn classify_sdk_error(error: &SdkError) -> (ErrorClass, SessionImpact) {
    match error {
        SdkError::CliNotFound { .. } | SdkError::InvalidConfig { .. } | SdkError::SpawnError(_) => {
            (ErrorClass::InvalidConfiguration, SessionImpact::Unknown)
        }
        SdkError::JsonError(_) | SdkError::JsonParseError { .. } => {
            (ErrorClass::NativeProtocol, SessionImpact::Unknown)
        }
        SdkError::ControlTimeout { .. } | SdkError::SessionStartTimeout { .. } => {
            (ErrorClass::UnresponsiveProcess, SessionImpact::Unknown)
        }
        SdkError::ProcessExit { .. } | SdkError::TransportClosed | SdkError::Cancelled => {
            (ErrorClass::ProcessExit, SessionImpact::Unknown)
        }
        SdkError::JsonlNotFound { .. } | SdkError::TmuxSessionDied { .. } => {
            (ErrorClass::UnavailableSession, SessionImpact::Lost)
        }
        SdkError::TmuxNotFound
        | SdkError::HookFifoFailed(_)
        | SdkError::HookHarnessFailed(_)
        | SdkError::ControlError { .. }
        | SdkError::Internal(_) => (ErrorClass::Internal, SessionImpact::Unknown),
    }
}

pub(super) fn classify_startup_error(error: &SdkError, resuming: bool) -> HarnessError {
    if resuming && is_session_unavailable(error) {
        return HarnessError::new(
            FailureStage::Startup,
            ErrorClass::UnavailableSession,
            SessionImpact::Lost,
            truncate_diagnostic(&error.to_string()),
        );
    }
    let (class, impact) = classify_sdk_error(error);
    HarnessError::new(
        FailureStage::Startup,
        class,
        impact,
        truncate_diagnostic(&error.to_string()),
    )
}

pub(super) fn classify_stream_error(error: &SdkError) -> HarnessError {
    let (class, impact) = classify_sdk_error(error);
    HarnessError::new(
        FailureStage::Execution,
        class,
        impact,
        truncate_diagnostic(&error.to_string()),
    )
}

/// Classify a terminal `ResultMessage`: a clean success completes the execution,
/// any error result becomes a classified failure the coordinator maps to policy.
pub(super) fn classify_result(result: &ResultMessage) -> ProcessTermination {
    match result {
        ResultMessage::Success(success) if !success.is_error => ProcessTermination::Completed,
        ResultMessage::Success(success) => {
            ProcessTermination::Failed(result_failure("", &success.result))
        }
        ResultMessage::Error(error) => {
            ProcessTermination::Failed(result_failure(&error.subtype, &error.errors.join("; ")))
        }
    }
}

fn result_failure(subtype: &str, text: &str) -> HarnessError {
    let haystack = format!("{subtype} {text}").to_lowercase();
    let matches_any = |patterns: &[&str]| patterns.iter().any(|p| haystack.contains(p));
    let class = if matches_any(&[
        "overloaded",
        "rate_limit",
        "capacity",
        "server_error",
        "529",
        "503",
        "502",
    ]) {
        ErrorClass::RateLimit
    } else if matches_any(&[
        "authenticat",
        "unauthorized",
        "401",
        "invalid api key",
        "invalid_api_key",
        "credit balance",
    ]) {
        ErrorClass::Authentication
    } else {
        // A completed-but-error result is the CLI rejecting the input (e.g.
        // "Unknown command: ..."), not a process/config failure.
        ErrorClass::InvalidInput
    };
    HarnessError::new(
        FailureStage::Execution,
        class,
        SessionImpact::Intact,
        truncate_diagnostic(text),
    )
}
