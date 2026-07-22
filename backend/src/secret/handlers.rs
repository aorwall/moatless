//! Secret HTTP handlers.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;

use crate::extractors::AuthUser;
use crate::state::AppState;

use super::error::SecretError;
use super::schemas::{
    ListSecretsQuery, PatchSecretRequest, PutSecretRequest, ResolveSecretsRequest,
    ResolveSecretsResponse, SecretMetadataResponse, SecretMutationResponse, ValidateSecretsRequest,
    ValidateSecretsResponse,
};

#[utoipa::path(
    get,
    path = "/api/v1/secrets",
    tag = "secrets",
    params(ListSecretsQuery),
    responses(
        (status = 200, description = "List secret metadata", body = Vec<SecretMetadataResponse>),
        (status = 403, description = "Forbidden", body = crate::error::ErrorBody)
    )
)]
pub async fn list_secrets(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListSecretsQuery>,
) -> Result<Json<Vec<SecretMetadataResponse>>, SecretError> {
    let rows = state
        .secrets
        .list_metadata(
            &auth.scope_context(),
            query.scope_type,
            query.scope_id.as_deref(),
        )
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
        (status = 404, description = "Secret not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_secret(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<SecretMetadataResponse>, SecretError> {
    let row = state
        .secrets
        .get_metadata(&auth.scope_context(), &id)
        .await?;
    Ok(Json(row))
}

#[utoipa::path(
    put,
    path = "/api/v1/secrets",
    tag = "secrets",
    request_body = PutSecretRequest,
    responses(
        (status = 200, description = "Secret created or replaced", body = SecretMutationResponse),
        (status = 400, description = "Invalid secret", body = crate::error::ErrorBody)
    )
)]
pub async fn put_secret(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<PutSecretRequest>,
) -> Result<Json<SecretMutationResponse>, SecretError> {
    let row = state
        .secrets
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
        (status = 404, description = "Secret not found", body = crate::error::ErrorBody)
    )
)]
pub async fn patch_secret(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<PatchSecretRequest>,
) -> Result<Json<SecretMutationResponse>, SecretError> {
    let row = state
        .secrets
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
        (status = 404, description = "Secret not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_secret(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, SecretError> {
    state
        .secrets
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
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ValidateSecretsRequest>,
) -> Result<Json<ValidateSecretsResponse>, SecretError> {
    let resp = state
        .secrets
        .validate_required(&auth.scope_context(), req)
        .await?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    path = "/api/v1/internal/secrets/resolve",
    tag = "secrets",
    request_body = ResolveSecretsRequest,
    responses(
        (status = 200, description = "Resolved task-scoped credentials", body = ResolveSecretsResponse),
        (status = 403, description = "Forbidden", body = crate::error::ErrorBody)
    )
)]
pub async fn resolve_task_secrets(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ResolveSecretsRequest>,
) -> Result<Json<ResolveSecretsResponse>, SecretError> {
    if auth.role.as_deref() != Some("admin") {
        return Err(SecretError::Forbidden(
            "internal secret resolution requires admin access".to_string(),
        ));
    }

    let task = state
        .tasks
        .find_by_id_internal(&req.task_id)
        .await?
        .ok_or_else(|| SecretError::BadRequest(format!("task not found: {}", req.task_id)))?;
    let credentials = state
        .secrets
        .resolve_accessible_plaintext(
            task.created_by.as_deref().unwrap_or(&auth.user_id.to_string()),
            task.team_id.as_deref(),
        )
        .await?;
    Ok(Json(ResolveSecretsResponse { credentials }))
}
