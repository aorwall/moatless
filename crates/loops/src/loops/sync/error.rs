//! Loop sync error types.
//!
//! Loop sync has exactly one call site: `repository_sync::handlers::sync_repo_config`,
//! the server-band composition behind the `sync-config` endpoint. It is never
//! its own HTTP handler, so this error does not implement `IntoResponse` — the
//! single caller maps it into `RepositoryError` with `?`. That abort is the
//! admin-visibility contract: the loop-sync error fails the explicit sync so an
//! administrator never mistakes a skipped refresh for a successful approval
//! revocation. (The git-declared workspace half of that same composition
//! deliberately does the opposite — it swallows failure best-effort — but that
//! is `WorkspaceSyncSummary`, not this error.)

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
    Store(#[from] moatless_backend_core::store::StoreError),

    #[error("Internal error: {0}")]
    Internal(String),
}
