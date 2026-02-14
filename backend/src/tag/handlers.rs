//! Tag HTTP handlers.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::state::AppState;

use super::schemas::TagResponse;

/// List all tags.
#[utoipa::path(
    get,
    path = "/api/v1/tags",
    tag = "tags",
    responses(
        (status = 200, description = "List of tags", body = Vec<TagResponse>),
        (status = 500, description = "Internal error", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn list_tags(
    State(state): State<AppState>,
) -> Result<Json<Vec<TagResponse>>, (StatusCode, String)> {
    let rows = state.tags.list_all()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {e}"),
            )
        })?;

    let tags: Vec<TagResponse> = rows
        .into_iter()
        .map(|r| TagResponse {
            tag_id: r.tag_id,
            name: r.name,
            description: r.description,
        })
        .collect();

    Ok(Json(tags))
}
