//! Task template HTTP handlers.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use tracing::info;

use crate::access_control::{ensure_can_write_resource, resolve_scope_for_create};
use crate::extractors::AuthUser;
use crate::scope::ScopedResource;
use crate::state::AppState;

use super::dao::TaskTemplatePatch;
use super::error::TaskTemplateError;
use super::models::TaskTemplate;
use super::schemas::*;

/// Create a new task template.
#[utoipa::path(
    post,
    path = "/api/v1/task-templates",
    tag = "task-templates",
    request_body = CreateTaskTemplateRequest,
    responses(
        (status = 201, description = "Task template created", body = TaskTemplateResponse),
        (status = 403, description = "Access denied", body = crate::error::ErrorBody)
    )
)]
pub async fn create_task_template(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateTaskTemplateRequest>,
) -> Result<(StatusCode, Json<TaskTemplateResponse>), TaskTemplateError> {
    let resolved_scope =
        resolve_scope_for_create(&state, &auth, req.scope, req.team_id.clone()).await?;

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let skills_json = req
        .skills
        .map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null));

    let row = TaskTemplate {
        id: id.clone(),
        name: req.name,
        description: req.description,
        icon: req.icon,
        color: req.color,
        repository_id: req.repository_id,
        agent_type: req.agent_type,
        agent_mode: req.agent_mode,
        default_model: req.default_model,
        initial_prompt: req.initial_prompt,
        skills: skills_json,
        scope: resolved_scope.scope,
        team_id: Some(resolved_scope.team_id),
        owner_user_id: resolved_scope.owner_user_id,
        featured: req.featured.unwrap_or(false),
        display_order: req.display_order.unwrap_or(0),
        created_by: Some(auth.user_id.clone()),
        created_at: now,
        updated_at: now,
    };

    state.task_templates.insert(&row).await?;
    info!(template_id = %id, user_id = %auth.user_id, "Task template created");

    Ok((StatusCode::CREATED, Json(row.into())))
}

/// List task templates visible to the current user.
#[utoipa::path(
    get,
    path = "/api/v1/task-templates",
    tag = "task-templates",
    responses(
        (status = 200, description = "List of task templates", body = Vec<TaskTemplateResponse>)
    )
)]
pub async fn list_task_templates(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<TaskTemplateResponse>>, TaskTemplateError> {
    let rows = state
        .task_templates
        .list_for_user(&auth.scope_context())
        .await?;
    let resp: Vec<TaskTemplateResponse> = rows.into_iter().map(Into::into).collect();
    Ok(Json(resp))
}

/// Query parameters for the featured chip strip endpoint.
#[derive(Debug, Deserialize)]
pub struct FeaturedQuery {
    pub limit: Option<i64>,
}

/// Get featured templates for the chip strip (recent-used first, then featured).
#[utoipa::path(
    get,
    path = "/api/v1/task-templates/featured",
    tag = "task-templates",
    params(
        ("limit" = Option<i64>, Query, description = "Maximum number of templates (default 6)")
    ),
    responses(
        (status = 200, description = "Featured templates for chip strip", body = Vec<TaskTemplateResponse>)
    )
)]
pub async fn list_featured_task_templates(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<FeaturedQuery>,
) -> Result<Json<Vec<TaskTemplateResponse>>, TaskTemplateError> {
    let limit = query.limit.unwrap_or(6).min(20);
    let rows = state
        .task_templates
        .list_featured_for_chips(&auth.scope_context(), limit)
        .await?;
    let resp: Vec<TaskTemplateResponse> = rows.into_iter().map(Into::into).collect();
    Ok(Json(resp))
}

