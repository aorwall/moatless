//! Tag HTTP handlers.

use axum::extract::State;
use axum::Json;

use crate::state::AppState;

use super::error::TagError;
use super::schemas::TagResponse;

/// List all tags.
#[utoipa::path(
    get,
    path = "/api/v1/tags",
    tag = "tags",
    responses(
        (status = 200, description = "List of tags", body = Vec<TagResponse>),
        (status = 500, description = "Internal error", body = crate::error::ErrorBody)
    )
)]
pub async fn list_tags(
    State(state): State<AppState>,
) -> Result<Json<Vec<TagResponse>>, TagError> {
    let rows = state.tags.list_all().await?;

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
