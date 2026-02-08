//! Tag listing module.

pub mod handlers;

use axum::routing::get;
use axum::Router;

use crate::state::AppState;

/// Build the tag router.
pub fn tag_router() -> Router<AppState> {
    Router::new().route("/api/v1/tags", get(handlers::list_tags))
}
