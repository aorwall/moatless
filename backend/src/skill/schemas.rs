//! Skill API DTOs — re-exported from the shared contract crate.

pub use moatless_api_schemas::skill::{ListSkillsResponse, SkillResponse, SkillSource};

use super::models::Skill;

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
