//! Loop sync error types.
//!
//! Loop sync is invoked from the repository handlers (create + sync-config),
//! never as its own HTTP handler, so this error does not implement
//! `IntoResponse`. Callers log it and either fold it into the response summary
//! (sync-config) or swallow it best-effort (create).

#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error("Authentication required to fetch {git_url}")]
    AuthRequired { git_url: String },

    #[error("Loop fetch failed for {git_url}: {message}")]
    Fetch { git_url: String, message: String },

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}
