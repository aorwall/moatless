//! Skill HTTP handlers.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;

use super::error::SkillError;
use super::schemas::ListSkillsResponse;
use super::service::SkillService;
use moatless_identity::extractors::AuthUser;

/// List all skills discovered for a repository.
///
/// Returns skills from the workspace repo's own `.claude/skills/` directory
/// plus the plugin skills the caller's own activation records turn on,
/// deduplicated by name (workspace shadows plugin, first plugin wins on
/// plugin-vs-plugin clash).
///
/// The plugin half is per-caller: this is the same list that will be delivered
/// into that caller's sandbox, so two people can see different skills for the
/// same repository.
#[utoipa::path(
    get,
    path = "/api/v1/repositories/{repository_id}/skills",
    tag = "skills",
    params(
        ("repository_id" = String, Path, description = "Repository identifier")
    ),
    responses(
        (status = 200, description = "List of skills", body = ListSkillsResponse),
        (status = 404, description = "Repository not found", body = moatless_core::error::ErrorBody),
        (status = 502, description = "Failed to fetch a git repository", body = moatless_core::error::ErrorBody),
    )
)]
pub async fn list_repository_skills(
    State(skills): State<Arc<SkillService>>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
) -> Result<Json<ListSkillsResponse>, SkillError> {
    let skills = skills.list_for_repository(&repository_id, &auth).await?;
    Ok(Json(ListSkillsResponse { skills }))
}
