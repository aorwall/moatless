//! Skill HTTP handlers.

use axum::extract::{Path, State};
use axum::Json;

use super::error::SkillError;
use super::schemas::ListSkillsResponse;
use crate::extractors::AuthUser;
use crate::state::AppState;

/// List all skills discovered for a repository.
///
/// Returns skills from the workspace repo's own `.claude/skills/` directory
/// plus skills from every linked plugin, deduplicated by name
/// (workspace shadows plugin, first plugin wins on plugin-vs-plugin clash).
#[utoipa::path(
    get,
    path = "/api/v1/repositories/{repository_id}/skills",
    tag = "skills",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    responses(
        (status = 200, description = "List of skills", body = ListSkillsResponse),
        (status = 404, description = "Repository not found", body = crate::error::ErrorBody),
        (status = 502, description = "Failed to fetch a git repository", body = crate::error::ErrorBody),
    )
)]
pub async fn list_repository_skills(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<ListSkillsResponse>, SkillError> {
    let skills = state
        .skills
        .list_for_repository(&repository_id, &auth)
        .await?;
    Ok(Json(ListSkillsResponse { skills }))
}
