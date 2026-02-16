//! Command HTTP handlers.

use axum::extract::{Path, State};
use axum::Json;

use super::models::Command;
use crate::extractors::AuthUser;
use crate::state::AppState;

use super::defaults::default_commands;
use super::error::CommandError;
use super::schemas::*;

/// Create a new command.
#[utoipa::path(
    post,
    path = "/api/v1/commands",
    tag = "commands",
    request_body = CreateCommandRequest,
    responses(
        (status = 200, description = "Command created", body = CommandResponse),
        (status = 409, description = "Command already exists", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn create_command(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(req): Json<CreateCommandRequest>,
) -> Result<Json<CommandResponse>, CommandError> {
    // Check if command already exists
    if state.commands.find_by_name(&req.name).await?.is_some() {
        return Err(CommandError::AlreadyExists(req.name));
    }

    let row = Command {
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

    state.commands.insert(&row).await?;
    Ok(Json(CommandResponse::from(row)))
}

/// List all commands.
#[utoipa::path(
    get,
    path = "/api/v1/commands",
    tag = "commands",
    responses(
        (status = 200, description = "List of commands", body = Vec<CommandResponse>)
    )
)]
pub async fn list_commands(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<CommandResponse>>, CommandError> {
    ensure_defaults(&state).await?;
    let rows = state.commands.list_all().await?;
    let commands: Vec<CommandResponse> = rows.into_iter().map(CommandResponse::from).collect();
    Ok(Json(commands))
}

/// Get a command by name.
#[utoipa::path(
    get,
    path = "/api/v1/commands/{name}",
    tag = "commands",
    params(
        ("name" = String, Path, description = "Command name")
    ),
    responses(
        (status = 200, description = "Command details", body = CommandResponse),
        (status = 404, description = "Command not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn get_command(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(name): Path<String>,
) -> Result<Json<CommandResponse>, CommandError> {
    let row = state.commands.find_by_name(&name)
        .await?
        .ok_or_else(|| CommandError::NotFound(name))?;

    Ok(Json(CommandResponse::from(row)))
}

/// Update a command by name.
#[utoipa::path(
    patch,
    path = "/api/v1/commands/{name}",
    tag = "commands",
    params(
        ("name" = String, Path, description = "Command name")
    ),
    request_body = UpdateCommandRequest,
    responses(
        (status = 200, description = "Command updated", body = CommandResponse),
        (status = 404, description = "Command not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn update_command(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(name): Path<String>,
    Json(req): Json<UpdateCommandRequest>,
) -> Result<Json<CommandResponse>, CommandError> {
    let updated = state.commands.update_by_name(
        &name,
        req.description.as_deref(),
        req.prompt.as_deref(),
        req.conditions.as_ref(),
        req.icon.as_deref(),
        req.variant.as_deref(),
        req.active,
        req.will_not_resume,
        req.agent_mode.as_ref(),
    )
    .await?;

    if !updated {
        return Err(CommandError::NotFound(name));
    }

    let row = state.commands.find_by_name(&name)
        .await?
        .ok_or_else(|| CommandError::NotFound(name))?;

    Ok(Json(CommandResponse::from(row)))
}

/// Delete a command by name.
#[utoipa::path(
    delete,
    path = "/api/v1/commands/{name}",
    tag = "commands",
    params(
        ("name" = String, Path, description = "Command name")
    ),
    responses(
        (status = 204, description = "Command deleted"),
        (status = 404, description = "Command not found", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn delete_command(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(name): Path<String>,
) -> Result<axum::http::StatusCode, CommandError> {
    let deleted = state.commands.delete_by_name(&name).await?;
    if !deleted {
        return Err(CommandError::NotFound(name));
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Get commands available for a task (active commands only).
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/commands",
    tag = "commands",
    params(
        ("task_id" = String, Path, description = "Task identifier")
    ),
    responses(
        (status = 200, description = "Available commands", body = Vec<CommandResponse>)
    )
)]
pub async fn get_available_commands(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(_task_id): Path<String>,
) -> Result<Json<Vec<CommandResponse>>, CommandError> {
    ensure_defaults(&state).await?;
    let rows = state.commands.list_all().await?;
    let commands: Vec<CommandResponse> = rows
        .into_iter()
        .filter(|r| r.active.unwrap_or(true))
        .map(CommandResponse::from)
        .collect();
    Ok(Json(commands))
}

/// Seed default commands if table is empty.
async fn ensure_defaults(state: &AppState) -> Result<(), CommandError> {
    let count = state.commands.count().await?;
    if count == 0 {
        state.commands.insert_batch(&default_commands()).await?;
    }
    Ok(())
}
