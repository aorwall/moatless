//! Team HTTP handlers.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use super::error::TeamError;
use super::models::Team;
use super::schemas::{
    AddTeamMemberRequest, CreateTeamRequest, TeamCredentialResponse, TeamMemberResponse,
    TeamResponse, UpdateTeamRequest, UpsertTeamCredentialRequest,
};
use crate::access_control::ensure_team_manager;
use crate::credentials::codex::CodexScope;
use crate::extractors::AuthUser;
use crate::state::AppState;
use moatless_api_schemas::provider_settings::{CodexConfigResponse, SaveCodexAuthRequest};

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

    state.teams.insert(&team).await?;
    // Add creator as owner member
    state.teams.add_member(&id, &auth.user_id, "owner").await?;

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
    let team = state
        .teams
        .find_by_id(&team_id)
        .await?
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
    auth: AuthUser,
    Path(team_id): Path<String>,
    Json(req): Json<UpdateTeamRequest>,
) -> Result<Json<TeamResponse>, TeamError> {
    let _existing = state
        .teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(&state, &auth, &team_id).await?;

    state
        .teams
        .update(&team_id, &req.name, req.description.as_deref())
        .await?;

    let updated = state
        .teams
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
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_team(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<StatusCode, TeamError> {
    let _existing = state
        .teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(&state, &auth, &team_id).await?;

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
    auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<Json<Vec<TeamMemberResponse>>, TeamError> {
    require_team_access(&state, &auth, &team_id).await?;
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
    auth: AuthUser,
    Path(team_id): Path<String>,
    Json(req): Json<AddTeamMemberRequest>,
) -> Result<StatusCode, TeamError> {
    let _existing = state
        .teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(&state, &auth, &team_id).await?;

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
    auth: AuthUser,
    Path((team_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, TeamError> {
    let _existing = state
        .teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(&state, &auth, &team_id).await?;

    state.teams.remove_member(&team_id, &user_id).await?;
    Ok(StatusCode::NO_CONTENT)
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
    ensure_team_manager(&state, &auth, &team_id).await?;

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
    ensure_team_manager(&state, &auth, &team_id).await?;

    state
        .provider_tokens
        .upsert_team_token(
            &team_id,
            &provider,
            "manual",
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

// ---- Team codex provider-token endpoints ----

/// Save a team-scoped Codex `auth.json`.
///
/// The team blob shadows the global blob but is itself shadowed by any
/// member's user-scoped row (precedence: user > team > global). Resolution
/// + just-in-time refresh share one code path with the user / global
/// endpoints via [`crate::credentials::codex::save_codex_auth_for_scope`].
#[utoipa::path(
    put,
    path = "/api/v1/teams/{team_id}/settings/codex/config",
    tag = "teams",
    params(("team_id" = String, Path, description = "Team identifier")),
    request_body = SaveCodexAuthRequest,
    responses(
        (status = 200, description = "Team codex auth saved", body = CodexConfigResponse),
        (status = 400, description = "Invalid auth JSON", body = crate::error::ErrorBody),
        (status = 403, description = "Not a team manager", body = crate::error::ErrorBody),
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn put_team_codex_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<String>,
    Json(req): Json<SaveCodexAuthRequest>,
) -> Result<Json<CodexConfigResponse>, TeamError> {
    let _existing = state
        .teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(&state, &auth, &team_id).await?;

    let resp = crate::credentials::codex::save_codex_auth_for_scope(
        &state,
        CodexScope::team(&team_id),
        req,
    )
    .await
    .map_err(team_error_from_provider_settings)?;
    Ok(Json(resp))
}

/// Delete the team's Codex blob.
#[utoipa::path(
    delete,
    path = "/api/v1/teams/{team_id}/settings/codex/config",
    tag = "teams",
    params(("team_id" = String, Path, description = "Team identifier")),
    responses(
        (status = 204, description = "Team codex disconnected"),
        (status = 403, description = "Not a team manager", body = crate::error::ErrorBody),
        (status = 404, description = "Team not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_team_codex_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<String>,
) -> Result<StatusCode, TeamError> {
    let _existing = state
        .teams
        .find_by_id(&team_id)
        .await?
        .ok_or_else(|| TeamError::NotFound(team_id.clone()))?;
    ensure_team_manager(&state, &auth, &team_id).await?;

    state
        .provider_tokens
        .delete_codex_blob(&CodexScope::team(&team_id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn team_error_from_provider_settings(
    err: crate::provider_settings::error::ProviderSettingsError,
) -> TeamError {
    use crate::provider_settings::error::ProviderSettingsError;
    match err {
        ProviderSettingsError::BadRequest(m) => TeamError::BadRequest(m),
        ProviderSettingsError::Forbidden(m) => TeamError::Forbidden(m),
        ProviderSettingsError::NotFound(m) => TeamError::NotFound(m),
        ProviderSettingsError::Db(e) => TeamError::Db(e),
        ProviderSettingsError::ProviderError(m) => TeamError::BadRequest(m),
        ProviderSettingsError::Secret(e) => TeamError::Internal(e.to_string()),
    }
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
