//! Provider settings module (GitHub OAuth/PAT management).

pub mod error;
pub mod github_client;
pub mod handlers;
pub mod types;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::state::AppState;

/// Build the provider settings router.
pub fn provider_settings_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/settings/github/config",
            get(handlers::get_github_config),
        )
        .route(
            "/api/v1/settings/github/pat",
            put(handlers::save_github_pat),
        )
        .route(
            "/api/v1/settings/github/oauth/url",
            get(handlers::get_github_oauth_url),
        )
        .route(
            "/api/v1/settings/github/oauth/token",
            post(handlers::exchange_github_token),
        )
        .route(
            "/api/v1/settings/github/config",
            delete(handlers::delete_github_config),
        )
}
