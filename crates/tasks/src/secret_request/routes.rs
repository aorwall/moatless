//! Secret request API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::handlers;
use super::service::SecretRequests;
use moatless_core::http_auth::UserTokenVerifier;

#[derive(OpenApi)]
#[openapi(
    tags((name = "secret-requests", description = "Secrets an agent asks a person for")),
    paths(handlers::create_secret_request)
)]
pub struct SecretRequestApiDoc;

/// Build the secret request router, generic over router state `S` like
/// [`crate::feedback::routes::feedback_router`].
pub fn secret_request_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<SecretRequests>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    OpenApiRouter::with_openapi(SecretRequestApiDoc::openapi())
        .routes(routes!(handlers::create_secret_request))
        .with_state(state)
        .split_for_parts()
}
