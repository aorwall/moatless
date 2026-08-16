//! Skill API DTOs.

use serde::{Deserialize, Serialize};

use super::models::Skill;

/// Origin of a discovered skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    /// Skill lives in the workspace repository's own `.claude/skills/` directory.
    Workspace,
    /// Skill lives inside a linked plugin.
    Plugin,
}

/// Single skill discovered from a `.claude/skills/{name}/SKILL.md` file.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillResponse {
    pub name: String,
    pub description: String,
    pub source: SkillSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_name: Option<String>,
}

/// Response body for `GET /api/v1/repositories/{repository_id}/skills`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListSkillsResponse {
    pub skills: Vec<SkillResponse>,
}

impl From<Skill> for SkillResponse {
    fn from(skill: Skill) -> Self {
        Self {
            name: skill.name,
            description: skill.description,
            source: skill.source,
            plugin_id: skill.plugin_id,
            plugin_name: skill.plugin_name,
        }
    }
}
