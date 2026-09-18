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
        handlers::session::verify_handler,
        handlers::login::auth_mode_handler,
        handlers::password::password_login_handler,
        handlers::password::change_password_handler,
        handlers::password::admin_reset_password_handler,
        handlers::users::me_handler,
        handlers::users::update_me_handler,
        handlers::logout::logout_handler,
        handlers::logout::logout_all_handler,
        handlers::exchange::exchange_api_key_handler,
        handlers::exchange::exchange_token_handler,
        handlers::api_keys::create_api_key_handler,
        handlers::api_keys::list_api_keys_handler,
        handlers::api_keys::revoke_api_key_handler,
        handlers::api_keys::create_pairing_code_handler,
        handlers::users::list_users_handler,
        handlers::users::create_user_handler,
        handlers::users::update_user_handler,
        handlers::users::admin_update_user_handler,
        handlers::users::set_user_provider_id_handler,
        handlers::users::delete_user_provider_id_handler,
        handlers::users::list_user_provider_ids_handler,
        handlers::users::set_user_provider_id_by_id_handler,
        handlers::users::delete_user_provider_id_by_id_handler,
        handlers::users::list_user_provider_ids_by_id_handler,
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
        .route("/api/v1/auth/login", get(handlers::login::login_handler))
        .route(
            "/api/v1/auth/cli/login",
            get(handlers::login::cli_login_handler),
        )
        .route(
            "/api/v1/auth/github-app",
            get(handlers::provider::github_app_init_handler),
        )
        .route(
            "/api/v1/auth/github-app/connect",
            get(handlers::provider::github_app_connect_handler),
        )
        .route(
            "/api/v1/auth/github-app/callback",
            get(handlers::provider::github_app_callback_handler),
        )
        .route(
            "/api/v1/auth/forgejo/connect",
            get(handlers::provider::forgejo_connect_handler),
        )
        .route(
            "/api/v1/auth/forgejo/callback",
            get(handlers::provider::forgejo_callback_handler),
        )
        .route(
            "/api/v1/auth/auth0",
            get(handlers::provider::auth0_init_handler),
        )
        .route(
            "/api/v1/auth/auth0/callback",
            get(handlers::provider::auth0_callback_handler),
        )
        .route(
            "/api/v1/auth/oidc/{provider}/start",
            get(handlers::provider::oidc_start_handler),
        )
        .route(
            "/api/v1/auth/oidc/{provider}/callback",
            get(handlers::provider::oidc_callback_handler),
        )
        .route(
            "/api/v1/auth/logout",
            get(handlers::logout::logout_get_handler),
        )
        .with_state(state);

    // Merge both into a single Router
    let router: axum::Router = Into::<axum::Router>::into(openapi_router).merge(oauth_router);

    (router, api)
}

fn documented_auth_routes() -> OpenApiRouter<AuthState> {
    OpenApiRouter::with_openapi(AuthApiDoc::openapi())
        .routes(routes!(handlers::session::verify_handler))
        .routes(routes!(handlers::login::auth_mode_handler))
        .routes(routes!(handlers::password::password_login_handler))
        .routes(routes!(handlers::password::change_password_handler))
        .routes(routes!(handlers::password::admin_reset_password_handler))
        .routes(routes!(
            handlers::users::me_handler,
            handlers::users::update_me_handler
        ))
        .routes(routes!(handlers::logout::logout_handler))
        .routes(routes!(handlers::logout::logout_all_handler))
        .routes(routes!(handlers::exchange::exchange_api_key_handler))
        .routes(routes!(handlers::exchange::exchange_token_handler))
        .routes(routes!(
            handlers::api_keys::create_api_key_handler,
            handlers::api_keys::list_api_keys_handler
        ))
        .routes(routes!(handlers::api_keys::revoke_api_key_handler))
        .routes(routes!(handlers::api_keys::create_pairing_code_handler))
        .routes(routes!(
            handlers::users::list_users_handler,
            handlers::users::create_user_handler
        ))
        .routes(routes!(handlers::users::update_user_handler))
        .routes(routes!(handlers::users::admin_update_user_handler))
        .routes(routes!(
            handlers::users::set_user_provider_id_handler,
            handlers::users::delete_user_provider_id_handler
        ))
        .routes(routes!(handlers::users::list_user_provider_ids_handler))
        .routes(routes!(
            handlers::users::set_user_provider_id_by_id_handler,
            handlers::users::delete_user_provider_id_by_id_handler
        ))
        .routes(routes!(
            handlers::users::list_user_provider_ids_by_id_handler
        ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn documented_auth_routes_can_be_constructed() {
        let _ = super::documented_auth_routes();
    }
}
