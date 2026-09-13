//! Loop sync error types.
//!
//! Loop sync has exactly one call site —
//! `repository_sync::handlers::sync_repo_config` — and never implements
//! `IntoResponse`; the caller maps it into `RepositoryError` with `?`. The
//! abort is deliberate: an administrator must never mistake a skipped refresh
//! for a successful approval revocation. (The git-declared workspace half of
//! that composition swallows failure best-effort instead, as
//! `WorkspaceSyncSummary`.)

#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error("Authentication required to fetch {git_url}")]
    AuthRequired { git_url: String },

    #[error("Loop fetch failed for {git_url}: {message}")]
    Fetch { git_url: String, message: String },

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    /// A read or write through a published store seam failed.
    #[error(transparent)]
    Store(#[from] moatless_types::store::StoreError),

    #[error("Internal error: {0}")]
    Internal(String),
}
