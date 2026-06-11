//! Retry/backoff policy: classifying transient agent errors and computing
//! exponential backoff for the actor's retry timer.

use std::time::SystemTime;

use claude_agent_sdk::types::messages::ResultMessage;
use claude_agent_sdk::SdkError;
use tokio::time::Instant;

use super::AgentRuntimeError;

// ── Retry constants ──────────────────────────────────────────────
pub(super) const MAX_RETRIES: u32 = 5;
const BACKOFF_BASE_SECS: f64 = 5.0;
const BACKOFF_CAP_SECS: f64 = 60.0;
const JITTER_FACTOR: f64 = 0.2;

/// Pending retry state for transient agent errors.
pub(super) struct RetryState {
    pub(super) prompt: String,
    pub(super) model: Option<String>,
    pub(super) agent_type: Option<String>,
    pub(super) agent_mode: Option<String>,
    pub(super) attempt: u32,
    pub(super) retry_at: Instant,
}

/// Check if a `ResultMessage` represents a transient error worth retrying.
///
/// Retryable when the API returned overloaded/rate-limit/server errors and the
/// session barely started (≤5 turns), meaning there's little work to lose.
pub(super) fn is_retryable_result(result: &ResultMessage) -> bool {
    match result {
        ResultMessage::Error(e) => {
            if e.num_turns > 5 {
                return false;
            }
            let subtype = e.subtype.as_str();
            let retryable_subtypes = ["overloaded", "rate_limit", "server_error", "capacity"];
            if retryable_subtypes.iter().any(|s| subtype.contains(s)) {
                return true;
            }
            let retryable_patterns = ["529", "503", "502", "overloaded_error", "rate_limit"];
            e.errors.iter().any(|err| {
                let lower = err.to_lowercase();
                retryable_patterns
                    .iter()
                    .any(|pattern| lower.contains(pattern))
            })
        }
        ResultMessage::Success(s) => {
            if !s.is_error || s.num_turns > 5 {
                return false;
            }
            let lower = s.result.to_lowercase();
            ["overloaded", "rate_limit", "529", "503", "502"]
                .iter()
                .any(|pattern| lower.contains(pattern))
        }
    }
}

/// Check if an SDK-level error is transient and worth retrying.
fn is_retryable_sdk_error(error: &SdkError) -> bool {
    matches!(
        error,
        SdkError::ProcessExit { .. } | SdkError::TransportClosed
    )
}

pub(super) fn is_retryable_runtime_error(error: &AgentRuntimeError) -> bool {
    match error {
        AgentRuntimeError::Claude(error) => is_retryable_sdk_error(error),
        AgentRuntimeError::Codex(_) => false,
    }
}

/// Check if a startup error indicates the resume-target session transcript is
/// unavailable — the case where falling back to a fresh session is safe and
/// useful (e.g. a stale session id after a pod restart). Credential/config/
/// cancellation failures are excluded: a fresh session would not fix those.
pub(super) fn is_session_unavailable_error(error: &SdkError) -> bool {
    matches!(
        error,
        SdkError::TmuxSessionDied
            | SdkError::SessionStartTimeout { .. }
            | SdkError::ProcessExit { .. }
            | SdkError::JsonlNotFound { .. }
    )
}

/// Compute exponential backoff with jitter for a given attempt (0-indexed).
pub(super) fn compute_backoff(attempt: u32) -> std::time::Duration {
    let base = BACKOFF_BASE_SECS * 2.0_f64.powi(attempt as i32);
    let capped = base.min(BACKOFF_CAP_SECS);

    // Deterministic-ish jitter from system time nanos
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let jitter_norm = (nanos as f64 / u32::MAX as f64) * 2.0 - 1.0;
    let jitter = capped * JITTER_FACTOR * jitter_norm;

    let secs = (capped + jitter).max(1.0);
    std::time::Duration::from_secs_f64(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use claude_agent_sdk::types::content::Usage as ClaudeUsage;
    use claude_agent_sdk::types::messages::{ResultError, ResultSuccess};

    fn make_result_error(subtype: &str, num_turns: u32, errors: Vec<String>) -> ResultMessage {
        ResultMessage::Error(ResultError {
            subtype: subtype.to_string(),
            uuid: "test-uuid".to_string(),
            session_id: "test-session".to_string(),
            duration_ms: 100,
            duration_api_ms: 50,
            is_error: true,
            num_turns,
            total_cost_usd: 0.0,
            usage: ClaudeUsage {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                service_tier: None,
                cache_creation: None,
                server_tool_use: None,
            },
            model_usage: HashMap::new(),
            permission_denials: vec![],
            errors,
        })
    }

    fn make_result_success(is_error: bool, num_turns: u32, result: &str) -> ResultMessage {
        ResultMessage::Success(ResultSuccess {
            uuid: "test-uuid".to_string(),
            session_id: "test-session".to_string(),
            duration_ms: 100,
            duration_api_ms: 50,
            is_error,
            num_turns,
            result: result.to_string(),
            total_cost_usd: 0.0,
            usage: ClaudeUsage {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                service_tier: None,
                cache_creation: None,
                server_tool_use: None,
            },
            model_usage: HashMap::new(),
            permission_denials: vec![],
            structured_output: None,
        })
    }

    #[test]
    fn is_retryable_result_overloaded_error() {
        let result = make_result_error("overloaded", 1, vec!["overloaded_error".to_string()]);
        assert!(is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_result_rate_limit_subtype() {
        let result = make_result_error("rate_limit", 0, vec![]);
        assert!(is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_result_server_error_pattern() {
        let result = make_result_error(
            "unknown",
            2,
            vec!["HTTP 503 service unavailable".to_string()],
        );
        assert!(is_retryable_result(&result));
    }

    #[test]
    fn is_not_retryable_after_many_turns() {
        let result = make_result_error("overloaded", 6, vec!["overloaded_error".to_string()]);
        assert!(!is_retryable_result(&result));
    }

    #[test]
    fn is_retryable_success_with_error_text() {
        let result = make_result_success(true, 1, "Service overloaded (529)");
        assert!(is_retryable_result(&result));
    }

    #[test]
    fn is_not_retryable_success_without_error() {
        let result = make_result_success(false, 1, "All done");
        assert!(!is_retryable_result(&result));
    }

    #[test]
    fn session_unavailable_errors_trigger_fresh_session_fallback() {
        // Transcript-unavailable startup failures → fallback is safe.
        assert!(is_session_unavailable_error(&SdkError::TmuxSessionDied));
        assert!(is_session_unavailable_error(&SdkError::SessionStartTimeout {
            timeout_ms: 5000
        }));
        assert!(is_session_unavailable_error(&SdkError::ProcessExit {
            code: Some(1),
            signal: None
        }));
        assert!(is_session_unavailable_error(&SdkError::JsonlNotFound {
            project_dir: "/tmp/projects".to_string(),
            timeout_ms: 5000
        }));
    }

    #[test]
    fn non_session_errors_do_not_trigger_fresh_session_fallback() {
        // A fresh session would not fix these — they must stay terminal.
        assert!(!is_session_unavailable_error(&SdkError::InvalidConfig {
            message: "bad config".to_string()
        }));
        assert!(!is_session_unavailable_error(&SdkError::CliNotFound {
            path: "/bin/claude".to_string()
        }));
        assert!(!is_session_unavailable_error(&SdkError::TmuxNotFound));
        assert!(!is_session_unavailable_error(&SdkError::Cancelled));
        assert!(!is_session_unavailable_error(&SdkError::TransportClosed));
    }
}
