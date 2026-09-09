//! Team HTTP handlers.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use super::error::TeamError;
use super::models::TeamCaller;
use super::operations;
use super::schemas::{
    AddTeamMemberRequest, CreateTeamRequest, TeamMemberResponse, TeamResponse, UpdateTeamRequest,
};
use std::sync::Arc;

use crate::team::dao::TeamDao;
use moatless_core::http_auth::AuthUser;

/// List all teams the user belongs to.
#[utoipa::path(
    get,
    path = "/api/v1/teams",
    tag = "teams",
    responses(
        (status = 200, description = "List of teams", body = Vec<TeamResponse>)
    )
)]
pub(crate) async fn list_teams(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
) -> Result<Json<Vec<TeamResponse>>, TeamError> {
    let caller = TeamCaller {
        user_id: auth.user_id,
        is_admin: auth.role.as_deref() == Some("admin"),
    };
    let result = operations::list_teams(teams.as_ref(), &caller).await?;
    Ok(Json(result.into_iter().map(Into::into).collect()))
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
pub(crate) async fn create_team(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Json(req): Json<CreateTeamRequest>,
) -> Result<(StatusCode, Json<TeamResponse>), TeamError> {
    let caller = TeamCaller {
        user_id: auth.user_id,
        is_admin: auth.role.as_deref() == Some("admin"),
    };
    let result = operations::create_team(teams.as_ref(), &caller, req).await?;
    Ok((StatusCode::CREATED, Json(result.into())))
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
pub(crate) async fn get_team(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<Json<TeamResponse>, TeamError> {
    let caller = TeamCaller {
        user_id: auth.user_id,
        is_admin: auth.role.as_deref() == Some("admin"),
    };
    let result = operations::get_team(teams.as_ref(), &caller, team_id).await?;
    Ok(Json(result.into()))
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
pub(crate) async fn update_team(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Path(team_id): Path<String>,
    Json(req): Json<UpdateTeamRequest>,
) -> Result<Json<TeamResponse>, TeamError> {
    let caller = TeamCaller {
        user_id: auth.user_id,
        is_admin: auth.role.as_deref() == Some("admin"),
    };
    let result = operations::update_team(teams.as_ref(), &caller, team_id, req).await?;
    Ok(Json(result.into()))
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
pub(crate) async fn delete_team(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<StatusCode, TeamError> {
    let caller = TeamCaller {
        user_id: auth.user_id,
        is_admin: auth.role.as_deref() == Some("admin"),
    };
    operations::delete_team(teams.as_ref(), &caller, team_id).await?;
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
pub(crate) async fn list_members(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<Json<Vec<TeamMemberResponse>>, TeamError> {
    let caller = TeamCaller {
        user_id: auth.user_id,
        is_admin: auth.role.as_deref() == Some("admin"),
    };
    let result = operations::list_members(teams.as_ref(), &caller, team_id).await?;
    Ok(Json(result.into_iter().map(Into::into).collect()))
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
pub(crate) async fn add_member(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Path(team_id): Path<String>,
    Json(req): Json<AddTeamMemberRequest>,
) -> Result<StatusCode, TeamError> {
    let caller = TeamCaller {
        user_id: auth.user_id,
        is_admin: auth.role.as_deref() == Some("admin"),
    };
    operations::add_member(teams.as_ref(), &caller, team_id, req).await?;
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
pub(crate) async fn remove_member(
    State(teams): State<Arc<dyn TeamDao>>,
    auth: AuthUser,
    Path((team_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, TeamError> {
    let caller = TeamCaller {
        user_id: auth.user_id,
        is_admin: auth.role.as_deref() == Some("admin"),
    };
    operations::remove_member(teams.as_ref(), &caller, team_id, user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
