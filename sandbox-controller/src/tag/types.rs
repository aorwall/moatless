//! Tag response types.

use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagResponse {
    pub tag_id: String,
    pub name: String,
    pub description: Option<String>,
}
