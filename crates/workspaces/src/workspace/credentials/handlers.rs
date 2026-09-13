//! Workspace credential HTTP handlers.
//!
//! Write-only values: `PUT` takes one, no route ever returns one. The provider
//! is a path segment rather than a body field so a credential has one stable
//! URL to `PUT` and `DELETE`.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use moatless_core::http_auth::AuthUser;

use super::models::WorkspaceCredentialMetadata;
use super::service::WorkspaceCredentialService;
use crate::workspace::error::WorkspaceError;
use crate::workspace::schemas::{SetWorkspaceCredentialRequest, WorkspaceCredentialResponse};
use moatless_workspace_types::workspace::WorkspaceCredentialProvider;

pub(super) fn credential_response(
    metadata: WorkspaceCredentialMetadata,
) -> WorkspaceCredentialResponse {
    WorkspaceCredentialResponse {
        workspace_id: metadata.workspace_id,
        provider: metadata.provider,
        env_key: metadata.env_key,
        created_by: metadata.created_by,
        updated_by: metadata.updated_by,
        created_at: metadata.created_at,
        updated_at: metadata.updated_at,
    }
}

/// Parse the `{provider}` path segment, naming the supported set on a miss.
fn parse_provider(provider: &str) -> Result<WorkspaceCredentialProvider, WorkspaceError> {
    WorkspaceCredentialProvider::try_from(provider).map_err(|_| {
        WorkspaceError::BadRequest(format!(
            "Unknown credential provider '{provider}'. Must be one of: {}",
            WorkspaceCredentialProvider::names()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::parse_provider;
    use moatless_workspace_types::workspace::WorkspaceCredentialProvider;

    #[test]
    fn an_unknown_provider_names_the_supported_set() {
        let err = parse_provider("codex").expect_err("codex is deliberately unsupported");
        let message = err.to_string();
        for provider in WorkspaceCredentialProvider::ALL {
            assert!(message.contains(provider.as_str()), "{message}");
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/workspaces/{workspace_id}/credentials",
    tag = "workspaces",
    params(("workspace_id" = String, Path, description = "Workspace id")),
    responses(
        (status = 200, description = "Credential metadata (never values)", body = Vec<WorkspaceCredentialResponse>),
        (status = 404, description = "Workspace not found", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn list_workspace_credentials(
    State(credentials): State<Arc<WorkspaceCredentialService>>,
    auth: AuthUser,
    Path(workspace_id): Path<String>,
) -> Result<Json<Vec<WorkspaceCredentialResponse>>, WorkspaceError> {
    let listed = credentials
        .list(&auth.scope_context(), &workspace_id)
        .await?;
    Ok(Json(listed.into_iter().map(credential_response).collect()))
}

#[utoipa::path(
    put,
    path = "/api/v1/workspaces/{workspace_id}/credentials/{provider}",
    tag = "workspaces",
    params(
        ("workspace_id" = String, Path, description = "Workspace id"),
        ("provider" = String, Path, description = "github | gitlab | gitness | openai | anthropic"),
    ),
    request_body = SetWorkspaceCredentialRequest,
    responses(
        (status = 200, description = "Credential stored", body = WorkspaceCredentialResponse),
        (status = 400, description = "Unknown provider or empty value", body = moatless_types::http::ErrorBody),
        (status = 403, description = "No control access to the workspace", body = moatless_types::http::ErrorBody),
        (status = 404, description = "Workspace not found", body = moatless_types::http::ErrorBody),
        (status = 503, description = "Secret encryption is not configured", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn set_workspace_credential(
    State(credentials): State<Arc<WorkspaceCredentialService>>,
    auth: AuthUser,
    Path((workspace_id, provider)): Path<(String, String)>,
    Json(req): Json<SetWorkspaceCredentialRequest>,
) -> Result<Json<WorkspaceCredentialResponse>, WorkspaceError> {
    let provider = parse_provider(&provider)?;
    let stored = credentials
        .set(&auth.scope_context(), &workspace_id, provider, &req.value)
        .await?;
    Ok(Json(credential_response(stored)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/workspaces/{workspace_id}/credentials/{provider}",
    tag = "workspaces",
    params(
        ("workspace_id" = String, Path, description = "Workspace id"),
        ("provider" = String, Path, description = "github | gitlab | gitness | openai | anthropic"),
    ),
    responses(
        (status = 204, description = "Credential removed"),
        (status = 400, description = "Unknown provider", body = moatless_types::http::ErrorBody),
        (status = 403, description = "No control access to the workspace", body = moatless_types::http::ErrorBody),
        (status = 404, description = "No such credential", body = moatless_types::http::ErrorBody)
    )
)]
pub async fn delete_workspace_credential(
    State(credentials): State<Arc<WorkspaceCredentialService>>,
    auth: AuthUser,
    Path((workspace_id, provider)): Path<(String, String)>,
) -> Result<StatusCode, WorkspaceError> {
    let provider = parse_provider(&provider)?;
    credentials
        .delete(&auth.scope_context(), &workspace_id, provider)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
