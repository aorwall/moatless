//! Secret HTTP handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;

use crate::extractors::AuthUser;
use crate::secret::service::SecretService;

use super::error::SecretError;
use super::schemas::{ListSecretsQuery, PatchSecretRequest, PutSecretRequest, SecretMetadataResponse, SecretMutationResponse, ValidateSecretsRequest, ValidateSecretsResponse};

#[utoipa::path(
    get,
    path = "/api/v1/secrets",
    tag = "secrets",
    params(ListSecretsQuery),
    responses(
        (status = 200, description = "List secret metadata", body = Vec<SecretMetadataResponse>),
        (status = 403, description = "Forbidden", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_secrets(
    State(secrets): State<Arc<SecretService>>,
    auth: AuthUser,
    Query(query): Query<ListSecretsQuery>,
) -> Result<Json<Vec<SecretMetadataResponse>>, SecretError> {
    let rows = secrets
        .list_metadata(&auth.scope_context(), query.scope)
        .await?;
    Ok(Json(rows))
}

#[utoipa::path(
    get,
    path = "/api/v1/secrets/{id}",
    tag = "secrets",
    params(("id" = String, Path, description = "Secret ID")),
    responses(
        (status = 200, description = "Secret metadata", body = SecretMetadataResponse),
        (status = 404, description = "Secret not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_secret(
    State(secrets): State<Arc<SecretService>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<SecretMetadataResponse>, SecretError> {
    let row = secrets.get_metadata(&auth.scope_context(), &id).await?;
    Ok(Json(row))
}

#[utoipa::path(
    put,
    path = "/api/v1/secrets",
    tag = "secrets",
    request_body = PutSecretRequest,
    responses(
        (status = 200, description = "Secret created or replaced", body = SecretMutationResponse),
        (status = 400, description = "Invalid secret", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn put_secret(
    State(secrets): State<Arc<SecretService>>,
    auth: AuthUser,
    Json(req): Json<PutSecretRequest>,
) -> Result<Json<SecretMutationResponse>, SecretError> {
    let row = secrets
        .put_secret(&auth.scope_context(), &auth.user_id.to_string(), req)
        .await?;
    Ok(Json(SecretMutationResponse { secret: row }))
}

#[utoipa::path(
    patch,
    path = "/api/v1/secrets/{id}",
    tag = "secrets",
    params(("id" = String, Path, description = "Secret ID")),
    request_body = PatchSecretRequest,
    responses(
        (status = 200, description = "Secret updated", body = SecretMutationResponse),
        (status = 404, description = "Secret not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn patch_secret(
    State(secrets): State<Arc<SecretService>>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<PatchSecretRequest>,
) -> Result<Json<SecretMutationResponse>, SecretError> {
    let row = secrets
        .patch_secret(&auth.scope_context(), &auth.user_id.to_string(), &id, req)
        .await?;
    Ok(Json(SecretMutationResponse { secret: row }))
}

#[utoipa::path(
    delete,
    path = "/api/v1/secrets/{id}",
    tag = "secrets",
    params(("id" = String, Path, description = "Secret ID")),
    responses(
        (status = 204, description = "Secret deleted"),
        (status = 404, description = "Secret not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_secret(
    State(secrets): State<Arc<SecretService>>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, SecretError> {
    secrets
        .delete_secret(&auth.scope_context(), &auth.user_id.to_string(), &id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/secrets/validate",
    tag = "secrets",
    request_body = ValidateSecretsRequest,
    responses(
        (status = 200, description = "Secret coverage validation", body = ValidateSecretsResponse)
    )
)]
pub async fn validate_secrets(
    State(secrets): State<Arc<SecretService>>,
    auth: AuthUser,
    Json(req): Json<ValidateSecretsRequest>,
) -> Result<Json<ValidateSecretsResponse>, SecretError> {
    let resp = secrets
        .validate_required(&auth.scope_context(), req)
        .await?;
    Ok(Json(resp))
}
