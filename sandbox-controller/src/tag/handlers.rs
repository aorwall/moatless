//! Tag HTTP handlers.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use crate::db::tag_dao;
use crate::state::AppState;

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagResponse {
    pub tag_id: String,
    pub name: String,
    pub description: Option<String>,
}

/// GET /api/v1/tags
pub async fn list_tags(
    State(state): State<AppState>,
) -> Result<Json<Vec<TagResponse>>, (StatusCode, String)> {
    let rows = tag_dao::list_all(&state.db)
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
