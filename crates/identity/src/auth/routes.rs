//! Authentication route registration.

use super::handlers;
pub(super) use super::state::AuthState;
use axum::routing::get;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "auth", description = "Authentication and session management"),
        (name = "api-keys", description = "API key management"),
        (name = "users", description = "User listing"),
    ),
    paths(
        handlers::verify_handler,
        handlers::auth_mode_handler,
        handlers::password_login_handler,
        handlers::change_password_handler,
        handlers::admin_reset_password_handler,
        handlers::me_handler,
        handlers::update_me_handler,
        handlers::logout_handler,
        handlers::logout_all_handler,
        handlers::exchange_api_key_handler,
        handlers::exchange_token_handler,
        handlers::create_api_key_handler,
        handlers::list_api_keys_handler,
        handlers::revoke_api_key_handler,
        handlers::create_pairing_code_handler,
        handlers::list_users_handler,
        handlers::create_user_handler,
        handlers::update_user_handler,
        handlers::set_user_provider_id_handler,
        handlers::delete_user_provider_id_handler,
        handlers::list_user_provider_ids_handler,
    )
)]
pub struct AuthApiDoc;

/// Create the auth router with all endpoints.
pub(super) fn auth_router(state: AuthState) -> (axum::Router, utoipa::openapi::OpenApi) {
    // Build OpenAPI-documented routes
    let (openapi_router, api) = documented_auth_routes()
        .with_state(state.clone())
        .split_for_parts();

    // Build a separate plain Router for OAuth redirect routes (not in OpenAPI — browser-only flows)
    let oauth_router = axum::Router::new()
        .route("/api/v1/auth/login", get(handlers::login_handler))
        .route("/api/v1/auth/cli/login", get(handlers::cli_login_handler))
        .route(
            "/api/v1/auth/github-app",
            get(handlers::github_app_init_handler),
        )
        .route(
            "/api/v1/auth/github-app/connect",
            get(handlers::github_app_connect_handler),
        )
        .route(
            "/api/v1/auth/github-app/callback",
            get(handlers::github_app_callback_handler),
        )
        .route("/api/v1/auth/auth0", get(handlers::auth0_init_handler))
        .route(
            "/api/v1/auth/auth0/callback",
            get(handlers::auth0_callback_handler),
        )
        .route(
            "/api/v1/auth/oidc/{provider}/start",
            get(handlers::oidc_start_handler),
        )
        .route(
            "/api/v1/auth/oidc/{provider}/callback",
            get(handlers::oidc_callback_handler),
        )
        .route("/api/v1/auth/logout", get(handlers::logout_get_handler))
        .with_state(state);

    // Merge both into a single Router
    let router: axum::Router = Into::<axum::Router>::into(openapi_router).merge(oauth_router);

    (router, api)
}

fn documented_auth_routes() -> OpenApiRouter<AuthState> {
    OpenApiRouter::with_openapi(AuthApiDoc::openapi())
        .routes(routes!(handlers::verify_handler))
        .routes(routes!(handlers::auth_mode_handler))
        .routes(routes!(handlers::password_login_handler))
        .routes(routes!(handlers::change_password_handler))
        .routes(routes!(handlers::admin_reset_password_handler))
        .routes(routes!(handlers::me_handler, handlers::update_me_handler))
        .routes(routes!(handlers::logout_handler))
        .routes(routes!(handlers::logout_all_handler))
        .routes(routes!(handlers::exchange_api_key_handler))
        .routes(routes!(handlers::exchange_token_handler))
        .routes(routes!(
            handlers::create_api_key_handler,
            handlers::list_api_keys_handler
        ))
        .routes(routes!(handlers::revoke_api_key_handler))
        .routes(routes!(handlers::create_pairing_code_handler))
        .routes(routes!(
            handlers::list_users_handler,
            handlers::create_user_handler
        ))
        .routes(routes!(handlers::update_user_handler))
        .routes(routes!(
            handlers::set_user_provider_id_handler,
            handlers::delete_user_provider_id_handler
        ))
        .routes(routes!(handlers::list_user_provider_ids_handler))
}

#[cfg(test)]
mod tests {
    #[test]
    fn documented_auth_routes_can_be_constructed() {
        let _ = super::documented_auth_routes();
    }
}
