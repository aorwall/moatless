//! Secret API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::service::SecretService;
use moatless_core::http_auth::UserTokenVerifier;

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
        moatless_types::secret::SecretKind,
        moatless_types::secret::SecretSource,
        super::schemas::SecretMetadataResponse,
        super::schemas::PutSecretRequest,
        super::schemas::PatchSecretRequest,
        super::schemas::ValidateSecretsRequest,
        super::schemas::ValidateSecretsResponse,
        super::schemas::SecretMutationResponse,
    ))
)]
pub struct SecretApiDoc;

/// Build the secret router with OpenAPI metadata.
///
/// Generic over the router state `S` so this module names no root type: the
/// composition root supplies the concrete state at the call site, and each
/// handler reaches its ports — `Arc<SecretService>` and the `UserTokenVerifier`
/// behind `AuthUser` — through `FromRef<S>`.
pub fn secret_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<SecretService>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
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
