//! Plugin response types.
//!
//! Re-exported from the `moatless-api-schemas` crate.

pub use moatless_api_schemas::plugin::*;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationSubjectParams {
    #[serde(default)]
    pub user_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearActivationParams {
    pub reach: ActivationReach,
    #[serde(default)]
    pub skill_name: Option<String>,
    #[serde(default)]
    pub user_id: Option<uuid::Uuid>,
    #[serde(default)]
    pub workspace_id: Option<String>,
}
