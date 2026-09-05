//! Skill HTTP handlers.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;

use super::error::SkillError;
use super::schemas::ListSkillsResponse;
use super::service::SkillService;
use moatless_core::http_auth::AuthUser;

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
///
/// `workspaceId` narrows the plugin half by that Workspace's own activation
/// records, which outrank both the global default and the caller's own. Pass
/// the Workspace the task runs in and the list is what its agent will have;
/// omit it and the list is what the caller gets anywhere.
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
