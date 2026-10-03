//! Failure classification for the Codex adapter: startup failures, stream
//! transport errors, request rejections and failed Turns become stable
//! [`HarnessError`] facts, so the coordinator never learns Codex error shapes.
//!
//! A failed Turn is classified from its named `codexErrorInfo`, not its free
//! text, which varies by provider. The texts read are a safety refusal and a
//! hub's `auth_unavailable`, ahead of any cause, and a usage limit where no
//! named cause says otherwise.

use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone, Utc};
use tracing::warn;

use crate::agent::codex_app_server::RpcError;
use crate::agent::codex_app_server::protocol::{CodexErrorInfo, TurnError};

use super::super::error::{
    ErrorClass, FailureStage, HarnessError, SessionImpact, is_provider_unavailable,
    is_safety_refusal,
};

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

/// A rejected request.
///
/// `thread/resume` is the one method whose rejection says something about the
/// Session itself (the thread could not be reopened), so it reports the Session
/// lost rather than letting the coordinator retry against a thread that isn't
/// there.
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
/// A missing or unknown `codexErrorInfo` takes the same conservative default as
/// an explicit `other` (input rejected, Session still usable) unless its
/// message names a usage limit; an unrecognised value is logged with its raw
/// JSON so a new upstream code stays visible.
pub(super) fn classify_turn_error(error: &TurnError) -> HarnessError {
    let text = turn_error_text(error);
    let lower = text.to_lowercase();
    let usage_limit_text = lower.contains("usage limit");
    let (class, impact) = match &error.codex_error_info {
        // Read ahead of every named cause: a refusal may quote a status, and
        // a hub's `auth_unavailable` arrives under whichever one Codex picks.
        Some(CodexErrorInfo::Unauthorized) => (ErrorClass::Authentication, SessionImpact::Unknown),
        _ if is_safety_refusal(&lower) => (ErrorClass::InvalidInput, SessionImpact::Intact),
        _ if is_provider_unavailable(&lower) => {
            (ErrorClass::ProviderUnavailable, SessionImpact::Intact)
        }
        Some(CodexErrorInfo::UsageLimitExceeded) => (ErrorClass::UsageLimit, SessionImpact::Intact),
        // Any 429 Codex did not read as `usage_limit_reached`, unretried
        // (`retry_429: false`) and with its body dropped, so a hub's
        // `model_cooldown` and a passing throttle arrive alike.
        Some(CodexErrorInfo::ResponseTooManyFailedAttempts {
            http_status_code: Some(429),
        }) => (ErrorClass::ProviderThrottle, SessionImpact::Intact),
        Some(
            CodexErrorInfo::ServerOverloaded
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
        Some(CodexErrorInfo::Other) | None if usage_limit_text => {
            (ErrorClass::UsageLimit, SessionImpact::Intact)
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
                "Unrecognized codexErrorInfo; classifying from its message"
            );
            if usage_limit_text {
                (ErrorClass::UsageLimit, SessionImpact::Intact)
            } else {
                (ErrorClass::InvalidInput, SessionImpact::Intact)
            }
        }
    };
    if class == ErrorClass::UsageLimit {
        return HarnessError::usage_limit(
            FailureStage::Execution,
            impact,
            &truncate_diagnostic(&text),
            try_again_at(&error.message, Local::now()),
        );
    }
    HarnessError::new(
        FailureStage::Execution,
        class,
        impact,
        truncate_diagnostic(&text),
    )
}

/// The reset time in Codex's usage-limit sentence ("… try again at Oct 3rd,
/// 2026 6:05 PM." or, the same day, "… at 6:05 PM."), which Codex renders in
/// the pod's local zone; `now` supplies that zone and the same-day date.
pub(super) fn try_again_at<Tz: TimeZone>(
    message: &str,
    now: DateTime<Tz>,
) -> Option<DateTime<Utc>> {
    const MARKER: &str = "try again at ";
    let start = message.to_lowercase().find(MARKER)? + MARKER.len();
    let stamp = message.get(start..)?.split('.').next()?.trim();
    let zone = now.timezone();
    let local = match stamp.split_once(", ") {
        // "Oct 3rd" + "2026 6:05 PM": drop the ordinal suffix chrono cannot read.
        Some((month_day, year_time)) => {
            let month_day = month_day.trim_end_matches(char::is_alphabetic);
            let (year, time) = year_time.split_once(' ')?;
            let date =
                NaiveDate::parse_from_str(&format!("{month_day} {year}"), "%b %d %Y").ok()?;
            date.and_time(NaiveTime::parse_from_str(time, "%I:%M %p").ok()?)
        }
        None => now
            .date_naive()
            .and_time(NaiveTime::parse_from_str(stamp, "%I:%M %p").ok()?),
    };
    zone.from_local_datetime(&local)
        .single()
        .map(|at| at.with_timezone(&Utc))
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
