//! User settings API handlers.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use crate::error::ErrorBody;
use crate::extractors::AuthUser;
use crate::shared_models::Settings;
use crate::state::AppState;

/// Response for user settings — wraps the shared [`Settings`] struct.
pub type UserSettingsResponse = Settings;

/// Request to update user settings — wraps the shared [`Settings`] struct.
pub type UpdateUserSettingsRequest = Settings;

/// Get the current user's settings.
#[utoipa::path(
    get,
    path = "/api/v1/users/me/settings",
    tag = "user-settings",
    responses(
        (status = 200, description = "User settings", body = UserSettingsResponse),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 404, description = "User not found", body = ErrorBody),
    )
)]
pub async fn get_settings(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<UserSettingsResponse>, (StatusCode, Json<ErrorBody>)> {
    let user = state
        .users
        .get_user(&auth.user_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| not_found("User not found"))?;

    Ok(Json(user.parsed_settings()))
}

/// Update the current user's settings.
#[utoipa::path(
    patch,
    path = "/api/v1/users/me/settings",
    tag = "user-settings",
    request_body = UpdateUserSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = UserSettingsResponse),
        (status = 400, description = "Invalid request", body = ErrorBody),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 404, description = "User or repository not found", body = ErrorBody),
    )
)]
pub async fn update_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UpdateUserSettingsRequest>,
) -> Result<Json<UserSettingsResponse>, (StatusCode, Json<ErrorBody>)> {
    // Validate repository exists if provided
    if let Some(ref repo_id) = body.default_channel_repository_id {
        state
            .repositories
            .find_repository_by_id(repo_id)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| not_found(&format!("Repository not found: {repo_id}")))?;
    }

    let patch = serde_json::to_value(&body).map_err(internal_error)?;

    let user = state
        .users
        .update_user_settings(&auth.user_id, &patch)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| not_found("User not found"))?;

    Ok(Json(user.parsed_settings()))
}

fn internal_error(e: impl std::fmt::Display) -> (StatusCode, Json<ErrorBody>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: format!("Internal error: {e}"),
        }),
    )
}

fn not_found(msg: &str) -> (StatusCode, Json<ErrorBody>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorBody {
            error: msg.to_string(),
        }),
    )
}
