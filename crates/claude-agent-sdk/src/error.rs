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

    #[error("tmux session died unexpectedly")]
    TmuxSessionDied,

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Convenience alias for SDK results.
pub type Result<T> = std::result::Result<T, SdkError>;
