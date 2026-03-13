//! Command request/response types.
//!
//! Re-exported from the `moatless-api-schemas` crate.

pub use moatless_api_schemas::command::*;

impl From<super::models::Command> for CommandResponse {
    fn from(row: super::models::Command) -> Self {
        Self {
            name: row.name,
            description: row.description,
            prompt: row.prompt,
            conditions: row.conditions,
            icon: row.icon,
            variant: row.variant,
            active: row.active,
            will_not_resume: row.will_not_resume,
            agent_mode: row.agent_mode,
        }
    }
}
