//! Failure classification for the Claude subprocess adapter.
//!
//! Maps SDK transport errors and terminal `ResultMessage`s into stable
//! [`HarnessError`] facts (class + Session impact + stage) so the coordinator
//! drives policy without learning Claude-native error shapes.

use chrono::{DateTime, Utc};
use claude_agent_sdk::SdkError;
use claude_agent_sdk::api::messages::ResultMessage;

use super::super::error::{
    ErrorClass, FailureStage, HarnessError, SessionImpact, is_provider_unavailable,
    is_safety_refusal, names_status,
};
use super::super::process::ProcessTermination;

/// Upper bound on a diagnostic string carried in a [`HarnessError`].
const MAX_DIAGNOSTIC: usize = 500;

fn truncate_diagnostic(text: &str) -> String {
    if text.len() <= MAX_DIAGNOSTIC {
        return text.to_string();
    }
    // Walk back to a char boundary: a diagnostic carries a repo path or an
    // ellipsis, and slicing through a multi-byte char panics.
    let mut end = MAX_DIAGNOSTIC;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
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
        SdkError::CliNotFound { .. }
        | SdkError::InvalidConfig { .. }
        | SdkError::SpawnError(_)
        | SdkError::ArgumentTooLong { .. } => {
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

/// Classify a terminal `ResultMessage`: a clean success completes the
/// execution, any error result becomes a classified failure the coordinator
/// maps to policy. `slash_command` says the Turn's prompt was a CLI command
/// (see [`is_slash_command`](crate::agent::harness::signal::is_slash_command)).
pub(super) fn classify_result(result: &ResultMessage, slash_command: bool) -> ProcessTermination {
    match result {
        // A success carrying no turns is the CLI ending the run without ever
        // reaching the model, so the person's message was consumed and answered
        // with nothing. Retrying re-drives the same text into the same session.
        // A command such as `/compact` runs without a model turn, so its result
        // has none either; retrying it compacts again.
        ResultMessage::Success(success)
            if !success.is_error && success.num_turns == 0 && !slash_command =>
        {
            ProcessTermination::Failed(HarnessError::new(
                FailureStage::Execution,
                ErrorClass::ProcessExit,
                SessionImpact::Intact,
                format!(
                    "Agent returned an empty result with no turns after {}ms",
                    success.duration_ms
                ),
            ))
        }
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
    // First: a refusal may quote a status or "usage", and must never be
    // retried or handed to the other provider.
    if is_safety_refusal(&haystack) {
        return HarnessError::new(
            FailureStage::Execution,
            ErrorClass::InvalidInput,
            SessionImpact::Intact,
            truncate_diagnostic(text),
        );
    }
    if is_provider_unavailable(&haystack) {
        return HarnessError::new(
            FailureStage::Execution,
            ErrorClass::ProviderUnavailable,
            SessionImpact::Intact,
            truncate_diagnostic(text),
        );
    }
    // Checked before the rate-limit patterns: a hub's cooldown body quotes the
    // upstream `rate_limit_error` that started it.
    if matches_any(&[
        "you've hit your",
        "you're out of extra usage",
        "usage limit reached",
        "model_cooldown",
    ]) {
        return HarnessError::usage_limit(
            FailureStage::Execution,
            SessionImpact::Intact,
            &truncate_diagnostic(text),
            usage_limit_reset(text),
        );
    }
    let class = if matches_any(&["overloaded", "rate_limit", "capacity", "server_error"])
        || names_status(&haystack, &["529", "503", "502"])
    {
        ErrorClass::RateLimit
    } else if matches_any(&[
        "authenticat",
        "unauthorized",
        "invalid api key",
        "invalid_api_key",
        "credit balance",
    ]) || names_status(&haystack, &["401"])
    {
        ErrorClass::Authentication
    } else if matches_any(&[
        "unable to connect",
        "could not resolve",
        "name resolution",
        "enotfound",
        "enotimp",
        "eai_again",
        "econnrefused",
        "econnreset",
        "etimedout",
        "ehostunreach",
        "enetunreach",
        "socket hang up",
    ]) {
        // The CLI never reached the provider — the pod's own egress (DNS, NAT,
        // proxy) is the suspect, so this must not read as a rejected request.
        ErrorClass::Connectivity
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

/// The reset time a usage-limit result states in a machine-readable form: the
/// 2.0 CLI's `Claude AI usage limit reached|<unix seconds>`, or a `CLIProxyAPI`
/// `model_cooldown` body's `reset_seconds`. The 2.1 CLI's "resets 3pm (UTC)"
/// is left in the text, where it already reads.
fn usage_limit_reset(text: &str) -> Option<DateTime<Utc>> {
    if let Some((_, seconds)) = text.split_once("usage limit reached|") {
        let seconds: i64 = seconds.trim().parse().ok()?;
        return DateTime::from_timestamp(seconds, 0);
    }
    let body: serde_json::Value = serde_json::from_str(&text[text.find('{')?..]).ok()?;
    let seconds = body["error"]["reset_seconds"].as_i64()?;
    Some(Utc::now() + chrono::Duration::seconds(seconds))
}
