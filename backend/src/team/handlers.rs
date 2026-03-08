//! Team HTTP handlers.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use super::error::TeamError;
use super::models::Team;
use super::schemas::{
    AddTeamMemberRequest, CreateTeamRequest, TeamCredentialResponse, TeamMemberResponse,
    TeamResponse, TeamSettingsResponse, UpdateTeamRequest, UpdateTeamSettingsRequest,
    UpsertTeamCredentialRequest,
};
use crate::extractors::AuthUser;
use crate::state::AppState;

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
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<TeamResponse>>, TeamError> {
    let teams = if auth.role.as_deref() == Some("admin") {
        state.teams.list_all().await?
    } else {
        state.teams.find_teams_for_user(&auth.user_id).await?
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
        (status = 409, description = "Slug already taken", body = crate::error::ErrorBody)
    )
)]
pub async fn create_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateTeamRequest>,
) -> Result<(StatusCode, Json<TeamResponse>), TeamError> {
    if state.teams.find_by_slug(&req.slug).await?.is_some() {
        return Err(TeamError::Conflict(format!("Slug '{}' already taken", req.slug)));
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

    state.teams.insert(&team).await?;
    // Add creator as admin member
    state.teams.add_member(&id, &auth.user_id, "admin").await?;

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
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<Json<TeamResponse>, TeamError> {
    let team = state.teams.find_by_id(&team_id).await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;

    // Non-admin must be a member
    if auth.role.as_deref() != Some("admin") {
        let is_member = state.teams.is_member(&team_id, &auth.user_id).await?;
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
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn update_team(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(team_id): Path<String>,
    Json(req): Json<UpdateTeamRequest>,
) -> Result<Json<TeamResponse>, TeamError> {
    let _existing = state.teams.find_by_id(&team_id).await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;

    state.teams.update(&team_id, &req.name, req.description.as_deref()).await?;

    let updated = state.teams.find_by_id(&team_id).await?
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
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_team(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<StatusCode, TeamError> {
    let _existing = state.teams.find_by_id(&team_id).await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;

    state.teams.delete(&team_id).await?;
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
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<Json<Vec<TeamMemberResponse>>, TeamError> {
    let members = state.teams.find_members(&team_id).await?;
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
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn add_member(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(team_id): Path<String>,
    Json(req): Json<AddTeamMemberRequest>,
) -> Result<StatusCode, TeamError> {
    let _existing = state.teams.find_by_id(&team_id).await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;

    let role = req.role.as_deref().unwrap_or("member");
    state.teams.add_member(&team_id, &req.user_id, role).await?;
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
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn remove_member(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path((team_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, TeamError> {
    let _existing = state.teams.find_by_id(&team_id).await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;

    state.teams.remove_member(&team_id, &user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Team settings endpoints ----

/// Get a team's settings.
#[utoipa::path(
    get,
    path = "/api/v1/teams/{team_id}/settings",
    tag = "teams",
    params(("team_id" = String, Path, description = "Team identifier")),
    responses(
        (status = 200, description = "Team settings", body = TeamSettingsResponse),
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<Json<TeamSettingsResponse>, TeamError> {
    require_team_access(&state, &auth, &team_id).await?;

    let team = state
        .teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id))?;

    let settings = team.parsed_settings();
    Ok(Json(TeamSettingsResponse {
        default_channel_repository_id: settings.default_channel_repository_id,
    }))
}

/// Update a team's settings.
#[utoipa::path(
    patch,
    path = "/api/v1/teams/{team_id}/settings",
    tag = "teams",
    params(("team_id" = String, Path, description = "Team identifier")),
    request_body = UpdateTeamSettingsRequest,
    responses(
        (status = 200, description = "Settings updated", body = TeamSettingsResponse),
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn update_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<String>,
    Json(body): Json<UpdateTeamSettingsRequest>,
) -> Result<Json<TeamSettingsResponse>, TeamError> {
    require_team_access(&state, &auth, &team_id).await?;

    // Validate repository exists if provided
    if let Some(ref repo_id) = body.default_channel_repository_id {
        state
            .repositories
            .find_repository_by_id(repo_id)
            .await
            .map_err(TeamError::Db)?
            .ok_or_else(|| TeamError::BadRequest(format!("Repository not found: {repo_id}")))?;
    }

    let patch = serde_json::json!({
        "default_channel_repository_id": body.default_channel_repository_id,
    });

    let team = state
        .teams
        .update_settings(&team_id, &patch)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id))?;

    let settings = team.parsed_settings();
    Ok(Json(TeamSettingsResponse {
        default_channel_repository_id: settings.default_channel_repository_id,
    }))
}

// ---- Team credential endpoints ----

/// List credentials for a team.
#[utoipa::path(
    get,
    path = "/api/v1/teams/{team_id}/credentials",
    tag = "teams",
    params(("team_id" = String, Path, description = "Team identifier")),
    responses(
        (status = 200, description = "List of team credentials", body = Vec<TeamCredentialResponse>),
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn list_credentials(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<Json<Vec<TeamCredentialResponse>>, TeamError> {
    require_team_access(&state, &auth, &team_id).await?;

    let tokens = state.provider_tokens.list_by_team(&team_id).await?;
    let resp: Vec<TeamCredentialResponse> = tokens
        .into_iter()
        .map(|t| TeamCredentialResponse {
            provider: t.provider,
            token_type: t.token_type,
            has_refresh_token: t.refresh_token.is_some(),
        })
        .collect();
    Ok(Json(resp))
}

/// Upsert a credential for a team.
#[utoipa::path(
    put,
    path = "/api/v1/teams/{team_id}/credentials/{provider}",
    tag = "teams",
    params(
        ("team_id" = String, Path, description = "Team identifier"),
        ("provider" = String, Path, description = "Provider name (github, gitlab, claude, etc.)"),
    ),
    request_body = UpsertTeamCredentialRequest,
    responses(
        (status = 204, description = "Credential upserted"),
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn upsert_credential(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, provider)): Path<(String, String)>,
    Json(req): Json<UpsertTeamCredentialRequest>,
) -> Result<StatusCode, TeamError> {
    require_team_access(&state, &auth, &team_id).await?;

    state
        .provider_tokens
        .upsert_team_token(
            &team_id,
            &provider,
            &req.access_token,
            req.refresh_token.as_deref(),
            req.token_type.as_deref(),
            req.metadata.as_ref(),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a credential for a team.
#[utoipa::path(
    delete,
    path = "/api/v1/teams/{team_id}/credentials/{provider}",
    tag = "teams",
    params(
        ("team_id" = String, Path, description = "Team identifier"),
        ("provider" = String, Path, description = "Provider name"),
    ),
    responses(
        (status = 204, description = "Credential deleted"),
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_credential(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, provider)): Path<(String, String)>,
) -> Result<StatusCode, TeamError> {
    require_team_access(&state, &auth, &team_id).await?;

    state
        .provider_tokens
        .delete_team_token(&team_id, &provider)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Verify the caller is an admin or a member of the team.
async fn require_team_access(
    state: &AppState,
    auth: &AuthUser,
    team_id: &str,
) -> Result<(), TeamError> {
    let _existing = state
        .teams
        .find_by_id(team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.to_string()))?;

    if auth.role.as_deref() != Some("admin") {
        let is_member = state.teams.is_member(team_id, &auth.user_id).await?;
        if !is_member {
            return Err(TeamError::Forbidden("Not a team member".into()));
        }
    }
    Ok(())
}
