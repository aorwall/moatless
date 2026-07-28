//! Secret API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "secrets", description = "Scoped secret metadata and write-only value management"),
    ),
    paths(
        handlers::list_secrets,
        handlers::get_secret,
        handlers::put_secret,
        handlers::patch_secret,
        handlers::delete_secret,
        handlers::validate_secrets,
    ),
    components(schemas(
        moatless_api_schemas::enums::Scope,
        super::models::SecretKind,
        super::models::SecretSource,
        super::schemas::SecretMetadataResponse,
        super::schemas::PutSecretRequest,
        super::schemas::PatchSecretRequest,
        super::schemas::ValidateSecretsRequest,
        super::schemas::ValidateSecretsResponse,
        super::schemas::SecretMutationResponse,
    ))
)]
pub struct SecretApiDoc;

pub fn secret_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(SecretApiDoc::openapi())
        .routes(routes!(handlers::list_secrets))
        .routes(routes!(
            handlers::get_secret,
            handlers::patch_secret,
            handlers::delete_secret
        ))
        .routes(routes!(handlers::put_secret))
        .routes(routes!(handlers::validate_secrets))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
