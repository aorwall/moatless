//! Skill request/response types.

use serde::{Deserialize, Serialize};

/// Origin of a discovered skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    /// Skill lives in the workspace repository's own `.claude/skills/` directory.
    Workspace,
    /// Skill lives inside a linked plugin repository.
    Plugin,
}

/// Single skill discovered from a `.claude/skills/{name}/SKILL.md` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SkillResponse {
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_repo_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_repo_name: Option<String>,
}

/// Response body for `GET /api/v1/repositories/{repository_id}/skills`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ListSkillsResponse {
    pub skills: Vec<SkillResponse>,
}
