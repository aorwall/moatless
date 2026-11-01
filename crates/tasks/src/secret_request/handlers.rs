//! Secret request HTTP handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use moatless_core::http_auth::AuthUser;

use super::error::SecretRequestError;
use super::schemas::{CreateSecretRequestRequest, SecretRequestResponse};
use super::service::{NewSecretRequest, SecretRequests};

/// Ask a person for a secret in the Task's thread. Answered in T3's secret
/// card; the Task is messaged when it is.
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/secret-requests",
    tag = "secret-requests",
    params(("task_id" = String, Path, description = "Task identifier")),
    request_body = CreateSecretRequestRequest,
    responses(
        (status = 200, description = "Request shown in the thread", body = SecretRequestResponse),
        (status = 400, description = "Invalid request", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Task not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn create_secret_request(
    State(requests): State<Arc<SecretRequests>>,
    auth: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<CreateSecretRequestRequest>,
) -> Result<Json<SecretRequestResponse>, SecretRequestError> {
    let asked = requests
        .request(
            &auth.scope_context(),
            &task_id,
            NewSecretRequest {
                key: req.key,
                scope: req.scope,
                kind: req.kind,
                label: req.label,
                reason: req.reason,
            },
        )
        .await?;
    Ok(Json(SecretRequestResponse {
        request_id: asked.request_id,
        key: asked.key,
        scope: asked.scope,
        kind: asked.kind,
        label: asked.label,
    }))
}
