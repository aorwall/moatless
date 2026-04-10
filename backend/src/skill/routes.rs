//! Skill API routes with OpenAPI documentation.

use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::schemas::{ListSkillsResponse, SkillResponse, SkillSource};
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = "skills", description = "Repository skill discovery"),
    ),
    paths(
        handlers::list_repository_skills,
    ),
    components(
        schemas(SkillResponse, ListSkillsResponse, SkillSource)
    )
)]
pub struct SkillApiDoc;

/// Build the skill router with OpenAPI metadata.
pub fn skill_router(state: AppState) -> (axum::Router, utoipa::openapi::OpenApi) {
    let (router, api) = OpenApiRouter::with_openapi(SkillApiDoc::openapi())
        .routes(routes!(handlers::list_repository_skills))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