/// Get a task template by ID.
#[utoipa::path(
    get,
    path = "/api/v1/task-templates/{template_id}",
    tag = "task-templates",
    params(
        ("template_id" = String, Path, description = "Template identifier")
    ),
    responses(
        (status = 200, description = "Task template details", body = TaskTemplateResponse),
        (status = 404, description = "Template not found", body = crate::error::ErrorBody)
    )
)]
pub async fn get_task_template(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(template_id): Path<String>,
) -> Result<Json<TaskTemplateResponse>, TaskTemplateError> {
    let row = state
        .task_templates
        .find_by_id(&template_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskTemplateError::NotFound(template_id))?;

    Ok(Json(row.into()))
}

/// Update a task template by ID.
#[utoipa::path(
    patch,
    path = "/api/v1/task-templates/{template_id}",
    tag = "task-templates",
    params(
        ("template_id" = String, Path, description = "Template identifier")
    ),
    request_body = UpdateTaskTemplateRequest,
    responses(
        (status = 200, description = "Task template updated", body = TaskTemplateResponse),
        (status = 403, description = "Access denied", body = crate::error::ErrorBody),
        (status = 404, description = "Template not found", body = crate::error::ErrorBody)
    )
)]
pub async fn update_task_template(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(template_id): Path<String>,
    Json(req): Json<UpdateTaskTemplateRequest>,
) -> Result<Json<TaskTemplateResponse>, TaskTemplateError> {
    let existing = state
        .task_templates
        .find_by_id_internal(&template_id)
        .await?
        .ok_or_else(|| TaskTemplateError::NotFound(template_id.clone()))?;

    ensure_can_write_resource(
        &state,
        &auth,
        &template_resource(&existing),
        "task template",
    )
    .await?;

    let resolved_scope = if req.scope.is_some() || req.team_id.is_some() {
        Some(resolve_scope_for_create(&state, &auth, req.scope, req.team_id.clone()).await?)
    } else {
        None
    };

    let skills_json = req
        .skills
        .map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null));

    let patch = TaskTemplatePatch {
        name: req.name,
        description: req.description.map(Some),
        icon: req.icon.map(Some),
        color: req.color.map(Some),
        repository_id: req.repository_id,
        agent_type: req.agent_type,
        agent_mode: req.agent_mode.map(Some),
        default_model: req.default_model.map(Some),
        initial_prompt: req.initial_prompt.map(Some),
        skills: skills_json.map(Some),
        scope: resolved_scope.as_ref().map(|scope| scope.scope),
        team_id: resolved_scope
            .as_ref()
            .map(|scope| Some(scope.team_id.clone())),
        owner_user_id: resolved_scope
            .as_ref()
            .map(|scope| scope.owner_user_id.clone()),
        featured: req.featured,
        display_order: req.display_order,
    };

    state.task_templates.update(&template_id, &patch).await?;

    let updated = state
        .task_templates
        .find_by_id(&template_id, &auth.scope_context())
        .await?
        .ok_or_else(|| TaskTemplateError::NotFound(template_id))?;

    Ok(Json(updated.into()))
}

/// Delete a task template by ID.
#[utoipa::path(
    delete,
    path = "/api/v1/task-templates/{template_id}",
    tag = "task-templates",
    params(
        ("template_id" = String, Path, description = "Template identifier")
    ),
    responses(
        (status = 204, description = "Task template deleted"),
        (status = 403, description = "Access denied", body = crate::error::ErrorBody),
        (status = 404, description = "Template not found", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_task_template(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(template_id): Path<String>,
) -> Result<StatusCode, TaskTemplateError> {
    let existing = state
        .task_templates
        .find_by_id_internal(&template_id)
        .await?
        .ok_or_else(|| TaskTemplateError::NotFound(template_id.clone()))?;

    ensure_can_write_resource(
        &state,
        &auth,
        &template_resource(&existing),
        "task template",
    )
    .await?;

    state.task_templates.delete(&template_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

fn template_resource(template: &TaskTemplate) -> ScopedResource {
    ScopedResource::borrowed(
        template.scope,
        template.team_id.as_deref(),
        template
            .owner_user_id
            .as_deref()
            .or(template.created_by.as_deref()),
    )
}
