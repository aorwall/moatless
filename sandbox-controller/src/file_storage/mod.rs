//! File storage module — upload/download via S3.

pub mod error;
pub mod handlers;
pub mod s3;
pub mod types;

use axum::routing::{get, post};
use axum::Router;

use crate::state::AppState;

/// Build the file storage router.
pub fn file_storage_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/uploads", post(handlers::upload_file))
        .route("/api/v1/uploads/{file_id}", get(handlers::download_file))
}
