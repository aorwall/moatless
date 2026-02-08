//! Figma integration module.

pub mod error;
pub mod handlers;
pub mod types;

use axum::routing::{get, post};
use axum::Router;

use crate::state::AppState;

/// Build the figma router.
pub fn figma_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/figma/import-clipboard",
            post(handlers::import_clipboard),
        )
        .route(
            "/api/v1/figma/import-node",
            post(handlers::import_node),
        )
        .route(
            "/api/v1/figma/context/{import_id}",
            get(handlers::get_figma_context),
        )
}
