//! Skill HTTP handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};

use super::error::SkillError;
use super::schemas::ListSkillsResponse;
use super::service::SkillService;
use moatless_core::http_auth::AuthUser;

/// List all skills discovered for a repository.
///
/// Combines the repo's `.claude/skills/` with the caller's plugin activations
/// (workspace shadows plugin, first plugin wins a clash) — the per-caller list
/// also delivered into their sandbox. `workspaceId` narrows the plugin half to
/// that Workspace's own records, which outrank the global default and the
/// caller's own.
#[utoipa::path(
    get,
    path = "/api/v1/repositories/{repository_id}/skills",
    tag = "skills",
    params(
        ("repository_id" = String, Path, description = "Repository identifier"),
        ("workspaceId" = Option<String>, Query, description = "Narrow to what loads in this Workspace"),
    ),
    responses(
        (status = 200, description = "List of skills", body = ListSkillsResponse),
        (status = 404, description = "Repository or workspace not found", body = moatless_api_schemas::error::ErrorBody),
        (status = 502, description = "Failed to fetch a git repository", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn list_repository_skills(
    State(skills): State<Arc<SkillService>>,
    auth: AuthUser,
    Path(repository_id): Path<String>,
    Query(params): Query<ListSkillsParams>,
) -> Result<Json<ListSkillsResponse>, SkillError> {
    let skills = skills
        .list_for_repository(
            &repository_id,
            params.workspace_id.as_deref(),
            auth.user_id,
            &auth.scope_context(),
        )
        .await?;
    Ok(Json(ListSkillsResponse { skills }))
}

/// Which Workspace the listing is for, when it is for one.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSkillsParams {
    #[serde(default)]
    pub workspace_id: Option<String>,
}
