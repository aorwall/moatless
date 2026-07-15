use thiserror::Error;

/// All errors that can occur in the Claude Agent SDK.
#[derive(Error, Debug)]
pub enum SdkError {
    #[error("Failed to spawn CLI process: {0}")]
    SpawnError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("JSON parse error on line: {raw_line}")]
    JsonParseError {
        source: serde_json::Error,
        raw_line: String,
    },

    #[error("Process exited with code {code:?}, signal {signal:?}")]
    ProcessExit {
        code: Option<i32>,
        signal: Option<String>,
    },

    #[error("Control request {request_id} timed out after {timeout_ms}ms")]
    ControlTimeout { request_id: String, timeout_ms: u64 },

    #[error("Control request {request_id} failed: {error}")]
    ControlError { request_id: String, error: String },

    #[error("Transport closed")]
    TransportClosed,

    #[error("Operation cancelled")]
    Cancelled,

    #[error("CLI not found at: {path}")]
    CliNotFound { path: String },

    #[error("Invalid configuration: {message}")]
    InvalidConfig { message: String },

    #[error("tmux binary not found — is tmux installed?")]
    TmuxNotFound,

    #[error("JSONL session file not found in {project_dir} within {timeout_ms}ms")]
    JsonlNotFound {
        project_dir: String,
        timeout_ms: u64,
    },

    #[error("tmux session died unexpectedly{}", pane_suffix(pane_tail))]
    TmuxSessionDied { pane_tail: Option<String> },

    #[error("Hook FIFO setup failed: {0}")]
    HookFifoFailed(std::io::Error),

    #[error(
        "Session did not start within {timeout_ms}ms (no SessionStart hook fired){}",
        pane_suffix(pane_tail)
    )]
    SessionStartTimeout {
        timeout_ms: u64,
        pane_tail: Option<String>,
    },

    #[error("Hook harness setup failed: {0}")]
    HookHarnessFailed(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl SdkError {
    /// Attach a captured tmux pane tail to the startup-failure variants
    /// ([`SdkError::TmuxSessionDied`] / [`SdkError::SessionStartTimeout`]) so the
    /// reason claude exited flows into the surfaced error (and from there into
    /// the task's error event via `Display`). Returns `self` unchanged for every
    /// other variant.
    pub fn with_pane_tail(self, pane: Option<String>) -> Self {
        match self {
            SdkError::TmuxSessionDied { .. } => SdkError::TmuxSessionDied { pane_tail: pane },
            SdkError::SessionStartTimeout { timeout_ms, .. } => SdkError::SessionStartTimeout {
                timeout_ms,
                pane_tail: pane,
            },
            other => other,
        }
    }
}

/// Max bytes of a captured pane tail embedded in an error's `Display`. Kept
/// small so the error — surfaced verbatim in the task's error event — stays
/// readable; `log_startup_failure` still logs the full `PANE_CAPTURE_MAX_BYTES`
/// (2048) tail to the pod's stdout.
const ERROR_PANE_TAIL_MAX_BYTES: usize = 800;

/// Render a captured tmux pane tail as a trailing clause for an error's
/// `Display`, or `""` when there's nothing useful to show. The tail is capped
/// at [`ERROR_PANE_TAIL_MAX_BYTES`], snapped forward to a UTF-8 char boundary.
fn pane_suffix(pane_tail: &Option<String>) -> String {
    let Some(tail) = pane_tail else {
        return String::new();
    };
    let trimmed = tail.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut start = trimmed.len().saturating_sub(ERROR_PANE_TAIL_MAX_BYTES);
    while start < trimmed.len() && !trimmed.is_char_boundary(start) {
        start += 1;
    }
    format!(" — last terminal output:\n{}", &trimmed[start..])
}

/// Convenience alias for SDK results.
pub type Result<T> = std::result::Result<T, SdkError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_session_died_display_includes_pane_tail() {
        let with = SdkError::TmuxSessionDied {
            pane_tail: Some("boom".into()),
        };
        let s = with.to_string();
        assert!(s.contains("tmux session died unexpectedly"), "{s}");
        assert!(s.contains("boom"), "{s}");

        // None case Display is unchanged (no trailing clause).
        let without = SdkError::TmuxSessionDied { pane_tail: None };
        assert_eq!(without.to_string(), "tmux session died unexpectedly");
    }

    #[test]
    fn session_start_timeout_display_appends_pane_tail() {
        let err = SdkError::SessionStartTimeout {
            timeout_ms: 60000,
            pane_tail: Some("No conversation found".into()),
        };
        let s = err.to_string();
        assert!(s.contains("60000ms"), "{s}");
        assert!(s.contains("No conversation found"), "{s}");
    }

    #[test]
    fn with_pane_tail_only_fills_startup_variants() {
        // Startup variants get the tail...
        let died = SdkError::TmuxSessionDied { pane_tail: None }.with_pane_tail(Some("x".into()));
        assert!(matches!(
            died,
            SdkError::TmuxSessionDied { pane_tail: Some(_) }
        ));

        // ...other variants are returned unchanged.
        let other = SdkError::TransportClosed.with_pane_tail(Some("x".into()));
        assert!(matches!(other, SdkError::TransportClosed));
    }
}
