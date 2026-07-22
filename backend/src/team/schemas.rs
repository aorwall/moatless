//! Team request/response types.
//!
//! Re-exported from the `moatless-api-schemas` crate.

pub use moatless_api_schemas::team::*;

use super::models::{Team, TeamMember};

impl From<Team> for TeamResponse {
    fn from(t: Team) -> Self {
        Self {
            id: t.id,
            name: t.name,
            slug: t.slug,
            description: t.description,
            is_default: t.is_default,
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
        }
    }
}

impl From<TeamMember> for TeamMemberResponse {
    fn from(m: TeamMember) -> Self {
        Self {
            team_id: m.team_id,
            user_id: m.user_id.to_string(),
            role: m.role,
            created_at: m.created_at.to_rfc3339(),
        }
    }
}
