//! Tag HTTP handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use super::error::TagError;
use super::operations::Tags;
use super::schemas::TagResponse;

/// List all tags.
#[utoipa::path(
    get,
    path = "/api/v1/tags",
    tag = "tags",
    responses(
        (status = 200, description = "List of tags", body = Vec<TagResponse>),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_tags(State(tags): State<Arc<Tags>>) -> Result<Json<Vec<TagResponse>>, TagError> {
    let rows = tags.list_all().await?;

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
