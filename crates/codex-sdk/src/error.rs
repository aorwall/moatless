use std::io;

/// Errors returned by the Codex SDK.
#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("startup failed: {0}")]
    Startup(#[source] io::Error),

    #[error("request failed: {0}")]
    Request(String),

    #[error("shutdown failed: {0}")]
    Shutdown(#[source] io::Error),

    #[error("operation cancelled")]
    Cancelled,

    #[error("{0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, SdkError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_error_display() {
        let err = SdkError::Config("bad model".to_string());
        assert_eq!(err.to_string(), "configuration error: bad model");
    }

    #[test]
    fn test_startup_error_display() {
        let err = SdkError::Startup(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not found",
        ));
        assert!(err.to_string().contains("startup failed"), "got: {}", err);
    }

    #[test]
    fn test_request_error_display() {
        let err = SdkError::Request("turn/start failed".to_string());
        assert_eq!(err.to_string(), "request failed: turn/start failed");
    }

    #[test]
    fn test_cancelled_error_display() {
        let err = SdkError::Cancelled;
        assert_eq!(err.to_string(), "operation cancelled");
    }

    #[test]
    fn test_internal_error_display() {
        let err = SdkError::Internal("unexpected state".to_string());
        assert_eq!(err.to_string(), "unexpected state");
    }
}
