//! Plugin response types.
//!
//! Re-exported from the `moatless-workspace-types::schemas` module.

pub use moatless_workspace_types::schemas::plugin::*;

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
