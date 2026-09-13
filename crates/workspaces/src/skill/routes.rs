//! Skill API routes with OpenAPI documentation.

use std::sync::Arc;

use axum::extract::FromRef;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use super::schemas::{ListSkillsResponse, SkillResponse, SkillSource};
use super::service::SkillService;
use moatless_core::http_auth::UserTokenVerifier;

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
///
/// Generic over `S` so this module names no root type; handlers reach
/// `Arc<SkillService>` and the `UserTokenVerifier` behind `AuthUser` through
/// `FromRef<S>`.
pub fn skill_router<S>(state: S) -> (axum::Router, utoipa::openapi::OpenApi)
where
    S: Clone + Send + Sync + 'static,
    Arc<SkillService>: FromRef<S>,
    Arc<dyn UserTokenVerifier>: FromRef<S>,
{
    let (router, api) = OpenApiRouter::with_openapi(SkillApiDoc::openapi())
        .routes(routes!(handlers::list_repository_skills))
        .with_state(state)
        .split_for_parts();

    (router, api)
}
