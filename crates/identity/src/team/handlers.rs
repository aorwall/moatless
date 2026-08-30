//! Team HTTP handlers.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use super::error::TeamError;
use super::models::Team;
use super::schemas::{
    AddTeamMemberRequest, CreateTeamRequest, TeamMemberResponse, TeamResponse, UpdateTeamRequest,
};
use std::sync::Arc;

use moatless_core::http_auth::AuthUser;
use crate::team::dao::TeamDao;

/// List all teams the user belongs to.
#[utoipa::path(
    get,
    path = "/api/v1/teams",
    tag = "teams",
    responses(
        (status = 200, description = "List of teams", body = Vec<TeamResponse>)
    )
)]
pub async fn list_teams(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
) -> Result<Json<Vec<TeamResponse>>, TeamError> {
    let teams = if auth.role.as_deref() == Some("admin") {
        teams.list_all().await?
    } else {
        teams.find_teams_for_user(&auth.user_id).await?
    };
    let resp: Vec<TeamResponse> = teams.into_iter().map(Into::into).collect();
    Ok(Json(resp))
}

/// Create a new team.
#[utoipa::path(
    post,
    path = "/api/v1/teams",
    tag = "teams",
    request_body = CreateTeamRequest,
    responses(
        (status = 201, description = "Team created", body = TeamResponse),
        (status = 409, description = "Slug already taken", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn create_team(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Json(req): Json<CreateTeamRequest>,
) -> Result<(StatusCode, Json<TeamResponse>), TeamError> {
    if teams.find_by_slug(&req.slug).await?.is_some() {
        return Err(TeamError::Conflict(format!(
            "Slug '{}' already taken",
            req.slug
        )));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let team = Team {
        id: id.clone(),
        name: req.name,
        slug: req.slug,
        description: req.description,
        external_id: None,
        external_provider: None,
        is_default: false,
        settings: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };

    teams.insert(&team).await?;
    // Add creator as owner member
    teams.add_member(&id, &auth.user_id, "owner").await?;

    Ok((StatusCode::CREATED, Json(team.into())))
}

/// Get a team by ID.
#[utoipa::path(
    get,
    path = "/api/v1/teams/{team_id}",
    tag = "teams",
    params(("team_id" = String, Path, description = "Team identifier")),
    responses(
        (status = 200, description = "Team details", body = TeamResponse),
        (status = 404, description = "Team not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_team(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<Json<TeamResponse>, TeamError> {
    let team = teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;

    // Non-admin must be a member
    if auth.role.as_deref() != Some("admin") {
        let is_member = teams.is_member(&team_id, &auth.user_id).await?;
        if !is_member {
            return Err(TeamError::Forbidden("Not a team member".into()));
        }
    }

    Ok(Json(team.into()))
}

/// Update a team.
#[utoipa::path(
    put,
    path = "/api/v1/teams/{team_id}",
    tag = "teams",
    params(("team_id" = String, Path, description = "Team identifier")),
    request_body = UpdateTeamRequest,
    responses(
        (status = 200, description = "Team updated", body = TeamResponse),
        (status = 404, description = "Team not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn update_team(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Path(team_id): Path<String>,
    Json(req): Json<UpdateTeamRequest>,
) -> Result<Json<TeamResponse>, TeamError> {
    let _existing = teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(&teams, &auth, &team_id).await?;

    teams
        .update(&team_id, &req.name, req.description.as_deref())
        .await?;

    let updated = teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id))?;

    Ok(Json(updated.into()))
}

/// Delete a team.
#[utoipa::path(
    delete,
    path = "/api/v1/teams/{team_id}",
    tag = "teams",
    params(("team_id" = String, Path, description = "Team identifier")),
    responses(
        (status = 204, description = "Team deleted"),
        (status = 404, description = "Team not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_team(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<StatusCode, TeamError> {
    let _existing = teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(&teams, &auth, &team_id).await?;

    teams.delete(&team_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List members of a team.
#[utoipa::path(
    get,
    path = "/api/v1/teams/{team_id}/members",
    tag = "teams",
    params(("team_id" = String, Path, description = "Team identifier")),
    responses(
        (status = 200, description = "List of team members", body = Vec<TeamMemberResponse>)
    )
)]
pub async fn list_members(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<Json<Vec<TeamMemberResponse>>, TeamError> {
    require_team_access(&teams, &auth, &team_id).await?;
    let members = teams.find_members(&team_id).await?;
    let resp: Vec<TeamMemberResponse> = members.into_iter().map(Into::into).collect();
    Ok(Json(resp))
}

/// Add a member to a team.
#[utoipa::path(
    post,
    path = "/api/v1/teams/{team_id}/members",
    tag = "teams",
    params(("team_id" = String, Path, description = "Team identifier")),
    request_body = AddTeamMemberRequest,
    responses(
        (status = 204, description = "Member added"),
        (status = 404, description = "Team not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn add_member(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Path(team_id): Path<String>,
    Json(req): Json<AddTeamMemberRequest>,
) -> Result<StatusCode, TeamError> {
    let _existing = teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(&teams, &auth, &team_id).await?;

    let role = req.role.as_deref().unwrap_or("member");
    let user_id = req
        .user_id
        .parse::<uuid::Uuid>()
        .map_err(|_| TeamError::BadRequest("user_id must be a uuid".to_string()))?;
    teams.add_member(&team_id, &user_id, role).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Remove a member from a team.
#[utoipa::path(
    delete,
    path = "/api/v1/teams/{team_id}/members/{user_id}",
    tag = "teams",
    params(
        ("team_id" = String, Path, description = "Team identifier"),
        ("user_id" = String, Path, description = "User identifier"),
    ),
    responses(
        (status = 204, description = "Member removed"),
        (status = 404, description = "Team not found", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn remove_member(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Path((team_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, TeamError> {
    let _existing = teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(&teams, &auth, &team_id).await?;

    let user_id = user_id
        .parse::<uuid::Uuid>()
        .map_err(|_| TeamError::NotFound(format!("User '{user_id}' not found")))?;
    teams.remove_member(&team_id, &user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Verify the caller is an admin or an owner/admin member of the team.
async fn ensure_team_manager(
    teams: &Arc<dyn TeamDao>,
    auth: &AuthUser,
    team_id: &str,
) -> Result<(), TeamError> {
    if auth.role.as_deref() == Some("admin") {
        return Ok(());
    }
    let members = teams.find_members(team_id).await?;
    let caller_role = members
        .iter()
        .find(|m| m.user_id == auth.user_id)
        .map(|m| m.role.as_str());
    match caller_role {
        Some("owner") | Some("admin") => Ok(()),
        _ => Err(TeamError::Forbidden("Must be a team owner or admin".into())),
    }
}

/// Verify the caller is an admin or a member of the team.
async fn require_team_access(
    teams: &Arc<dyn TeamDao>,
    auth: &AuthUser,
    team_id: &str,
) -> Result<(), TeamError> {
    let _existing = teams
        .find_by_id(team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.to_string()))?;

    if auth.role.as_deref() != Some("admin") {
        let is_member = teams.is_member(team_id, &auth.user_id).await?;
        if !is_member {
            return Err(TeamError::Forbidden("Not a team member".into()));
        }
    }
    Ok(())
}
