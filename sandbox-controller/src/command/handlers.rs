//! Command HTTP handlers.

use axum::extract::{Path, State};
use axum::Json;

use crate::db::command_dao;
use crate::db::models::CommandRow;
use crate::extractors::AuthUser;
use crate::state::AppState;

use super::defaults::default_commands;
use super::error::CommandError;
use super::types::*;

/// POST /api/v1/commands
pub async fn create_command(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(req): Json<CreateCommandRequest>,
) -> Result<Json<CommandResponse>, CommandError> {
    // Check if command already exists
    if command_dao::find_by_name(&state.db, &req.name).await?.is_some() {
        return Err(CommandError::AlreadyExists(req.name));
    }

    let row = CommandRow {
        name: req.name,
        description: req.description,
        prompt: req.prompt,
        conditions: req.conditions,
        icon: req.icon,
        variant: req.variant,
        active: req.active.or(Some(true)),
        will_not_resume: req.will_not_resume,
        agent_mode: req.agent_mode,
    };

    command_dao::insert(&state.db, &row).await?;
    Ok(Json(CommandResponse::from(row)))
}

/// GET /api/v1/commands
pub async fn list_commands(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<CommandResponse>>, CommandError> {
    ensure_defaults(&state).await?;
    let rows = command_dao::list_all(&state.db).await?;
    let commands: Vec<CommandResponse> = rows.into_iter().map(CommandResponse::from).collect();
    Ok(Json(commands))
}

/// GET /api/v1/commands/{name}
pub async fn get_command(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(name): Path<String>,
) -> Result<Json<CommandResponse>, CommandError> {
    let row = command_dao::find_by_name(&state.db, &name)
        .await?
        .ok_or_else(|| CommandError::NotFound(name))?;

    Ok(Json(CommandResponse::from(row)))
}

/// PATCH /api/v1/commands/{name}
pub async fn update_command(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(name): Path<String>,
    Json(req): Json<UpdateCommandRequest>,
) -> Result<Json<CommandResponse>, CommandError> {
    let updated = command_dao::update_by_name(
        &state.db,
        &name,
        req.description.as_deref(),
        req.prompt.as_deref(),
        req.conditions.as_ref(),
        req.icon.as_deref(),
        req.variant.as_deref(),
        req.active,
        req.will_not_resume,
        req.agent_mode.as_deref(),
    )
    .await?;

    if !updated {
        return Err(CommandError::NotFound(name));
    }

    let row = command_dao::find_by_name(&state.db, &name)
        .await?
        .ok_or_else(|| CommandError::NotFound(name))?;

    Ok(Json(CommandResponse::from(row)))
}

/// DELETE /api/v1/commands/{name}
pub async fn delete_command(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(name): Path<String>,
) -> Result<axum::http::StatusCode, CommandError> {
    let deleted = command_dao::delete_by_name(&state.db, &name).await?;
    if !deleted {
        return Err(CommandError::NotFound(name));
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// GET /api/v1/tasks/{task_id}/commands
pub async fn get_available_commands(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(_task_id): Path<String>,
) -> Result<Json<Vec<CommandResponse>>, CommandError> {
    ensure_defaults(&state).await?;
    let rows = command_dao::list_all(&state.db).await?;
    let commands: Vec<CommandResponse> = rows
        .into_iter()
        .filter(|r| r.active.unwrap_or(true))
        .map(CommandResponse::from)
        .collect();
    Ok(Json(commands))
}

/// Seed default commands if table is empty.
async fn ensure_defaults(state: &AppState) -> Result<(), CommandError> {
    let count = command_dao::count(&state.db).await?;
    if count == 0 {
        command_dao::insert_batch(&state.db, &default_commands()).await?;
    }
    Ok(())
}
