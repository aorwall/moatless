//! Task template request/response types.
//!
//! Re-exported from the `moatless-api-schemas` crate.

pub use moatless_api_schemas::task_template::*;

use super::models::TaskTemplate;

impl From<TaskTemplate> for TaskTemplateResponse {
    fn from(t: TaskTemplate) -> Self {
        Self {
            id: t.id,
            name: t.name,
            description: t.description,
            icon: t.icon,
            color: t.color,
            repository_id: t.repository_id,
            agent_type: t.agent_type,
            agent_mode: t.agent_mode,
            default_model: t.default_model,
            initial_prompt: t.initial_prompt,
            skills: t
                .skills
                .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok()),
            scope: t.scope,
            team_id: t.team_id,
            featured: t.featured,
            display_order: t.display_order,
            created_by: t.created_by,
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
        }
    }
}
