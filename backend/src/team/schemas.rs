//! Team request/response types.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::models::{Team, TeamMember};

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTeamRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddTeamMemberRequest {
    pub user_id: String,
    pub role: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TeamResponse {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TeamMemberResponse {
    pub team_id: String,
    pub user_id: String,
    pub role: String,
    pub created_at: String,
}

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
            user_id: m.user_id,
            role: m.role,
            created_at: m.created_at.to_rfc3339(),
        }
    }
}

// ---- Team credential schemas ----

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpsertTeamCredentialRequest {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TeamCredentialResponse {
    pub provider: String,
    pub token_type: Option<String>,
    pub has_refresh_token: bool,
}
