//! Credential service error type.

use crate::auth::error::AuthError;

/// Errors returned by [`crate::credentials::CredentialService`].
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("auth error: {0}")]
    Auth(#[from] AuthError),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("secret storage error: {0}")]
    Secret(#[from] crate::secret::error::SecretError),

    /// The stored Codex `auth.json` blob is not parseable JSON, so the refresh
    /// cannot read the tokens out of it. Retains the parse failure — the offset
    /// it reports is what makes a corrupt row diagnosable.
    #[error("stored Codex auth blob for {scope} is not valid JSON")]
    CodexBlobUnreadable {
        scope: String,
        #[source]
        source: serde_json::Error,
    },

    /// An invariant this module synthesizes, with no underlying failure to
    /// carry — kept distinct from the wrapping variants above.
    #[error("internal: {0}")]
    Internal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_blob_failure_keeps_the_parse_error_as_source() {
        let source = serde_json::from_str::<serde_json::Value>("{\"tokens\":").unwrap_err();
        let err = CredentialError::CodexBlobUnreadable {
            scope: "user:alice".to_string(),
            source,
        };
        let source = std::error::Error::source(&err).expect("parse error is the source");
        assert!(source.to_string().contains("line 1"));

        // The Display of the wrapper names the scope but never the blob, so a
        // log line built from it carries no token material.
        assert_eq!(
            err.to_string(),
            "stored Codex auth blob for user:alice is not valid JSON"
        );
    }

    #[test]
    fn synthesized_internal_messages_have_no_source() {
        let err = CredentialError::Internal("Unknown credential key: NOPE".to_string());
        assert!(std::error::Error::source(&err).is_none());
    }
}
